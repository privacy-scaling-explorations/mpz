//! Stepwise execution of a single call into a module.
//!
//! A [`Thread`] interprets a local function one operation at a time against a
//! shared [`Global`] state, tracking which values are concrete and which are
//! symbolic (held by a remote party). Execution proceeds by repeated calls to
//! [`Thread::step`], each of which returns a [`StepResult`] describing what
//! happened.
//!
//! When the interpreter reaches an operation whose outcome depends on data this
//! party does not hold — a symbolic branch condition or a host/import call —
//! the thread blocks and reports a [`Pending`] condition. The embedder supplies
//! the missing decision through the matching `resolve_*` method
//! ([`Thread::resolve_branch`] or [`Thread::resolve_host_call`]) before
//! stepping again.
//!
//! An operation that may trap on operands this party cannot inspect (an integer
//! divide/remainder with an unheld operand) does not block: it is emitted as an
//! ordinary [`Directive`] and execution continues as if it did not trap.
//! Whether it actually trapped is decided externally by correlating its
//! operation index (see [`Thread::op_counter`]) against the trap a peer
//! reports.
//!
//! All execution runs against a [`Context`], which pairs the [`Module`] being
//! interpreted with the mutable [`Global`] state it operates on.

use std::{collections::HashMap, sync::Arc};

use crate::taint::{Class, Taints};
use mpz_vm_ir::{
    BinaryOp, BlockId, Function, FunctionBody, Instruction, InstructionArith, LoadKind,
    LocalFunction, Reg, Terminator,
};

use mpz_vm_ir::Module;

use crate::{
    Directive, Global, MaybeTrap, Op, Operand, Trap, Visibility,
    access_log::{Access, AccessAddr, AccessKind},
    analysis::{BranchRegion, FunctionAnalysis},
    arithmetic,
    call::{Call, Param},
    memory::Memory,
    value::Value,
};

use crate::Error;

/// The environment a [`Thread`] executes against: the [`Module`] being
/// interpreted paired with the mutable [`Global`] state it reads and writes.
///
/// This is internal plumbing that bundles the two references threaded through
/// the interpreter; embedders pass `module` and `global` to [`Thread::call`]
/// and [`Thread::step`] directly.
struct Context<'a> {
    module: &'a Module,
    global: &'a mut Global,
}

/// A condition that a [`Thread`] is blocked on, awaiting resolution.
///
/// Each variant corresponds to a decision the thread cannot make on its own
/// because it depends on data held by a remote party or supplied by the
/// embedder.
#[derive(Debug, Clone)]
pub enum Pending {
    /// A branch whose condition is symbolic and not held by this party, so the
    /// taken path cannot be decided locally.
    Branch,
    /// A call to a host or imported function, whose return value must be
    /// supplied by the embedder.
    HostCall {
        /// The index of the called function.
        func_idx: u32,
        /// The register that receives the return value, if the call returns
        /// one.
        dst: Option<Reg>,
        /// The call arguments, so the embedder can service the host call.
        args: Vec<Operand>,
    },
    /// An indirect call whose table index is symbolic and not held by this
    /// party, so the dispatch target cannot be chosen locally. The embedder
    /// resolves it with [`Thread::resolve_call_indirect`] supplying the
    /// concrete index.
    CallIndirect {
        /// The absolute register holding the symbolic table index.
        table_idx: Reg,
    },
    /// A `memory.grow` whose page count is symbolic and not held by this party.
    /// Growing shared memory by a private amount would diverge between parties,
    /// so the count must be supplied via [`Thread::resolve_memory_grow`].
    MemoryGrow {
        /// The absolute register holding the symbolic page count.
        pages: Reg,
    },
}

/// The outcome of a single [`Thread::step`].
#[derive(Debug)]
pub enum StepResult {
    /// The operation completed with no externally observable effect; step again
    /// to continue.
    Continue,
    /// The operation produced a [`Directive`] for the embedder to observe.
    Directive(Directive),
    /// The thread is blocked on a [`Pending`] condition that must be resolved
    /// with [`Thread::resolve`] before stepping again.
    Blocked(Pending),
    /// The operation trapped, terminating the call.
    Trapped {
        /// The operation index at which the trap occurred.
        index: u64,
        /// The directive describing the trapping operation, if it was symbolic.
        directive: Option<Directive>,
        /// The trap that terminated the call.
        trap: Trap,
    },
    /// The call returned; no further stepping is possible.
    Done {
        /// The return value, if the function produces one.
        result: Option<Value>,
        /// Whether the result is symbolic rather than a concrete value.
        symbolic: bool,
    },
}

#[derive(Debug)]
enum FrameStepResult {
    Continue,
    Directive(Directive),
    Trapped {
        directive: Option<Directive>,
        trap: Trap,
    },
    Call {
        func_idx: u32,
        args: Vec<Operand>,
        dst: Option<Reg>,
    },
    Return {
        return_reg: Option<Reg>,
    },
    /// The frame cannot proceed until the embedder resolves `pending`. The
    /// instruction pointer is left unadvanced so the instruction re-runs once a
    /// resolution is supplied.
    Blocked(Pending),
}

fn binary_trap(op: BinaryOp, lhs: Value, rhs: Value) -> Option<Trap> {
    use BinaryOp::*;
    match op {
        I32DivU | I32RemU | I32DivS | I32RemS if matches!(rhs, Value::I32(0)) => {
            Some(Trap::DivideByZero)
        }
        I64DivU | I64RemU | I64DivS | I64RemS if matches!(rhs, Value::I64(0)) => {
            Some(Trap::DivideByZero)
        }
        I32DivS if matches!((lhs, rhs), (Value::I32(i32::MIN), Value::I32(-1))) => {
            Some(Trap::IntegerOverflow)
        }
        I64DivS if matches!((lhs, rhs), (Value::I64(i64::MIN), Value::I64(-1))) => {
            Some(Trap::IntegerOverflow)
        }
        _ => None,
    }
}

fn binary_could_trap(op: BinaryOp) -> bool {
    use BinaryOp::*;
    matches!(
        op,
        I32DivU | I32RemU | I32DivS | I32RemS | I64DivU | I64RemU | I64DivS | I64RemS
    )
}

fn binary_trap_needs_lhs(op: BinaryOp) -> bool {
    matches!(op, BinaryOp::I32DivS | BinaryOp::I64DivS)
}

macro_rules! mem_try {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(trap) => {
                return Ok(FrameStepResult::Trapped {
                    directive: None,
                    trap,
                });
            }
        }
    };
}

#[derive(Debug, Clone, Copy)]
enum PrivateCfExit {
    Join { block: BlockId, depth: usize },
    Return { depth: usize },
}

/// A stepwise interpreter for a single call into a module.
///
/// A thread maintains its own call stack and register file and is driven by
/// repeated calls to [`step`](Self::step). Construct one with
/// [`new`](Self::new), begin a call with [`call`](Self::call), then step until
/// a [`StepResult::Done`] or [`StepResult::Trapped`] is observed, resolving any
/// [`StepResult::Blocked`] conditions with [`resolve`](Self::resolve) along the
/// way.
#[derive(Debug)]
pub struct Thread {
    call_stack: Vec<Frame>,
    registers: Vec<Value>,
    reg_taints: Taints,
    done: bool,
    has_result: bool,
    pending: Option<Pending>,
    op_counter: u64,
    private_cf: Option<PrivateCfExit>,
    deferred_complete: bool,
    deferred_trap: Option<Trap>,
    /// A concrete value supplied by
    /// `resolve_call_indirect`/`resolve_memory_grow` to re-run the blocked
    /// instruction with.
    resolved: Option<i32>,
    /// Per-function branch side-effect analysis, computed lazily on first entry
    /// to a function and cached by function index.
    analysis: HashMap<u32, Arc<FunctionAnalysis>>,
}

impl Default for Thread {
    fn default() -> Self {
        Self::new()
    }
}

impl Thread {
    /// Creates a new thread with an empty call stack and register file.
    pub fn new() -> Self {
        Self {
            call_stack: Vec::new(),
            registers: Vec::new(),
            reg_taints: Taints::new(),
            done: false,
            has_result: false,
            pending: None,
            op_counter: 0,
            private_cf: None,
            deferred_complete: false,
            deferred_trap: None,
            resolved: None,
            analysis: HashMap::new(),
        }
    }

    /// Returns the global operation counter: the index of the next operation to
    /// be emitted.
    ///
    /// The counter advances once per emitted operation and is identical across
    /// parties for every operation up to and including a trap, so embedders can
    /// use it to correlate directives and trap points.
    pub fn op_counter(&self) -> u64 {
        self.op_counter
    }

    /// Marks the register at absolute index `abs_reg` as held by the local
    /// party.
    ///
    /// A held register's value is available locally and may be read back
    /// through [`registers`](Self::registers_mut).
    pub fn set_register_available(&mut self, abs_reg: u32) {
        self.reg_taints.mark_held(abs_reg);
    }

    /// Returns `true` if the thread is currently executing inside a region of
    /// private control flow.
    pub fn in_private_cf(&self) -> bool {
        self.private_cf.is_some()
    }

    /// Returns a read-only view of the thread's register file, indexed by
    /// absolute register number.
    pub fn registers(&self) -> &[Value] {
        &self.registers
    }

    /// Returns whether the register at absolute index `abs_reg` currently
    /// holds symbolic (committed) data.
    ///
    /// Symbolic-ness is identical across parties: a register is symbolic for
    /// both or neither, only *heldness* differs.
    pub fn is_register_symbolic(&self, abs_reg: u32) -> bool {
        self.reg_taints.is_symbolic(abs_reg)
    }

    /// Returns a mutable view of the thread's register file, indexed by
    /// absolute register number.
    pub fn registers_mut(&mut self) -> &mut [Value] {
        &mut self.registers
    }

    /// Resolves a blocked [`Pending::Branch`].
    ///
    /// `unreachable` reports whether the path chosen externally is unreachable,
    /// which raises [`Trap::Unreachable`] on the next [`step`](Self::step).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotBlocked`] if the thread is not blocked and
    /// [`Error::UnexpectedResolution`] if it is blocked on a different
    /// kind of condition.
    pub fn resolve_branch(&mut self, unreachable: bool) -> Result<(), Error> {
        match &self.pending {
            Some(Pending::Branch) => {
                self.pending = None;
                if unreachable {
                    self.deferred_trap = Some(Trap::Unreachable);
                } else {
                    self.op_counter += 1;
                }
                Ok(())
            }
            Some(_) => Err(Error::UnexpectedResolution),
            None => Err(Error::NotBlocked),
        }
    }

    /// Resolves a blocked [`Pending::HostCall`].
    ///
    /// `value` supplies the call's return value; it is required when the call
    /// has a destination register and ignored otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotBlocked`] if the thread is not blocked,
    /// [`Error::UnexpectedResolution`] if it is blocked on a different
    /// kind of condition, and [`Error::MissingHostCallValue`] if the call
    /// requires a return value that was not supplied.
    pub fn resolve_host_call(
        &mut self,
        value: Option<Value>,
        visibility: Visibility,
    ) -> Result<(), Error> {
        let dst = match &self.pending {
            Some(Pending::HostCall { dst, .. }) => *dst,
            Some(_) => return Err(Error::UnexpectedResolution),
            None => return Err(Error::NotBlocked),
        };
        if let Some(reg) = dst {
            match value {
                Some(v) => {
                    self.registers[reg.index()] = v;
                    self.reg_taints.set(reg.as_u32(), Class::from(visibility));
                }
                None => return Err(Error::MissingHostCallValue),
            }
        }
        self.pending = None;
        Ok(())
    }

    /// Resolves a blocked [`Pending::CallIndirect`] with the concrete table
    /// `index`, letting the indirect call's dispatch proceed on the next
    /// [`step`](Self::step).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotBlocked`] if the thread is not blocked and
    /// [`Error::UnexpectedResolution`] if it is blocked on a different
    /// kind of condition.
    pub fn resolve_call_indirect(&mut self, index: i32) -> Result<(), Error> {
        match &self.pending {
            Some(Pending::CallIndirect { .. }) => {
                self.pending = None;
                self.resolved = Some(index);
                Ok(())
            }
            Some(_) => Err(Error::UnexpectedResolution),
            None => Err(Error::NotBlocked),
        }
    }

    /// Resolves a blocked [`Pending::MemoryGrow`] with the concrete page count
    /// `pages`, letting the growth proceed on the next [`step`](Self::step).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotBlocked`] if the thread is not blocked and
    /// [`Error::UnexpectedResolution`] if it is blocked on a different
    /// kind of condition.
    pub fn resolve_memory_grow(&mut self, pages: i32) -> Result<(), Error> {
        match &self.pending {
            Some(Pending::MemoryGrow { .. }) => {
                self.pending = None;
                self.resolved = Some(pages);
                Ok(())
            }
            Some(_) => Err(Error::UnexpectedResolution),
            None => Err(Error::NotBlocked),
        }
    }

    /// Returns the thread's call stack, innermost [`Frame`] last.
    pub fn call_stack(&self) -> &[Frame] {
        &self.call_stack
    }

    #[cfg(test)]
    fn is_done(&self) -> bool {
        self.done
    }

    /// Begins a call into a local function of `module`.
    ///
    /// Sets up the initial call frame from `call`, binding its parameters to
    /// registers and recording each one's visibility class. After this returns,
    /// drive execution with [`step`](Self::step), passing the same `module` and
    /// `global`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyRunning`] if a call is already in progress,
    /// [`Error::UndefinedFunction`] if the function index is not present,
    /// [`Error::InvalidFunction`] if it does not refer to a local
    /// function, and [`Error::Unimplemented`] if the function returns more
    /// than one value.
    pub fn call(&mut self, module: &Module, global: &mut Global, call: Call) -> Result<(), Error> {
        let cx = &mut Context { module, global };
        let Call { func_idx, params } = call;

        if !self.call_stack.is_empty() {
            return Err(Error::AlreadyRunning);
        }

        let Function::Local(func) = cx
            .module
            .function(func_idx)
            .ok_or(Error::UndefinedFunction(func_idx))?
        else {
            return Err(Error::InvalidFunction(func_idx));
        };

        let num_results = func.func_type().results.len();
        if num_results > 1 {
            return Err(Error::Unimplemented("multi-value return"));
        }

        self.has_result = num_results > 0;
        let reg_base = num_results;
        let num_args = params.len();

        self.registers.resize(num_results + num_args, Value::I32(0));

        for (i, param) in params.into_iter().enumerate() {
            let abs_reg = (reg_base + i) as u32;
            let (value, class) = match param {
                Param::Private(value) => (value, Class::Private),
                Param::Public(value) => (value, Class::Public),
                Param::Blind(ty) => (Value::zero(ty), Class::Blind),
            };
            self.registers[reg_base + i] = value;
            self.reg_taints.set(abs_reg, class);
        }

        let reg_result = if self.has_result { Some(Reg(0)) } else { None };
        self.enter_frame(
            cx,
            func_idx,
            func,
            (reg_base as u32..(reg_base + num_args) as u32)
                .map(Reg)
                .collect(),
            reg_result,
        )?;

        Ok(())
    }

    /// Executes a single operation and reports the outcome.
    ///
    /// Advances the interpreter by one step, returning a [`StepResult`] that
    /// describes whether execution continued, emitted a [`Directive`], blocked
    /// on a [`Pending`] condition, trapped, or completed. Trapping and
    /// completion are reported as ordinary [`StepResult`] values, not errors.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Blocked`] if the thread is blocked on an
    /// unresolved [`Pending`] condition, [`Error::NotStarted`] if no call
    /// has been started, [`Error::Completed`] if the call has already
    /// completed, and [`Error`] variants such as
    /// [`Error::InvalidFunction`], [`Error::UndefinedFunction`], or
    /// [`Error::Unimplemented`] when the operation being executed cannot be
    /// interpreted.
    pub fn step(&mut self, module: &Module, global: &mut Global) -> Result<StepResult, Error> {
        if let Some(pending @ Pending::HostCall { .. }) = &self.pending {
            return Ok(StepResult::Blocked(pending.clone()));
        }
        if self.pending.is_some() {
            return Err(Error::Blocked);
        }
        if self.deferred_complete {
            self.deferred_complete = false;
            return self.complete();
        }
        if let Some(trap) = self.deferred_trap.take() {
            self.done = true;
            return Ok(StepResult::Trapped {
                index: self.op_counter,
                directive: None,
                trap,
            });
        }
        let cx = &mut Context { module, global };
        self.check_private_cf_exit();
        let in_private_cf = self.in_private_cf();

        let func_idx = match self.call_stack.last() {
            Some(frame) => frame.func_idx,
            None if self.done => return Err(Error::Completed),
            None => return Err(Error::NotStarted),
        };
        let func = match cx.module.function(func_idx) {
            Some(Function::Local(f)) => f,
            _ => return Err(Error::InvalidFunction(func_idx)),
        };

        let analysis = match self.analysis.get(&func_idx) {
            Some(analysis) => Arc::clone(analysis),
            None => {
                let analysis = Arc::new(FunctionAnalysis::compute(func.body()));
                self.analysis.insert(func_idx, Arc::clone(&analysis));
                analysis
            }
        };

        let resolved = self.resolved.take();
        let frame = self
            .call_stack
            .last_mut()
            .expect("call stack should be non-empty");
        let frame_result = frame.step(
            cx,
            &mut self.registers,
            &mut self.reg_taints,
            func.body(),
            &analysis,
            in_private_cf,
            resolved,
        );
        let frame_result = frame_result?;
        match frame_result {
            FrameStepResult::Continue => {
                self.check_private_cf_exit();
                Ok(StepResult::Continue)
            }
            FrameStepResult::Directive(event) => {
                match &event {
                    Directive::Branch {
                        cond:
                            Some(Operand::Symbol {
                                value: cond_value, ..
                            }),
                        exit,
                        bail_out,
                        ..
                    } => {
                        if !bail_out && self.private_cf.is_none() {
                            self.private_cf = Some(match exit {
                                Some(block) => PrivateCfExit::Join {
                                    block: *block,
                                    depth: self.call_stack.len(),
                                },
                                None => PrivateCfExit::Return {
                                    depth: self.call_stack.len(),
                                },
                            });
                        }
                        if cond_value.is_none() {
                            if exit.is_none() {
                                self.exit_frame()?;
                            }
                            self.pending = Some(Pending::Branch);
                            self.check_private_cf_exit();
                            return Ok(StepResult::Blocked(Pending::Branch));
                        }
                    }
                    Directive::Call {
                        dst,
                        func_idx,
                        args,
                        ..
                    } => {
                        self.pending = Some(Pending::HostCall {
                            func_idx: *func_idx,
                            dst: *dst,
                            args: args.clone(),
                        });
                    }
                    _ => {}
                }
                self.check_private_cf_exit();
                self.op_counter += 1;
                Ok(StepResult::Directive(event))
            }
            FrameStepResult::Trapped { directive, trap } => {
                let index = self.op_counter;
                self.done = true;
                Ok(StepResult::Trapped {
                    index,
                    directive,
                    trap,
                })
            }
            FrameStepResult::Call {
                func_idx,
                args,
                dst,
            } => {
                let func = match cx.module.function(func_idx) {
                    Some(Function::Local(f)) => f,
                    _ => return Err(Error::UndefinedFunction(func_idx)),
                };
                let reg_args: Vec<Reg> = args
                    .iter()
                    .map(|a| match a {
                        Operand::Symbol { reg, .. } => *reg,
                        _ => unreachable!("local call args should be taints"),
                    })
                    .collect();
                let param_base = Reg(self.registers.len() as u32);
                self.enter_frame(cx, func_idx, func, reg_args, dst)?;
                self.op_counter += 1;
                Ok(StepResult::Directive(Directive::Call {
                    dst,
                    func_idx,
                    args,
                    param_base,
                }))
            }
            FrameStepResult::Return { return_reg } => {
                let popped = self.exit_frame()?;
                self.check_private_cf_exit();
                let src = return_reg.map(|r| popped.reg_base + r);
                let (dst, reclaim) = if self.call_stack.is_empty() {
                    self.deferred_complete = true;
                    (None, None)
                } else {
                    (popped.reg_result, Some((popped.reg_base, popped.num_regs)))
                };
                self.op_counter += 1;
                Ok(StepResult::Directive(Directive::Return {
                    dst,
                    src,
                    reclaim,
                }))
            }
            FrameStepResult::Blocked(pending) => {
                self.pending = Some(pending.clone());
                Ok(StepResult::Blocked(pending))
            }
        }
    }

    fn check_private_cf_exit(&mut self) {
        let exited = match self.private_cf {
            Some(PrivateCfExit::Join { block, depth }) => {
                self.call_stack.len() < depth
                    || (self.call_stack.len() == depth
                        && self
                            .call_stack
                            .last()
                            .map(|f| f.current_block == block && f.ip == 0)
                            .unwrap_or(false))
            }
            Some(PrivateCfExit::Return { depth }) => self.call_stack.len() < depth,
            None => false,
        };
        if exited {
            self.private_cf = None;
        }
    }

    fn enter_frame(
        &mut self,
        _cx: &mut Context<'_>,
        func_idx: u32,
        func: &LocalFunction,
        args: Vec<Reg>,
        reg_result: Option<Reg>,
    ) -> Result<(), Error> {
        if func.func_type().results.len() > 1 {
            return Err(Error::Unimplemented("multi-value return"));
        }

        let func_type = func.func_type();
        let num_regs = func.register_count() as usize;
        let reg_base = self.registers.len();

        let target_len = reg_base + num_regs;
        if self.registers.len() < target_len {
            self.registers.resize(target_len, Value::I32(0));
        }

        for (i, arg) in args.iter().enumerate() {
            let abs_dst = (reg_base + i) as u32;
            self.registers[reg_base + i] = self.registers[arg.index()];
            self.reg_taints.copy(arg.as_u32(), abs_dst, 1);
        }

        let num_params = func_type.params.len();
        let mut local_idx = num_params;
        for local in func.locals() {
            let zero = Value::zero(local.ty);
            for _ in 0..local.count {
                let abs = (reg_base + local_idx) as u32;
                self.registers[reg_base + local_idx] = zero;
                self.reg_taints.set(abs, Class::Public);
                local_idx += 1;
            }
        }

        self.call_stack.push(Frame {
            func_idx,
            reg_base: Reg(reg_base as u32),
            num_regs: num_regs as u32,
            current_block: func.body().entry,
            ip: 0,
            reg_result,
        });

        Ok(())
    }

    fn exit_frame(&mut self) -> Result<Frame, Error> {
        let frame = self
            .call_stack
            .pop()
            .ok_or_else(|| Error::Internal("no frame to exit".into()))?;
        self.reg_taints.set_range(
            frame.reg_base.as_u32(),
            frame.num_regs as usize,
            Class::Public,
        );
        self.registers.truncate(frame.reg_base.index());
        Ok(frame)
    }

    fn complete(&mut self) -> Result<StepResult, Error> {
        self.done = true;
        let (result, symbolic) = if self.has_result {
            (Some(self.registers[0]), self.reg_taints.is_symbolic(0))
        } else {
            (None, false)
        };
        Ok(StepResult::Done { result, symbolic })
    }
}

/// A single activation record on a [`Thread`]'s call stack.
#[derive(Debug)]
pub struct Frame {
    func_idx: u32,
    reg_base: Reg,
    num_regs: u32,
    reg_result: Option<Reg>,
    current_block: BlockId,
    ip: usize,
}

impl Frame {
    /// Returns the index of the function this frame is executing.
    pub fn func_idx(&self) -> u32 {
        self.func_idx
    }

    /// Returns the absolute register index at which this frame's registers
    /// begin.
    pub fn reg_base(&self) -> Reg {
        self.reg_base
    }

    /// Returns the block currently executing in this frame.
    pub fn current_block(&self) -> BlockId {
        self.current_block
    }

    /// Returns the instruction pointer into the current block.
    pub fn ip(&self) -> usize {
        self.ip
    }

    fn abs(&self, reg: Reg) -> Reg {
        self.reg_base + reg
    }

    fn get(&self, registers: &[Value], reg: Reg) -> Value {
        registers[self.reg_base.index() + reg.index()]
    }

    fn set(&self, registers: &mut [Value], reg: Reg, value: Value) {
        registers[self.reg_base.index() + reg.index()] = value;
    }

    fn is_symbolic(&self, taints: &Taints, reg: Reg) -> bool {
        taints.is_symbolic(self.abs(reg).as_u32())
    }

    fn is_available(&self, taints: &Taints, reg: Reg) -> bool {
        taints.is_held(self.abs(reg).as_u32())
    }

    /// Returns `true` if `reg` holds an address this party cannot locate: it is
    /// symbolic and its bits are not available here.
    fn addr_unlocatable(&self, taints: &Taints, reg: Reg) -> bool {
        self.is_symbolic(taints, reg) && !self.is_available(taints, reg)
    }

    /// Conservatively marks the whole linear memory as blind (symbolic and
    /// unheld), used when an op writes at an address this party cannot locate.
    fn mark_all_memory_blind(&self, cx: &mut Context<'_>) {
        if let Some(memory) = cx.global.memory() {
            let len = memory.len();
            cx.global
                .memory_taints_mut()
                .set_range(0, len, Class::Blind);
        }
    }

    fn operand(&self, registers: &[Value], taints: &Taints, reg: Reg) -> Operand {
        if self.is_symbolic(taints, reg) {
            let value = if self.is_available(taints, reg) {
                Some(self.get(registers, reg))
            } else {
                None
            };
            Operand::Symbol {
                reg: self.abs(reg),
                value,
            }
        } else {
            Operand::Concrete(self.get(registers, reg))
        }
    }

    fn set_clear(&self, registers: &mut [Value], taints: &mut Taints, reg: Reg, value: Value) {
        self.set(registers, reg, value);
        taints.set(self.abs(reg).as_u32(), Class::Public);
    }

    fn set_symbolic(&self, taints: &mut Taints, reg: Reg, available: bool) {
        taints.set(self.abs(reg).as_u32(), Class::symbolic(available));
    }

    fn advance_ip(&mut self) {
        self.ip += 1;
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        body: &FunctionBody,
        analysis: &FunctionAnalysis,
        in_private_cf: bool,
        resolved: Option<i32>,
    ) -> Result<FrameStepResult, Error> {
        let block = &body.blocks[self.current_block.index()];

        if self.ip >= block.body.len() {
            return self.execute_terminator(
                cx,
                registers,
                taints,
                &block.terminator,
                analysis,
                in_private_cf,
            );
        }

        let instr = block.body[self.ip].clone();
        match &instr {
            Instruction::Nop => self.advance_ip(),

            Instruction::I32Const { dst, val } => {
                self.set_clear(registers, taints, *dst, Value::from(*val));
                self.advance_ip();
            }

            Instruction::I64Const { dst, val } => {
                self.set_clear(registers, taints, *dst, Value::from(*val));
                self.advance_ip();
            }

            Instruction::F32Const { dst, val } => {
                self.set_clear(registers, taints, *dst, Value::from(f32::from_bits(*val)));
                self.advance_ip();
            }

            Instruction::F64Const { dst, val } => {
                self.set_clear(registers, taints, *dst, Value::from(f64::from_bits(*val)));
                self.advance_ip();
            }

            Instruction::Copy { dst, src } => {
                let value = self.get(registers, *src);
                if self.is_symbolic(taints, *src) {
                    let d = Directive::Op(Op::Copy {
                        dst: self.abs(*dst),
                        src: self.abs(*src),
                    });
                    let avail = self.is_available(taints, *src);
                    self.set(registers, *dst, value);
                    self.set_symbolic(taints, *dst, avail);
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                self.set_clear(registers, taints, *dst, value);
                self.advance_ip();
            }

            Instruction::GlobalGet { dst, global_idx } => {
                let value = cx
                    .global
                    .globals()
                    .get(*global_idx as usize)
                    .cloned()
                    .ok_or(Error::UndefinedGlobal(*global_idx))?;
                if cx.global.global_taints().is_symbolic(*global_idx) {
                    let d = Directive::Op(Op::GlobalGet {
                        dst: self.abs(*dst),
                        global_idx: *global_idx,
                    });
                    let avail = cx.global.global_taints().is_held(*global_idx);
                    self.set(registers, *dst, value);
                    self.set_symbolic(taints, *dst, avail);
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                self.set_clear(registers, taints, *dst, value);
                self.advance_ip();
            }

            Instruction::GlobalSet { global_idx, src } => {
                let value = self.get(registers, *src);
                let op_src = self.operand(registers, taints, *src);
                let idx = *global_idx as usize;
                if self.is_symbolic(taints, *src) {
                    let src_avail = self.is_available(taints, *src);
                    let d = Directive::Op(Op::GlobalSet {
                        global_idx: *global_idx,
                        src: op_src,
                    });
                    let globals = cx.global.globals_mut();
                    if idx >= globals.len() {
                        return Err(Error::UndefinedGlobal(*global_idx));
                    }
                    globals[idx] = value;
                    cx.global
                        .global_taints_mut()
                        .set(*global_idx, Class::symbolic(src_avail));
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                let globals = cx.global.globals_mut();
                if idx >= globals.len() {
                    return Err(Error::UndefinedGlobal(*global_idx));
                }
                globals[idx] = value;
                cx.global
                    .global_taints_mut()
                    .set(*global_idx, Class::Public);
                self.advance_ip();
            }

            Instruction::Select {
                dst,
                cond,
                if_true,
                if_false,
            } => {
                let cond_sym = self.is_symbolic(taints, *cond);
                let true_sym = self.is_symbolic(taints, *if_true);
                let false_sym = self.is_symbolic(taints, *if_false);

                if !cond_sym {
                    let condition = self.get(registers, *cond).as_i32()?;
                    let (result, sel_reg, sel_sym) = if condition != 0 {
                        (self.get(registers, *if_true), *if_true, true_sym)
                    } else {
                        (self.get(registers, *if_false), *if_false, false_sym)
                    };
                    if sel_sym {
                        let d = Directive::Op(Op::Select {
                            dst: self.abs(*dst),
                            cond: self.operand(registers, taints, *cond),
                            if_true: self.operand(registers, taints, *if_true),
                            if_false: self.operand(registers, taints, *if_false),
                        });
                        let avail = self.is_available(taints, sel_reg);
                        self.set(registers, *dst, result);
                        self.set_symbolic(taints, *dst, avail);
                        self.advance_ip();
                        return Ok(FrameStepResult::Directive(d));
                    }
                    self.set_clear(registers, taints, *dst, result);
                    self.advance_ip();
                } else {
                    let d = Directive::Op(Op::Select {
                        dst: self.abs(*dst),
                        cond: self.operand(registers, taints, *cond),
                        if_true: self.operand(registers, taints, *if_true),
                        if_false: self.operand(registers, taints, *if_false),
                    });
                    if self.is_available(taints, *cond) {
                        let condition = self.get(registers, *cond).as_i32()?;
                        let (result, sel_reg) = if condition != 0 {
                            (self.get(registers, *if_true), *if_true)
                        } else {
                            (self.get(registers, *if_false), *if_false)
                        };
                        let avail = self.is_available(taints, sel_reg);
                        self.set(registers, *dst, result);
                        self.set_symbolic(taints, *dst, avail);
                    } else {
                        let placeholder = self.get(registers, *if_true);
                        self.set(registers, *dst, placeholder);
                        self.set_symbolic(taints, *dst, false);
                    }
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
            }

            Instruction::Load {
                kind,
                dst,
                addr,
                memarg,
            } => {
                return self.do_load(cx, registers, taints, *kind, *dst, *addr, *memarg);
            }

            Instruction::Store {
                kind,
                addr,
                val,
                memarg,
            } => {
                let store_op = Directive::Op(Op::Store {
                    kind: *kind,
                    addr: self.operand(registers, taints, *addr),
                    val: self.operand(registers, taints, *val),
                    memarg: *memarg,
                });
                return self.do_store(
                    cx,
                    registers,
                    taints,
                    *kind,
                    *addr,
                    *val,
                    *memarg,
                    in_private_cf,
                    store_op,
                );
            }

            Instruction::MemorySize { dst } => {
                let memory = cx.global.memory().ok_or(Error::MemoryNotDefined)?;
                let pages = memory.size_pages();
                self.set_clear(registers, taints, *dst, Value::from(pages as i32));
                self.advance_ip();
            }

            Instruction::MemoryGrow { dst, pages } => {
                let delta = match resolved {
                    Some(v) => v,
                    None if self.is_symbolic(taints, *pages)
                        && !self.is_available(taints, *pages) =>
                    {
                        return Ok(FrameStepResult::Blocked(Pending::MemoryGrow {
                            pages: self.abs(*pages),
                        }));
                    }
                    None => self.get(registers, *pages).as_i32()?,
                };
                let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
                let result = match memory.grow(delta as u32) {
                    Ok(prev_pages) => prev_pages as i32,
                    Err(_) => -1i32,
                };
                self.set_clear(registers, taints, *dst, Value::from(result));
                self.advance_ip();
            }

            Instruction::MemoryFill { dest, val, len } => {
                if self.addr_unlocatable(taints, *dest) {
                    let d = Directive::Op(Op::MemoryFill {
                        dest: self.operand(registers, taints, *dest),
                        val: self.operand(registers, taints, *val),
                        len: self.operand(registers, taints, *len),
                    });
                    self.mark_all_memory_blind(cx);
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                let size = self.get(registers, *len).as_i32()? as usize;
                let byte_val = self.get(registers, *val).as_i32()? as u8;
                let dest_addr = self.get(registers, *dest).as_i32()? as usize;
                let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
                mem_try!(memory.fill(dest_addr, byte_val, size));
                if !in_private_cf {
                    cx.global
                        .memory_taints_mut()
                        .set_range(dest_addr as u32, size, Class::Public);
                }
                self.advance_ip();
            }

            Instruction::MemoryCopy { dest, src, len } => {
                if self.addr_unlocatable(taints, *dest) || self.addr_unlocatable(taints, *src) {
                    let d = Directive::Op(Op::MemoryCopy {
                        dest: self.operand(registers, taints, *dest),
                        src: self.operand(registers, taints, *src),
                        len: self.operand(registers, taints, *len),
                    });
                    self.mark_all_memory_blind(cx);
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                let size = self.get(registers, *len).as_i32()? as usize;
                let src_addr = self.get(registers, *src).as_i32()? as usize;
                let dest_addr = self.get(registers, *dest).as_i32()? as usize;
                let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
                mem_try!(memory.copy(dest_addr, src_addr, size));
                if !in_private_cf {
                    cx.global
                        .memory_taints_mut()
                        .copy(src_addr as u32, dest_addr as u32, size);
                }
                self.advance_ip();
            }

            Instruction::MemoryInit {
                data_idx,
                dest,
                src_offset,
                len,
            } => {
                if self.addr_unlocatable(taints, *dest) {
                    let d = Directive::Op(Op::MemoryInit {
                        data_idx: *data_idx,
                        dest: self.operand(registers, taints, *dest),
                        src_offset: self.operand(registers, taints, *src_offset),
                        len: self.operand(registers, taints, *len),
                    });
                    self.mark_all_memory_blind(cx);
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                let size = self.get(registers, *len).as_i32()? as usize;
                let offset = self.get(registers, *src_offset).as_i32()? as usize;
                let dest_addr = self.get(registers, *dest).as_i32()? as usize;
                let data = cx
                    .module
                    .data()
                    .get(*data_idx as usize)
                    .ok_or_else(|| Error::Internal("invalid data segment".into()))?;
                let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
                let src_data = data
                    .data
                    .get(offset..offset + size)
                    .ok_or_else(|| Error::Internal("data segment out of bounds".into()))?;
                mem_try!(memory.write_bytes(dest_addr as u32, src_data));
                self.advance_ip();
            }

            Instruction::DataDrop { .. } => {
                self.advance_ip();
            }

            Instruction::Call {
                dst,
                func_idx,
                args,
            } => {
                return self.do_call(cx, registers, taints, *func_idx, args, *dst);
            }

            Instruction::CallIndirect {
                dst,
                type_index,
                table_index: _,
                table_idx,
                args,
            } => {
                let idx = match resolved {
                    Some(v) => v,
                    None if self.is_symbolic(taints, *table_idx)
                        && !self.is_available(taints, *table_idx) =>
                    {
                        return Ok(FrameStepResult::Blocked(Pending::CallIndirect {
                            table_idx: self.abs(*table_idx),
                        }));
                    }
                    None => self.get(registers, *table_idx).as_i32()?,
                };

                let table = cx.global.table();
                if idx < 0 || idx as usize >= table.len() {
                    return Ok(FrameStepResult::Trapped {
                        directive: None,
                        trap: Trap::UndefinedElement,
                    });
                }
                let callee_idx = match table[idx as usize] {
                    Some(callee_idx) => callee_idx,
                    None => {
                        return Ok(FrameStepResult::Trapped {
                            directive: None,
                            trap: Trap::UndefinedElement,
                        });
                    }
                };

                let expected_type = &cx.module.types()[*type_index as usize];
                let callee = cx
                    .module
                    .function(callee_idx)
                    .ok_or(Error::UndefinedFunction(callee_idx))?;
                if callee.func_type() != expected_type {
                    return Ok(FrameStepResult::Trapped {
                        directive: None,
                        trap: Trap::IndirectCallTypeMismatch,
                    });
                }

                return self.do_call(cx, registers, taints, callee_idx, args, *dst);
            }

            Instruction::Arith(arith_instr) => {
                return self.execute_arith(registers, taints, arith_instr);
            }

            Instruction::RefNull { dst, .. } => {
                self.set_clear(registers, taints, *dst, Value::from(0i32));
                self.advance_ip();
            }

            Instruction::RefIsNull { dst, src } => {
                let val = self.get(registers, *src).as_i32()?;
                let result = if val == 0 { 1i32 } else { 0i32 };
                self.set_clear(registers, taints, *dst, Value::from(result));
                self.advance_ip();
            }

            Instruction::RefFunc { dst, func_idx } => {
                self.set_clear(registers, taints, *dst, Value::from(*func_idx as i32));
                self.advance_ip();
            }
        }

        Ok(FrameStepResult::Continue)
    }

    fn execute_arith(
        &mut self,
        registers: &mut [Value],
        taints: &mut Taints,
        instr: &InstructionArith,
    ) -> Result<FrameStepResult, Error> {
        let reg_base = self.reg_base.index();
        match instr {
            InstructionArith::Unary(unary) => {
                if self.is_symbolic(taints, unary.src) {
                    let d = Directive::Op(Op::Unary {
                        dst: self.abs(unary.dst),
                        op: unary.op,
                        src: self.abs(unary.src),
                    });
                    if self.is_available(taints, unary.src) {
                        let (dst, val) = match arithmetic::execute(instr, |reg| {
                            Ok(registers[reg_base + reg.index()])
                        })? {
                            Ok(pair) => pair,
                            Err(trap) => {
                                return Ok(FrameStepResult::Trapped {
                                    directive: Some(d),
                                    trap,
                                });
                            }
                        };
                        registers[reg_base + dst.index()] = val;
                        self.set_symbolic(taints, unary.dst, true);
                    } else {
                        self.set(registers, unary.dst, Value::zero(unary.op.return_ty()));
                        self.set_symbolic(taints, unary.dst, false);
                    }
                    self.advance_ip();
                    return Ok(FrameStepResult::Directive(d));
                }
                let (dst, val) = match arithmetic::execute(instr, |reg| {
                    Ok(registers[reg_base + reg.index()])
                })? {
                    Ok(pair) => pair,
                    Err(trap) => {
                        return Ok(FrameStepResult::Trapped {
                            directive: None,
                            trap,
                        });
                    }
                };
                self.set_clear(registers, taints, dst, val);
                self.advance_ip();
            }
            InstructionArith::Binary(binary) => {
                let lhs_sym = self.is_symbolic(taints, binary.lhs);
                let rhs_sym = self.is_symbolic(taints, binary.rhs);
                let could_trap = binary_could_trap(binary.op);
                let needs_lhs = binary_trap_needs_lhs(binary.op);
                if lhs_sym || rhs_sym {
                    let d = Directive::Op(Op::Binary {
                        dst: self.abs(binary.dst),
                        op: binary.op,
                        lhs: self.operand(registers, taints, binary.lhs),
                        rhs: self.operand(registers, taints, binary.rhs),
                    });
                    let lhs_avail = self.is_available(taints, binary.lhs);
                    let rhs_avail = self.is_available(taints, binary.rhs);
                    self.advance_ip();

                    if could_trap && rhs_avail && (!needs_lhs || lhs_avail) {
                        let lhs = registers[reg_base + binary.lhs.index()];
                        let rhs = registers[reg_base + binary.rhs.index()];
                        if let Some(trap) = binary_trap(binary.op, lhs, rhs) {
                            return Ok(FrameStepResult::Trapped {
                                directive: Some(d),
                                trap,
                            });
                        }
                    }

                    if lhs_avail && rhs_avail {
                        match arithmetic::execute(instr, |reg| {
                            Ok(registers[reg_base + reg.index()])
                        })? {
                            Ok((dst, val)) => {
                                registers[reg_base + dst.index()] = val;
                                self.set_symbolic(taints, binary.dst, true);
                            }
                            Err(trap) => {
                                return Ok(FrameStepResult::Trapped {
                                    directive: Some(d),
                                    trap,
                                });
                            }
                        }
                    } else {
                        self.set(registers, binary.dst, Value::zero(binary.op.return_ty()));
                        self.set_symbolic(taints, binary.dst, false);
                    }
                    return Ok(FrameStepResult::Directive(d));
                }
                match arithmetic::execute(instr, |reg| Ok(registers[reg_base + reg.index()]))? {
                    Ok((dst, val)) => {
                        self.set_clear(registers, taints, dst, val);
                        self.advance_ip();
                    }
                    Err(trap) => {
                        return Ok(FrameStepResult::Trapped {
                            directive: None,
                            trap,
                        });
                    }
                }
            }
        }
        Ok(FrameStepResult::Continue)
    }

    #[allow(clippy::too_many_arguments)]
    fn do_load(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        kind: mpz_vm_ir::LoadKind,
        dst: Reg,
        addr: Reg,
        memarg: mpz_vm_ir::MemArg,
    ) -> Result<FrameStepResult, Error> {
        let byte_size = kind.byte_size() as u32;
        let read_full = |mem: &Memory, a: u32| read_value(mem, kind, a);
        let addr_sym = self.is_symbolic(taints, addr);
        let addr_val = self.get(registers, addr).as_i32()?;
        let eff_addr = (addr_val as u64 + memarg.offset as u64) as u32;
        let memory = cx.global.memory().ok_or(Error::MemoryNotDefined)?;

        let mask = cx.global.memory_taints().symbolic_mask(eff_addr, byte_size);
        let is_symbolic = addr_sym || mask != 0;

        if !is_symbolic {
            let value = match read_full(memory, eff_addr) {
                Ok(value) => value,
                Err(trap) => {
                    return Ok(FrameStepResult::Trapped {
                        directive: None,
                        trap,
                    });
                }
            };
            self.set_clear(registers, taints, dst, value);
            self.advance_ip();
            if let Some(log) = cx.global.access_log_mut() {
                log.push(Access {
                    kind: AccessKind::Read,
                    addr: AccessAddr::Public(eff_addr),
                    width: byte_size as u8,
                    symbolic_mask: 0,
                    value: None,
                    emitted: false,
                    host: false,
                });
            }
            return Ok(FrameStepResult::Continue);
        }

        let addr_op = self.operand(registers, taints, addr);
        let (concrete, symbolic_mask) = if addr_sym {
            (0, 0)
        } else {
            let bytes = match memory.read_bytes(eff_addr, byte_size as usize) {
                Ok(bytes) => bytes,
                Err(trap) => {
                    return Ok(FrameStepResult::Trapped {
                        directive: None,
                        trap,
                    });
                }
            };
            let mut concrete = 0u64;
            for (i, &b) in bytes.iter().enumerate() {
                if mask & (1 << i) == 0 {
                    concrete |= (b as u64) << (i * 8);
                }
            }
            (concrete, mask)
        };
        let d = Directive::Op(Op::Load {
            kind,
            dst: self.abs(dst),
            addr: addr_op,
            memarg,
            concrete,
            symbolic_mask,
        });
        let result_avail = self.is_available(taints, addr)
            && !cx
                .global
                .memory_taints()
                .any_unheld(eff_addr, byte_size as usize);
        if result_avail {
            let value = match read_full(memory, eff_addr) {
                Ok(value) => value,
                Err(trap) => {
                    return Ok(FrameStepResult::Trapped {
                        directive: None,
                        trap,
                    });
                }
            };
            self.set(registers, dst, value);
            self.set_symbolic(taints, dst, true);
        } else {
            self.set(registers, dst, Value::zero(kind.result_ty()));
            self.set_symbolic(taints, dst, false);
        }
        self.advance_ip();
        if let Some(log) = cx.global.access_log_mut() {
            log.push(Access {
                kind: AccessKind::Read,
                addr: if addr_sym {
                    AccessAddr::Symbolic
                } else {
                    AccessAddr::Public(eff_addr)
                },
                width: byte_size as u8,
                symbolic_mask,
                value: None,
                emitted: true,
                host: false,
            });
        }
        Ok(FrameStepResult::Directive(d))
    }

    #[allow(clippy::too_many_arguments)]
    fn do_store(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        kind: mpz_vm_ir::StoreKind,
        addr: Reg,
        val: Reg,
        memarg: mpz_vm_ir::MemArg,
        in_private_cf: bool,
        store_op: Directive,
    ) -> Result<FrameStepResult, Error> {
        let byte_size = kind.byte_size();
        let full_mask = ((1u16 << byte_size) - 1) as u8;
        if self.is_symbolic(taints, addr) {
            if !self.is_available(taints, addr) {
                if let Some(memory) = cx.global.memory() {
                    let len = memory.len();
                    cx.global
                        .memory_taints_mut()
                        .set_range(0, len, Class::Blind);
                }
                self.advance_ip();
                if let Some(log) = cx.global.access_log_mut() {
                    log.push(Access {
                        kind: AccessKind::Write,
                        addr: AccessAddr::Symbolic,
                        width: byte_size as u8,
                        symbolic_mask: full_mask,
                        value: None,
                        emitted: true,
                        host: false,
                    });
                }
                return Ok(FrameStepResult::Directive(store_op));
            }
            let addr_val = self.get(registers, addr).as_i32()?;
            let eff_addr = (addr_val as u64 + memarg.offset as u64) as u32;
            let val_avail = self.is_available(taints, val);
            let bytes = self.get(registers, val).to_le_bytes();
            if let Some(memory) = cx.global.memory() {
                let len = memory.len();
                cx.global.memory_taints_mut().mark_symbolic_range(0, len);
            }
            let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
            mem_try!(memory.write_bytes(eff_addr, &bytes[..byte_size]));
            cx.global.memory_taints_mut().set_range(
                eff_addr,
                byte_size,
                Class::symbolic(val_avail),
            );
            self.advance_ip();
            if let Some(log) = cx.global.access_log_mut() {
                log.push(Access {
                    kind: AccessKind::Write,
                    addr: AccessAddr::Symbolic,
                    width: byte_size as u8,
                    symbolic_mask: full_mask,
                    value: None,
                    emitted: true,
                    host: false,
                });
            }
            return Ok(FrameStepResult::Directive(store_op));
        }
        let addr_val = self.get(registers, addr).as_i32()?;
        let eff_addr = (addr_val as u64 + memarg.offset as u64) as u32;
        let val_sym = self.is_symbolic(taints, val);

        if !val_sym {
            let bytes = self.get(registers, val).to_le_bytes();
            let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
            mem_try!(memory.write_bytes(eff_addr, &bytes[..byte_size]));
            if !in_private_cf {
                cx.global
                    .memory_taints_mut()
                    .set_range(eff_addr, byte_size, Class::Public);
            }
            if let Some(log) = cx.global.access_log_mut() {
                let mut value = 0u64;
                for (i, &b) in bytes[..byte_size].iter().enumerate() {
                    value |= (b as u64) << (i * 8);
                }
                log.push(Access {
                    kind: AccessKind::Write,
                    addr: AccessAddr::Public(eff_addr),
                    width: byte_size as u8,
                    symbolic_mask: 0,
                    value: Some(value),
                    emitted: false,
                    host: false,
                });
            }
        } else {
            let val_avail = self.is_available(taints, val);
            if val_avail {
                let bytes = self.get(registers, val).to_le_bytes();
                let memory = cx.global.memory_mut().ok_or(Error::MemoryNotDefined)?;
                mem_try!(memory.write_bytes(eff_addr, &bytes[..byte_size]));
            }
            cx.global.memory_taints_mut().set_range(
                eff_addr,
                byte_size,
                Class::symbolic(val_avail),
            );
            self.advance_ip();
            if let Some(log) = cx.global.access_log_mut() {
                log.push(Access {
                    kind: AccessKind::Write,
                    addr: AccessAddr::Public(eff_addr),
                    width: byte_size as u8,
                    symbolic_mask: full_mask,
                    value: None,
                    emitted: true,
                    host: false,
                });
            }
            return Ok(FrameStepResult::Directive(store_op));
        }
        self.advance_ip();
        Ok(FrameStepResult::Continue)
    }

    fn do_call(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        func_idx: u32,
        args: &[Reg],
        dst: Option<Reg>,
    ) -> Result<FrameStepResult, Error> {
        let func = cx
            .module
            .function(func_idx)
            .ok_or(Error::UndefinedFunction(func_idx))?;

        match func {
            Function::Import(_) => {
                let args: Vec<Operand> = args
                    .iter()
                    .map(|&r| self.operand(registers, taints, r))
                    .collect();
                self.advance_ip();
                Ok(FrameStepResult::Directive(Directive::Call {
                    dst: dst.map(|r| self.abs(r)),
                    func_idx,
                    args,
                    param_base: Reg(0),
                }))
            }
            Function::Local(_) => {
                let dst = dst.map(|r| self.reg_base + r);
                let args: Vec<Operand> = args
                    .iter()
                    .map(|&r| {
                        let abs = self.reg_base + r;
                        let value = if self.is_available(taints, r) {
                            Some(registers[abs.index()])
                        } else {
                            None
                        };
                        Operand::Symbol { reg: abs, value }
                    })
                    .collect();

                self.advance_ip();
                Ok(FrameStepResult::Call {
                    func_idx,
                    args,
                    dst,
                })
            }
        }
    }

    fn jump_to(&mut self, target: BlockId) {
        self.current_block = target;
        self.ip = 0;
    }

    fn execute_terminator(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        terminator: &Terminator,
        analysis: &FunctionAnalysis,
        in_private_cf: bool,
    ) -> Result<FrameStepResult, Error> {
        match terminator {
            Terminator::Jump { target } => {
                self.jump_to(*target);
                Ok(FrameStepResult::Directive(Directive::Branch {
                    func_idx: self.func_idx,
                    block: self.current_block,
                    cond: None,
                    exit: None,
                    bail_out: false,
                }))
            }
            Terminator::BrCond {
                cond,
                then_target,
                else_target,
                join,
            } => {
                if self.is_symbolic(taints, *cond) {
                    let region = analysis.region(self.current_block);
                    if !self.is_available(taints, *cond) {
                        return self.handle_symbolic_branch(
                            cx,
                            registers,
                            taints,
                            *cond,
                            *join,
                            region,
                            in_private_cf,
                        );
                    }
                    let result = self.handle_symbolic_branch(
                        cx,
                        registers,
                        taints,
                        *cond,
                        *join,
                        region,
                        in_private_cf,
                    )?;
                    let condition = self.get(registers, *cond).as_i32()?;
                    if condition != 0 {
                        self.jump_to(*then_target);
                    } else {
                        self.jump_to(*else_target);
                    }
                    return Ok(result);
                }
                let condition = self.get(registers, *cond).as_i32()?;
                let target = if condition != 0 {
                    *then_target
                } else {
                    *else_target
                };
                self.jump_to(target);
                Ok(FrameStepResult::Directive(Directive::Branch {
                    func_idx: self.func_idx,
                    block: self.current_block,
                    cond: Some(Operand::Concrete(Value::I32(condition))),
                    exit: Some(*join),
                    bail_out: false,
                }))
            }
            Terminator::BrTable {
                idx,
                targets,
                default,
                join,
            } => {
                if self.is_symbolic(taints, *idx) {
                    let region = analysis.region(self.current_block);
                    if !self.is_available(taints, *idx) {
                        return self.handle_symbolic_branch(
                            cx,
                            registers,
                            taints,
                            *idx,
                            *join,
                            region,
                            in_private_cf,
                        );
                    }
                    let result = self.handle_symbolic_branch(
                        cx,
                        registers,
                        taints,
                        *idx,
                        *join,
                        region,
                        in_private_cf,
                    )?;
                    let index = self.get(registers, *idx).as_i32()?;
                    let target = if (index as usize) < targets.len() {
                        targets[index as usize]
                    } else {
                        *default
                    };
                    self.jump_to(target);
                    return Ok(result);
                }
                let index = self.get(registers, *idx).as_i32()?;
                let target = if (index as usize) < targets.len() {
                    targets[index as usize]
                } else {
                    *default
                };
                self.jump_to(target);
                Ok(FrameStepResult::Directive(Directive::Branch {
                    func_idx: self.func_idx,
                    block: self.current_block,
                    cond: Some(Operand::Concrete(Value::I32(index))),
                    exit: Some(*join),
                    bail_out: false,
                }))
            }
            Terminator::Return { values } => {
                let return_reg = if !values.is_empty() {
                    let result = self.get(registers, values[0]);
                    let class = taints.class(self.abs(values[0]).as_u32());
                    if let Some(idx) = self.reg_result {
                        registers[idx.index()] = result;
                        taints.set(idx.as_u32(), class);
                    }
                    Some(values[0])
                } else {
                    None
                };
                Ok(FrameStepResult::Return { return_reg })
            }
            Terminator::Unreachable => Ok(FrameStepResult::Trapped {
                directive: None,
                trap: Trap::Unreachable,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_symbolic_branch(
        &mut self,
        cx: &mut Context<'_>,
        registers: &mut [Value],
        taints: &mut Taints,
        cond_reg: Reg,
        join: BlockId,
        region: &BranchRegion,
        _in_private_cf: bool,
    ) -> Result<FrameStepResult, Error> {
        for &global_idx in &region.globals_written {
            cx.global
                .global_taints_mut()
                .mark_symbolic_range(global_idx, 1);
        }

        if region.has_memory_store
            && let Some(memory) = cx.global.memory()
        {
            let len = memory.len();
            cx.global.memory_taints_mut().mark_symbolic_range(0, len);
        }

        for &reg in &region.registers_written {
            taints.mark_symbolic_range(self.abs(reg).as_u32(), 1);
        }

        if let Some(idx) = self.reg_result {
            taints.mark_symbolic_range(idx.as_u32(), 1);
        }

        let branch_block = self.current_block;

        let exit = if region.join_is_path_independent {
            Some(join)
        } else {
            None
        };

        let bail_out = region.bail_out;

        if self.is_available(taints, cond_reg) {
            return Ok(FrameStepResult::Directive(Directive::Branch {
                func_idx: self.func_idx,
                block: branch_block,
                cond: Some(Operand::Symbol {
                    reg: self.abs(cond_reg),
                    value: Some(self.get(registers, cond_reg)),
                }),
                exit,
                bail_out,
            }));
        }

        if region.join_is_path_independent {
            self.jump_to(join);
        }

        Ok(FrameStepResult::Directive(Directive::Branch {
            func_idx: self.func_idx,
            block: branch_block,
            cond: Some(Operand::Symbol {
                reg: self.abs(cond_reg),
                value: None,
            }),
            exit,
            bail_out,
        }))
    }
}

/// Reads a value of the given [`LoadKind`] from memory at `addr`, applying the
/// kind's width and sign/zero extension.
fn read_value(mem: &Memory, kind: LoadKind, addr: u32) -> MaybeTrap<Value> {
    let byte_size = kind.byte_size();
    let signed = kind.is_signed();
    match kind {
        LoadKind::I32 => mem.read_i32(addr).map(Value::from),
        LoadKind::I64 => mem.read_i64(addr).map(Value::from),
        LoadKind::F32 => mem.read_f32(addr).map(Value::from),
        LoadKind::F64 => mem.read_f64(addr).map(Value::from),
        LoadKind::I32Load8S | LoadKind::I32Load8U | LoadKind::I32Load16S | LoadKind::I32Load16U => {
            mem.read_i32_partial(addr, byte_size, signed)
                .map(Value::from)
        }
        LoadKind::I64Load8S
        | LoadKind::I64Load8U
        | LoadKind::I64Load16S
        | LoadKind::I64Load16U
        | LoadKind::I64Load32S
        | LoadKind::I64Load32U => mem
            .read_i64_partial(addr, byte_size, signed)
            .map(Value::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Global, Op, Param, call::Call};
    use mpz_vm_ir::{ExportKind, Module, ValType};

    fn export_func_idx(module: &Module, name: &str) -> u32 {
        module
            .exports()
            .iter()
            .find_map(|e| match e.kind {
                ExportKind::Func(idx) if e.name == name => Some(idx),
                _ => None,
            })
            .expect("function should be exported")
    }

    #[test]
    fn memory_grow_blind_pages_blocks_then_resolves() {
        let wat = r#"
            (module
              (memory 1)
              (func (export "g") (param i32) (result i32)
                local.get 0
                memory.grow))
        "#;
        let module = Module::parse(&wat::parse_str(wat).unwrap()).unwrap();
        let func_idx = export_func_idx(&module, "g");
        let mut global = Global::new(&module).unwrap();
        let mut thread = Thread::new();
        thread
            .call(
                &module,
                &mut global,
                Call {
                    func_idx,
                    params: vec![Param::Blind(ValType::I32)],
                },
            )
            .unwrap();

        let mut blocked = false;
        loop {
            match thread.step(&module, &mut global).unwrap() {
                StepResult::Blocked(Pending::MemoryGrow { .. }) => {
                    blocked = true;
                    thread.resolve_memory_grow(1).unwrap();
                }
                StepResult::Done { result, .. } => {
                    assert_eq!(result, Some(Value::I32(1)));
                    break;
                }
                StepResult::Trapped { trap, .. } => panic!("unexpected trap: {trap:?}"),
                _ => {}
            }
        }
        assert!(blocked, "a blind page count must block");
    }

    const DIV_WAT: &str = r#"
        (module
          (func (export "div") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.div_u))
    "#;

    fn div_module() -> Module {
        let bytes = wat::parse_str(DIV_WAT).expect("wat should compile");
        Module::parse(&bytes).expect("module should parse")
    }

    fn div_func_idx(module: &Module) -> u32 {
        use mpz_vm_ir::ExportKind;
        module
            .exports()
            .iter()
            .find_map(|e| match e.kind {
                ExportKind::Func(idx) if e.name == "div" => Some(idx),
                _ => None,
            })
            .expect("div should be exported")
    }

    fn setup_div_params(module: &Module, params: Vec<Param>) -> (Thread, Global) {
        let func_idx = div_func_idx(module);
        let mut global = Global::new(module).expect("global");
        let mut thread = Thread::new();
        thread
            .call(module, &mut global, Call { func_idx, params })
            .expect("call should set up the frame");
        (thread, global)
    }

    fn setup_div_thread(module: &Module, a: i32, b: i32) -> (Thread, Global) {
        setup_div_params(
            module,
            vec![Param::Private(Value::I32(a)), Param::Private(Value::I32(b))],
        )
    }

    fn is_div_u_directive(d: &Directive) -> bool {
        matches!(
            d,
            Directive::Op(Op::Binary {
                op: BinaryOp::I32DivU,
                ..
            })
        )
    }

    #[test]
    fn blind_divisor_div_u_emits_directive_and_continues() {
        let module = div_module();
        let (mut thread, mut global) = setup_div_params(
            &module,
            vec![Param::Public(Value::I32(6)), Param::Blind(ValType::I32)],
        );

        let mut div_index = None;
        loop {
            let pre_counter = thread.op_counter();
            match thread.step(&module, &mut global).expect("step") {
                StepResult::Directive(d) if is_div_u_directive(&d) => {
                    assert_eq!(thread.op_counter(), pre_counter + 1);
                    assert!(!thread.is_done());
                    div_index = Some(pre_counter);
                }
                StepResult::Done { .. } => break,
                StepResult::Continue | StepResult::Directive(_) => {}
                StepResult::Blocked(_) => panic!("a blind divisor must not block"),
                StepResult::Trapped { .. } => panic!("a blind divisor must not self-trap"),
            }
        }
        assert!(
            div_index.is_some(),
            "should have emitted the div_u directive"
        );
    }

    #[test]
    fn concrete_div_by_zero_traps() {
        let module = div_module();
        let (mut thread, mut global) = setup_div_thread(&module, 6, 0);

        let mut trapped = false;
        loop {
            let pre_counter = thread.op_counter();
            let result = { thread.step(&module, &mut global).expect("step") };
            match result {
                StepResult::Trapped {
                    index,
                    directive,
                    trap,
                } => {
                    let directive = directive.expect("symbolic div_u carries its directive");
                    assert!(is_div_u_directive(&directive), "trapped on the div_u");
                    assert_eq!(trap, Trap::DivideByZero);
                    assert_eq!(index, pre_counter, "trapped index is the op's slot");
                    assert!(thread.is_done());
                    trapped = true;
                    break;
                }
                StepResult::Blocked(_) => panic!("held operands must not block"),
                StepResult::Done { .. } => break,
                _ => {}
            }
        }
        assert!(trapped, "div by zero should trap when operands are held");
    }

    #[test]
    fn op_counter_matches_for_div_u() {
        let module = div_module();

        let run = |blind_divisor: bool| -> u64 {
            let divisor = if blind_divisor {
                Param::Blind(ValType::I32)
            } else {
                Param::Private(Value::I32(2))
            };
            let (mut thread, mut global) =
                setup_div_params(&module, vec![Param::Public(Value::I32(6)), divisor]);
            loop {
                let pre_counter = thread.op_counter();
                match thread.step(&module, &mut global).expect("step") {
                    StepResult::Directive(d) if is_div_u_directive(&d) => {
                        return pre_counter;
                    }
                    StepResult::Done { .. } => panic!("should reach the div_u"),
                    StepResult::Trapped { .. } => panic!("non-trapping divisor"),
                    StepResult::Blocked(_) => panic!("div_u must not block"),
                    _ => {}
                }
            }
        };

        assert_eq!(
            run(false),
            run(true),
            "the div_u op must get the same index whether or not the divisor is held"
        );
    }

    #[test]
    fn self_traps_on_public_zero_divisor() {
        let module = div_module();
        let (mut thread, mut global) = setup_div_params(
            &module,
            vec![Param::Private(Value::I32(7)), Param::Public(Value::I32(0))],
        );

        let mut trapped = false;
        loop {
            let pre = thread.op_counter();
            let result = { thread.step(&module, &mut global).expect("step") };
            match result {
                StepResult::Trapped {
                    index,
                    directive,
                    trap,
                } => {
                    let directive = directive.expect("symbolic dividend carries the directive");
                    assert!(is_div_u_directive(&directive));
                    assert_eq!(trap, Trap::DivideByZero);
                    assert_eq!(index, pre);
                    assert!(thread.is_done());
                    trapped = true;
                    break;
                }
                StepResult::Blocked(_) => panic!("must not block: the divisor is public"),
                StepResult::Done { .. } => break,
                _ => {}
            }
        }
        assert!(
            trapped,
            "public zero divisor should trap locally without blocking"
        );
    }

    #[test]
    fn public_div_by_zero_traps_without_directive() {
        let module = div_module();
        {
            let (mut thread, mut global) = setup_div_params(
                &module,
                vec![Param::Public(Value::I32(7)), Param::Public(Value::I32(0))],
            );
            let mut trapped = false;
            loop {
                let result = {
                    thread
                        .step(&module, &mut global)
                        .expect("step is Ok even on a trap")
                };
                match result {
                    StepResult::Trapped {
                        directive, trap, ..
                    } => {
                        assert!(
                            directive.is_none(),
                            "a public op carries no symbolic directive"
                        );
                        assert_eq!(trap, Trap::DivideByZero);
                        assert!(thread.is_done());
                        trapped = true;
                        break;
                    }
                    StepResult::Blocked(_) => panic!("public operands must not block"),
                    StepResult::Done { .. } => break,
                    _ => {}
                }
            }
            assert!(trapped, "public div-by-zero should trap");
        }
    }

    /// Steps `thread` to completion, collecting every emitted [`Directive`].
    fn run_collect(module: &Module, global: &mut Global, thread: &mut Thread) -> Vec<Directive> {
        let mut directives = Vec::new();
        loop {
            match thread.step(module, global).expect("step") {
                StepResult::Directive(d) => directives.push(d),
                StepResult::Continue => {}
                StepResult::Done { .. } => break,
                StepResult::Trapped { trap, .. } => panic!("unexpected trap: {trap:?}"),
                StepResult::Blocked(p) => panic!("unexpected block: {p:?}"),
            }
        }
        directives
    }

    /// The per-byte symbolic axis of linear memory, LSB-address first.
    fn memory_symbolic_axis(global: &Global) -> Vec<bool> {
        let len = global.memory().expect("memory should be defined").len();
        (0..len as u32)
            .map(|i| global.memory_tainted(i, 1))
            .collect()
    }

    #[test]
    fn symbolic_address_store_smears_symbolic_axis_identically() {
        let wat = r#"
            (module
              (memory 1)
              (func (export "s") (param i32 i32)
                local.get 0
                local.get 1
                i32.store))
        "#;
        let module = Module::parse(&wat::parse_str(wat).unwrap()).unwrap();
        let func_idx = export_func_idx(&module, "s");

        let mut prover_global = Global::new(&module).unwrap();
        let mut prover = Thread::new();
        prover
            .call(
                &module,
                &mut prover_global,
                Call {
                    func_idx,
                    params: vec![
                        Param::Private(Value::I32(8)),
                        Param::Private(Value::I32(0x1122_3344)),
                    ],
                },
            )
            .unwrap();
        run_collect(&module, &mut prover_global, &mut prover);

        let mut verifier_global = Global::new(&module).unwrap();
        let mut verifier = Thread::new();
        verifier
            .call(
                &module,
                &mut verifier_global,
                Call {
                    func_idx,
                    params: vec![Param::Blind(ValType::I32), Param::Blind(ValType::I32)],
                },
            )
            .unwrap();
        run_collect(&module, &mut verifier_global, &mut verifier);

        let prover_axis = memory_symbolic_axis(&prover_global);
        let verifier_axis = memory_symbolic_axis(&verifier_global);
        assert_eq!(
            prover_axis, verifier_axis,
            "symbolic axes must match after a symbolic-address store"
        );
        assert!(
            prover_axis.iter().all(|&b| b),
            "the whole memory must be smeared symbolic"
        );
    }

    #[test]
    fn symbolic_address_load_leaves_memory_taint_untouched() {
        let wat = r#"
            (module
              (memory 1)
              (func (export "l") (param i32) (result i32)
                local.get 0
                i32.load))
        "#;
        let module = Module::parse(&wat::parse_str(wat).unwrap()).unwrap();
        let func_idx = export_func_idx(&module, "l");

        let mut prover_global = Global::new(&module).unwrap();
        let mut prover = Thread::new();
        prover
            .call(
                &module,
                &mut prover_global,
                Call {
                    func_idx,
                    params: vec![Param::Private(Value::I32(8))],
                },
            )
            .unwrap();
        let before = memory_symbolic_axis(&prover_global);
        let directives = run_collect(&module, &mut prover_global, &mut prover);

        let mut verifier_global = Global::new(&module).unwrap();
        let mut verifier = Thread::new();
        verifier
            .call(
                &module,
                &mut verifier_global,
                Call {
                    func_idx,
                    params: vec![Param::Blind(ValType::I32)],
                },
            )
            .unwrap();
        run_collect(&module, &mut verifier_global, &mut verifier);

        assert!(
            directives
                .iter()
                .any(|d| matches!(d, Directive::Op(Op::Load { .. }))),
            "a symbolic-address load must emit its directive"
        );
        assert_eq!(
            memory_symbolic_axis(&prover_global),
            before,
            "a symbolic-address load must not touch memory taint"
        );
        assert!(
            memory_symbolic_axis(&verifier_global).iter().all(|&b| !b),
            "a symbolic-address load must not touch memory taint on the verifier"
        );
    }

    #[test]
    fn concrete_store_stays_silent_and_logs() {
        let overwrite_wat = r#"
            (module
              (memory 1)
              (func (export "f") (param i32)
                i32.const 0
                local.get 0
                i32.store
                i32.const 0
                i32.const 42
                i32.store))
        "#;
        let module = Module::parse(&wat::parse_str(overwrite_wat).unwrap()).unwrap();
        let func_idx = export_func_idx(&module, "f");
        let mut global = Global::new(&module).unwrap();
        global.enable_access_log();
        let mut thread = Thread::new();
        thread
            .call(
                &module,
                &mut global,
                Call {
                    func_idx,
                    params: vec![Param::Private(Value::I32(7))],
                },
            )
            .unwrap();
        let directives = run_collect(&module, &mut global, &mut thread);
        let concrete_stores = directives
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    Directive::Op(Op::Store {
                        val: Operand::Concrete(_),
                        ..
                    })
                )
            })
            .count();
        assert_eq!(
            concrete_stores, 0,
            "a concrete store must stay silent even when overwriting symbolic bytes"
        );
        let log = global.access_log().expect("access log should be enabled");
        let overwrite = log
            .entries()
            .iter()
            .find(|a| matches!(a.kind, AccessKind::Write) && a.value == Some(42))
            .expect("the concrete overwrite should be logged");
        assert!(
            !overwrite.emitted,
            "the logged overwrite must be marked unemitted"
        );

        let public_wat = r#"
            (module
              (memory 1)
              (func (export "g")
                i32.const 0
                i32.const 42
                i32.store))
        "#;
        let module = Module::parse(&wat::parse_str(public_wat).unwrap()).unwrap();
        let func_idx = export_func_idx(&module, "g");
        let mut global = Global::new(&module).unwrap();
        let mut thread = Thread::new();
        thread
            .call(
                &module,
                &mut global,
                Call {
                    func_idx,
                    params: vec![],
                },
            )
            .unwrap();
        let directives = run_collect(&module, &mut global, &mut thread);
        let stores = directives
            .iter()
            .filter(|d| matches!(d, Directive::Op(Op::Store { .. })))
            .count();
        assert_eq!(
            stores, 0,
            "a concrete store over public bytes must stay silent"
        );
    }

    const MIXED_WAT: &str = r#"
        (module
          (memory 1)
          (func (export "mixed") (param i32 i32)
            ;; silent public store at address 0
            i32.const 0
            i32.const 42
            i32.store
            ;; fast-path public load at address 0
            i32.const 0
            i32.load
            drop
            ;; symbolic-value store at public address 8
            i32.const 8
            local.get 0
            i32.store
            ;; concrete store overwriting the symbolic bytes at address 8
            i32.const 8
            i32.const 99
            i32.store
            ;; symbolic-address store
            local.get 1
            i32.const 7
            i32.store))
    "#;

    fn mixed_module() -> Module {
        Module::parse(&wat::parse_str(MIXED_WAT).expect("wat should compile"))
            .expect("module should parse")
    }

    fn prover_mixed_params() -> Vec<Param> {
        vec![
            Param::Private(Value::I32(0x1122_3344)),
            Param::Private(Value::I32(16)),
        ]
    }

    fn verifier_mixed_params() -> Vec<Param> {
        vec![Param::Blind(ValType::I32), Param::Blind(ValType::I32)]
    }

    fn run_mixed(params: Vec<Param>, enable_log: bool) -> (Global, Vec<Directive>) {
        let module = mixed_module();
        let func_idx = export_func_idx(&module, "mixed");
        let mut global = Global::new(&module).unwrap();
        if enable_log {
            global.enable_access_log();
        }
        let mut thread = Thread::new();
        thread
            .call(&module, &mut global, Call { func_idx, params })
            .unwrap();
        let directives = run_collect(&module, &mut global, &mut thread);
        (global, directives)
    }

    /// Collapses a directive to its shape, ignoring operand values, so two runs
    /// can be compared for a matching directive skeleton.
    fn directive_skeleton(d: &Directive) -> &'static str {
        match d {
            Directive::Call { .. } => "call",
            Directive::Return { .. } => "return",
            Directive::Branch { .. } => "branch",
            Directive::Op(Op::Load { .. }) => "load",
            Directive::Op(Op::Store { .. }) => "store",
            Directive::Op(_) => "op",
        }
    }

    #[test]
    fn access_log_disabled_by_default() {
        let (global, _directives) = run_mixed(prover_mixed_params(), false);
        assert!(
            global.access_log().is_none(),
            "the access log must be off by default"
        );
    }

    #[test]
    fn access_log_does_not_change_behavior() {
        let (off_global, off_dirs) = run_mixed(prover_mixed_params(), false);
        let (on_global, on_dirs) = run_mixed(prover_mixed_params(), true);

        let off_skeleton: Vec<_> = off_dirs.iter().map(directive_skeleton).collect();
        let on_skeleton: Vec<_> = on_dirs.iter().map(directive_skeleton).collect();
        assert_eq!(
            off_skeleton, on_skeleton,
            "enabling the log must not change the directive stream"
        );
        assert_eq!(
            memory_symbolic_axis(&off_global),
            memory_symbolic_axis(&on_global),
            "enabling the log must not change memory taint"
        );
    }

    #[test]
    fn prover_verifier_access_logs_match() {
        let (prover_global, _) = run_mixed(prover_mixed_params(), true);
        let (verifier_global, _) = run_mixed(verifier_mixed_params(), true);

        let prover = prover_global.access_log().expect("log enabled");
        let verifier = verifier_global.access_log().expect("log enabled");
        assert_eq!(
            prover.entries(),
            verifier.entries(),
            "the access log must match field-for-field on both parties"
        );

        let prover_sym = prover
            .entries()
            .last()
            .expect("the symbolic-address store is the last access");
        let verifier_sym = verifier
            .entries()
            .last()
            .expect("the symbolic-address store is the last access");
        assert!(
            matches!(prover_sym.addr, AccessAddr::Symbolic),
            "the symbolic-address store must have a symbolic addr on the prover"
        );
        assert!(
            matches!(verifier_sym.addr, AccessAddr::Symbolic),
            "the symbolic-address store must have a symbolic addr on the verifier"
        );
    }

    #[test]
    fn emitted_entries_zip_memory_directives() {
        let (global, directives) = run_mixed(prover_mixed_params(), true);
        let log = global.access_log().expect("log enabled");

        let emitted: Vec<&Access> = log.entries().iter().filter(|a| a.emitted).collect();
        let mem_dirs: Vec<&Directive> = directives
            .iter()
            .filter(|d| {
                matches!(
                    d,
                    Directive::Op(Op::Load { .. }) | Directive::Op(Op::Store { .. })
                )
            })
            .collect();

        assert_eq!(
            emitted.len(),
            mem_dirs.len(),
            "each emitted entry corresponds to one memory directive"
        );
        for (access, directive) in emitted.iter().zip(mem_dirs.iter()) {
            match (access.kind, directive) {
                (AccessKind::Read, Directive::Op(Op::Load { .. })) => {}
                (AccessKind::Write, Directive::Op(Op::Store { .. })) => {}
                _ => panic!("emitted entry {access:?} does not match directive {directive:?}"),
            }
        }
    }

    #[test]
    fn drain_upto_advances_base_and_preserves_clock() {
        let (mut global, _) = run_mixed(prover_mixed_params(), true);
        let all: Vec<Access> = global.access_log().expect("log enabled").entries().to_vec();
        let clock = global.access_log().expect("log enabled").clock();
        assert_eq!(clock, all.len() as u64);

        let log = global.access_log_mut().expect("log enabled");
        log.drain_upto(2);
        assert_eq!(log.base(), 2, "draining two entries advances base to 2");
        assert_eq!(
            log.clock(),
            clock,
            "draining must not change the next clock"
        );
        assert_eq!(
            log.entries(),
            &all[2..],
            "the resident suffix must start at clock 2"
        );

        let end = log.clock();
        log.drain_upto(end);
        assert!(
            log.entries().is_empty(),
            "draining up to the clock empties the log"
        );
        assert_eq!(
            log.base(),
            clock,
            "the emptied log keeps the full clock as its base"
        );
        assert_eq!(log.clock(), clock);
    }
}
