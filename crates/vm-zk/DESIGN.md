# vm-zk — A streaming VOLE-based zero-knowledge VM for WASM-IR

`vm-zk` is a designated-verifier zero-knowledge virtual machine built on a
VOLE-based circuit argument (the QuickSilver family): no trusted setup, and no
public-key cryptography on the hot path. It proves that a program — an
`mpz-vm-ir` module compiled from WebAssembly — executed correctly on a private
witness, without revealing that witness. Both parties drive the same program
through the `Vm` trait; the prover holds the secret inputs and emits a proof,
the verifier checks it against the public inputs and outputs.

This document is the architectural reference for the system. It describes the
target design as a whole, organised around the single concern that shapes every
decision: **performance** — proving throughput, bounded memory, and parallel
scaling. Mechanisms that are designed but not yet wired are marked *(target)* at
the point of description and consolidated in §13.

**Vocabulary.** Five nouns recur. A **wire** is one authenticated bit (§4.1). A
**tape** is a vector of correlation drawn from sVOLE, consumed one entry per
commitment (§4.1). The **skeleton** is the deterministic public structure of an
execution both parties derive identically (§5.5). A **chunk** is one streamed,
bounded unit of proving (§6); a **segment** is a parallel sub-range of a chunk
(§7).

---

## 1. Purpose and scope

The VM executes the integer subset of WebAssembly semantics (i32/i64 arithmetic,
bitwise, shifts, comparisons, division with advice, linear memory, globals,
calls, structured control flow) plus a small set of host calls (value reveals
and cryptographic precompiles). Floating point is out of scope for the proof.

The proof is **interactive and designated-verifier**: there is no public
verifiability and no trusted setup. Interaction is one challenge round per chunk
(commit → challenge), plus one closing round-trip for the whole-call memory check
(§8.6). Soundness rests on the verifier holding a secret MAC key `Δ` and
sampling fresh challenges; this trades universal verifiability for speed.

---

## 2. Trust and security model

- **Parties.** A prover and a verifier connected by a transport-agnostic
  channel. The protocol is executor- and transport-agnostic (`Sink`/`Stream`),
  with correlated randomness supplied by an sVOLE channel.
- **Privacy.** The verifier learns the program, the public inputs/outputs, any
  values the prover explicitly reveals, and the *public skeleton* of execution
  (control flow and addresses that are public — see §5.3, §5.5). It learns
  nothing else about the witness.
- **Soundness.** A cheating prover convinces the verifier with probability
  negligible in the field size. Every authenticated value carries an IT-MAC
  under `Δ`; challenges are sampled by the verifier *after* the prover commits,
  so the witness cannot be adapted to them.
- **Determinism.** Both parties derive an identical execution skeleton from the
  program and public inputs. All protocol-shaping quantities (chunk boundaries,
  segment marks (§7), tape sizes (§4.1), challenge counts) are functions of that
  shared skeleton, never of private data or local machine characteristics. This
  is what lets the two sides stay in lockstep without negotiation, and the same
  guarantee that admits cross-chunk pipelining (§12).

### 2.1 Security parameters

| Component | Field / primitive | Per-check error |
|-----------|-------------------|-----------------|
| MAC and gate challenge | `GF(2^128)` | ≈ 2⁻¹²⁸ |
| Memory grand product *(target)* | `GF(2^64)`, challenges `r`, `γ` | ≈ 2⁻⁶⁴ per entry, amortised over the trace |
| Assertion binding | blake3 (256-bit) | 2⁻¹²⁸ collision |
| Output mask | 128-bit VOPE line | information-theoretic |

The MAC key satisfies `Δ` LSB = 1 (a protocol invariant): the low-bit pointer
convention that lets a wire carry its bit in the MAC's LSB requires it. One
fresh challenge is drawn per multiplication and per polynomial constraint; the
overall soundness error is a union bound over all checks in a proof.

---

## 3. Design principles (the performance north star)

Every structural choice below follows from one of these. Each names the section
that realises it.

1. **Public work is free** (§5.3). The proof pays only for *authenticated*
   computation. Any value, branch, or address both parties can compute is
   evaluated in the clear and contributes zero gates, zero commitment, zero
   communication. The taint and skeleton machinery (§5.4–5.5) exists to separate
   public structure from private witness so only the witness-dependent residue
   enters the circuit.

2. **Commit little, derive much** (§4.1, §4.5, §7.1, §8.3–8.4). sVOLE correlation is the
   dominant cost, so the design minimises committed bits. IT-MACs combine
   linearly for free, so only nonlinear gates and genuine inputs are committed.
   Three mechanisms push this further: boundary *deltas* commit only what a
   segment writes; the accelerated grand product commits only chunk-boundary
   partial products; the composite polynomial check commits no intermediate
   products at all.

3. **Stream to bound memory** (§6). Execution is proven in fixed-size *chunks* so
   that per-chunk proving buffers stay bounded regardless of trace length. A
   chunk's gate budget is sized so its correlation demand typically fits a single
   Ferret extension round.

4. **Parallelise within the bound** (§7). Each chunk splits into *segments*
   proven by independent workers. The accumulate pass is linear in the verifier's
   challenge weights, so disjoint trace ranges fold independently and combine by
   field addition; segment workers exchange no wires, only re-materialise shared
   commitments off a common tape.

5. **One allocation, two passes, fewest rounds** (§4.3, §6.2). Correlated
   randomness is challenge-independent, so each chunk draws it in a single
   allocation — and that same independence lets the allocation be prefetched for
   the next chunk under the current one's proving (§12). The prover walks the
   circuit twice — a cheap plaintext *commit* pass and a proof-folding
   *accumulate* pass — separated by exactly the challenges soundness requires.

6. **One field where it suffices, two where it pays** (§4.6). Bit-level VM
   semantics run over `GF(2)`; the memory permutation argument runs over
   `GF(2^64)`, where one element carries 64 packed bits and the grand product
   shrinks accordingly. Both share the same `GF(2^128)` MACs and fold into one
   proof.

---

## 4. Proof system foundation

### 4.1 Authenticated wires

Every secret bit `b` is carried as an information-theoretic MAC: the prover holds
`M = K + b·Δ`, the verifier holds the key `K`, and only the verifier knows `Δ`.
Addition of wires is local on both sides (XOR of MACs / keys), so linear circuit
structure is free. The convention packs the bit into the low bit of the MAC, so a
wire is a single `GF(2^128)` element. Public constants use fixed, agreed wires.

A **tape** is a vector of such correlations drawn from sVOLE: the prover gets
`(mask, MAC)` per entry, the verifier gets the matching key. Committing a witness
bit means recording one *adjustment* bit (the difference between the bit and its
mask); the verifier applies the adjustment to its key to reach the same wire.

### 4.2 QuickSilver: the multiplication check and assertion binding

Linear gates are free; the proof only has to certify multiplications and constant
assertions. For each multiplication the prover accumulates two `GF(2^128)` running
sums `(u, v)` weighted by a per-gate verifier challenge, and the verifier
accumulates a single `w`. A correct circuit satisfies `w = u + Δ·v`; a single
cheated gate breaks the identity with overwhelming probability — which is exactly
why each gate draws an *independent* challenge rather than a shared power series.

Constant assertions bind public facts into the proof. Each one folds the
asserted wire (the MAC on the prover, the key on the verifier) into a running
blake3 **assertion hash**; per-segment hashes are concatenated and re-hashed. The
assertion hash is the binding primitive for outputs, reveals, and operand-bound
traps: a cheated asserted value changes the folded wire and so the digest, and
the verifier accepts only when its digest matches the prover's alongside the
masked `(u, v)` check.

### 4.3 The two-pass protocol

The prover walks the circuit twice:

- **Commit pass** — mask-only plaintext evaluation. It XORs each witness bit and
  each gate output into the mask tape in place, never touching MACs, producing
  the adjustment bits. It reads only wire LSBs — no field multiply, no challenge
  draw — so it is far cheaper per gate than accumulate, and embarrassingly
  parallel.
- **Accumulate pass** — run from the committed tape with the verifier's challenge
  stream installed, folding every multiplication and assertion into `(u, v)` and
  the assertion hash.

The verifier performs a single accumulate pass over the same circuit from the
commitment it received, materialising its wires from the adjustment bits. The
prover therefore does ~2× the gate walks of the verifier; the commit pass cannot
be fused into allocation because it *produces* the adjustment bits the verifier
needs before any challenge exists. Because both accumulate passes are *linear in
the challenge weights*, a trace may be folded in disjoint sub-ranges — each over
its slice of the tape with the challenge stream seeked to its gate offset — and
the partials summed. This linearity is the foundation of segment parallelism
(§7) and of cross-chunk pipelining (§12).

### 4.4 VOPE masking

The raw `(u, v)` would leak witness information, so before sending them the prover
masks them with a VOPE (vector oblivious polynomial evaluation) correlation — a
one-time affine line in `Δ` whose verifier-side value cancels in the final check.
The verifier accepts iff the masked triple identity holds and the assertion
hashes match. One VOPE line covers the whole chunk and is amortised across all
its segments, so the proof is constant-size per chunk: an assertion hash, two
field elements, and (when polynomial constraints are present) a fixed-length
coefficient vector.

### 4.5 Composite polynomial constraints *(target)*

Some relations are naturally high-degree (the memory grand product is the leading
example). Rather than committing every intermediate product, the system verifies
a degree-`d` polynomial constraint `f(w) = 0` directly: the prover folds each
constraint into per-degree accumulators and sends `d_max` masked coefficients per
proof; the verifier batches all constraints into one check against powers of `Δ`.
This keeps the committed footprint independent of constraint degree — the
mechanism that makes the accelerated grand product worth its degree. The
machinery is built (`zk-core::poly`); it is exercised once the memory argument is
wired (§13).

### 4.6 The GF(2^64) extension *(target)*

The same machinery runs over `GF(2^64)` instead of single bits, keeping the
`GF(2^128)` MACs (a subfield IT-MAC). Sixty-four committed boolean bits *pack*
injectively into one `GF(2^64)` wire for free — the canonical lift the memory
argument relies on to turn already-committed tuples into field elements without
fresh correlation. Both the multiplication check and the polynomial check are
available in this field. The two fields share the proof: their `(u, v)`
accumulators add, their polynomial states merge, and one VOPE masks the combined
result. The contexts are built (`zk-core::gf64`); they enter the pipeline with
the memory argument (§13).

---

## 5. Execution model

### 5.1 Programs and the IR

A program is an `mpz-vm-ir` `Module`: function signatures, imported and local
functions, linear memories, globals, tables, and data segments. Local functions
are control-flow graphs over a register IR; imports are host calls (reveals and
precompiles). Both parties load the same module and drive it through the `Vm`
trait: `write` stages inputs, `call` proves a function, `read`/`reveal` query
results.

Execution is interpreted by a `Thread` over shared `Global` state (concrete
memory, globals, tables, and per-byte/per-global visibility taints). Stepping the
thread yields a stream of **directives** — `Op` (copy, global get/set, binary,
unary, load, store), `Call`, `Return`, `Branch` — the linearised, register-level
skeleton of execution.

### 5.2 Public versus symbolic

Each value is either **public** (concrete, known to both parties) or **symbolic**
(authenticated, known in clear only to the holding party). Symbolic in turn
splits into Private and Blind by which party holds the bits (§5.4), and the mask
that blends mixed loads is read off memory taint. The distinction is per operand,
per memory byte, and per global. Operations on entirely public operands are
executed in the clear and never enter the circuit; mixed loads blend public and
symbolic bytes by mask; a constant operand selects cheaper, constant-specialised
gadget variants. This is principle 1: the cost of a program is the cost of its
*symbolic* residue, not its instruction count.

Control flow follows from this. The captured directive trace is already the
linearised taken path, so a `Branch` is a no-op in replay and emits zero gates —
free public structure. The current design supports only public (publicly
deducible) branch conditions; a symbolic condition is rejected as unsupported,
as are a symbolic indirect-call table index and a symbolic `memory.grow` count,
because resolving them privately would diverge the shared skeleton (§5.5). These
are the boundaries of the supported directive set.

### 5.3 Authenticated state

Alongside the concrete thread state, the proof tracks an `AuthState`:
authenticated registers, globals, and byte-addressed linear memory, each value a
bundle of authenticated bits (`I32`/`I64` as 32/64 wires). This is the witness as
the circuit sees it. Public-addressed loads and stores of routable bytes are pure
wire routing — slicing and concatenating bytes — and cost no gates; only
arithmetic and logic gates do. Reads of unroutable bytes and writes under an open
bracket instead enter the memory argument (§8).

### 5.4 Memory tainting

The public-versus-symbolic split of §5.2 is not a property a value carries on its
own; it is read off **taint** — per-item visibility state both parties maintain
in lockstep. Taint is the mechanism that decides what is authenticated and what
is free, that keeps the two skeletons identical, and that stops either party from
reading committed memory it is not entitled to.

**Three classes.** Every register, global, and memory byte is one of three
visibility classes: **Public** (concrete, both parties hold the cleartext),
**Private** (symbolic, the *local* party holds the bits), or **Blind** (symbolic,
*another* party holds the bits and the local party does not). The split between
Private and Blind is purely local and dual: a private input the prover stages is
exactly the blind input the verifier reserves for it — the same wire from both
sides. Each taint domain stores a *symbolic* axis and a *held* axis, so an item
is Public when not symbolic, Blind when symbolic and unheld, and Private when
symbolic and held. The symbolic axis drives the proof; the held axis only records
which side can read the plaintext and never differs in shape between the parties.

**Three domains.** Taint lives wherever a value lives. Register taints travel
with the executing thread — frame-scoped, reclaimed when a frame pops — while
global taints and byte-granular memory taints live in the shared `Global`
alongside the concrete state. Memory taint is per byte because WASM stores have
partial widths: a `store8` can taint one byte of a word a later load reads whole.

**Taint as the authenticated/free decision.** An operation enters the circuit
only when a taint says it must. A binary op on two public operands is evaluated
in the clear and emits no directive; a load whose address and bytes are all
public returns a concrete value and never touches a wire. The pivotal case is the
*mixed* load. Capture computes a per-byte **symbolic mask** over the loaded range
straight from the memory taints — bit `i` set iff byte `i` is symbolic — and
records it on the load directive together with the public bytes (symbolic
positions zeroed). Replay consumes the mask byte by byte: a symbolic byte sources
its committed wires from authenticated memory, a public byte is materialised as
public-bit wires on the fly. The mask is the exact line between paid and free work
*inside a single instruction*, and because it is read off shared taint both
parties emit the identical blend. This is principle 1 at sub-instruction
granularity, and the concrete reason memory taint is tracked per byte.

**Propagation.** Public computation moves cleartext and clears taint: storing a
concrete value marks its bytes Public, erasing any earlier symbolic taint there,
so a region overwritten in the clear costs nothing downstream. Storing a symbolic
value taints the written bytes — Private if the storing party holds them, Blind
otherwise — and a load of any symbolic byte taints its destination register. This
is why a boundary snapshot must consult live taint rather than the write log
alone: a byte written symbolically inside a segment but publicly overwritten
before the mark has a dead wire and must not be stitched across the boundary
(§7.1).

**Identical skeletons.** The symbolic axis is a function of public structure
alone — which inputs were declared private or blind, and which operations touched
them — so it is bit-for-bit identical on both parties, even though the held axis
is local. Capture leans on this directly: at each segment mark both sides record
the same symbolic registers, symbolic globals, and publicly-overwritten bytes,
and so derive the same boundary layout, tape lengths, and challenge counts
without exchanging them. Taint is the concrete reason the determinism principle
(§2) extends to the witness-dependent structure of the proof, not just to control
flow and addresses.

**Symbolic addresses and the three contracts.** Everything above assumes the
*address* of an access is public, so both parties name the same cell. A symbolic
address breaks that: the prover holds the target, the verifier cannot, and the
shared skeleton may depend only on public structure. Taint answers *who knows* a
value; how the proof *names* that value once its address is symbolic is a
proof-system concern that belongs to vm-zk alone (custody, §8). vm-core stays free
of custody and discharges its part in three contracts, none of which name brackets
or routability:

- **Symmetric smear.** A symbolic-address *store* — any width — smears the
  *symbolic* axis over the full reachable range identically on both parties: the
  prover, who holds the address, marks the range Private; the verifier marks it
  Blind. The held axis is where the two sides legitimately differ; the symbolic
  axis never does, so the shared-skeleton invariant above — the symbolic axis is a
  function of public structure alone — is preserved. A symbolic-address *load*
  leaves memory taint untouched: it changes no byte's value, and taint answers
  value knowledge — only the load's destination register is symbolic, as any
  symbolic op's result is. Today the reachable range is the whole linear memory; a
  declared or range-asserted region is the future refinement *(target)*.
- **Observable overwrites.** A concrete store that overwrites symbolic bytes emits
  its directive *before* clearing their taint, so every transition of committed
  state is observable to the embedder. For vm-zk that directive is the re-anchoring
  store (§8.2) whose logged tuple retires the cell's standing multiset entry. A
  store whose target bytes are already fully public stays silent and free — the
  common path is untouched.
- **Observable stores under an open bracket** *(target)*. vm-core exposes an
  embedder-controlled **store-observability mode**: while enabled, every store
  emits its directive regardless of taint, so a value change in bytes that are
  bracketed but publicly tainted still surfaces to the embedder. vm-zk enables the
  mode at the first symbolic-address access; both parties see that access's
  directive at the same step, so the mode flips identically on both sides and the
  skeleton stays shared. The mechanism lives in vm-core, the policy in the
  embedder — vm-core still knows nothing of brackets or routability.

**The tainted-read guard.** A party may read linear memory through the `Vm`
interface only where it is untainted. Reading a symbolic range fails by
construction: the prover would be leaking witness it has not opened, and the
verifier simply does not hold the cleartext. This guard is what makes "commit,
then optionally open" sound — committed-but-unrevealed memory is unreadable on
both sides until a reveal lifts it (§9.2).

### 5.5 The deterministic skeleton (capture)

Before proving, both parties *capture* a chunk: they step the thread in lockstep,
recording an identical directive trace, its gate cost (§10), its segment marks
(§7), the host-call actions, and (prover only) the plaintext snapshots needed at
boundaries. The verifier reaches the same skeleton because control flow and
public structure are shared; the prover announces only the unavoidable public
outcomes (a trap and its location, the payloads of reveals) ahead of capture so
the verifier can mirror them. Capture is where public work is discharged for free
and where the protocol's shared shape — every tape length and challenge count —
is fixed. It reads no MAC tape and no challenge, which is what lets it run ahead
of proving for pipelining (§12).

---

## 6. Streaming: chunks

### 6.1 Bounded buffers and the correlation budget

A long execution is proven as a sequence of chunks, each capped at a fixed gate
budget (`DEFAULT_CHUNK_CAP`). A chunk is the pipeline's lookahead window (§12); the
cap bounds its gate-plus-advice op cost so that per-chunk proving buffers —
capture, the plan, the mask and MAC tapes, the boundary tape — are bounded
independent of trace length. The cap is chosen so the
op cost fits comfortably within the net correlations one Ferret extension round
delivers (one round produces a large batch, minus the seed it reserves for the
next).

The cap bounds op cost, not the full allocation: a chunk's sVOLE demand is its op
cost *plus* the input/memory-commit prefix, the boundary-commitment bits, and the
VOPE tail. This total typically still fits one extension round, but input- or
write-heavy chunks can push it over and require a second. The cost accounting
(§10) is responsible for keeping the worst-case overhead within budget.

### 6.2 The per-chunk protocol

Each chunk runs the two-pass protocol (§4.3) over a single sVOLE allocation:

1. **Capture** the chunk (both, lockstep, §5.5) — challenge-independent, so it
   can run ahead of the previous chunk's proof (§12).
2. **Announce** trap/reveal outcomes (prover → verifier) — one one-way flight,
   sent for every chunk, including fully-public ones.
3. **Plan** the segmentation and memory layout from the shared skeleton (both).
4. **Allocate** one sVOLE tape sized for the whole chunk — gates, advice,
   boundary commitments, input commitments, the memory argument's commitments,
   and the VOPE masks. Because the size is a deterministic function of the shared
   skeleton and is challenge-independent, this allocation can be issued for the
   *next* chunk while the current one is still proving, taking the Ferret flush
   off the steady-state critical path (§12).
5. **Commit pass** (parallel segments, §4.3): produce the adjustment bits.
6. **Commit** the adjustments and squeeze the challenges soundness requires.
7. **Accumulate pass** (parallel segments, §4.3): fold the proof.
8. **Prove**: send the masked proof, the output, and any revealed cleartext.

State that outlives the chunk — the authenticated registers, globals, and the
working memory set — is carried forward to seed the next chunk.

The wire messages, in protocol order, are: `ChunkOutcome` (trap announcement plus
disclosed reveal payloads), `Commitment` (the adjustment bit-vector), the 32-byte
challenge, and `ProofMessage` (output, revealed cleartext, and the `Proof` —
assertion hash, masked `u`/`v`, coefficient vector). The verifier checks the
commitment length against its own deterministically-derived tape size and rejects
inconsistent trap fields.

### 6.3 The fully-public fast path

A chunk that reaches no authenticated work before the program ends or traps — no
committed bits, no gates, only public computation — has nothing to prove. Both
parties detect this from the shared skeleton and skip the
allocate/commit/challenge/prove exchange in lockstep. Such a chunk incurs only
the one-way `ChunkOutcome` announce (step 2) that keeps the two parties in
lockstep — no proving round-trip.

---

## 7. Parallelism: segments

### 7.1 Boundary deltas

A chunk's trace is split at cost marks into segments proven by independent
workers. The challenge is continuity: worker `j+1` must see the state worker `j`
produced. The design avoids passing wires between workers — which would serialise
them — by committing, at each segment boundary, a **delta**: only the registers,
globals, and memory bytes that segment *wrote* (consulting live taint, §5.4, so a
byte overwritten in the clear is not stitched, and skipping unroutable bytes (§8.3),
whose values live only in the memory argument's committed tuples and need no worker
stitching — re-anchored, routable bytes are stitched normally), plus tombstones for
registers it reclaimed. Worker `j` asserts its final wires equal the delta; every later worker
re-materialises the delta directly off the shared tape. An item written in
segment `i` reaches all later workers as the *same* materialised wire, so
cross-segment equality holds by construction.

Two cost dimensions follow, and they are different. The committed boundary *tape*
is linear in the chunk's writes — only a segment's own writes are committed, never
quadratically across segments. But the *seeding work* is not: each worker
re-materialises every boundary before its own segment, so cumulative per-worker
replay is O(segments²) in the boundary widths. That quadratic seeding term, not
the tape, is the binding constraint that fixes the segment count at a small target
(`TARGET_SEGMENTS`) rather than scaling it with core count or trace length — large
enough to oversubscribe typical cores for work-stealing balance, small enough that
seeding stays well below the linear replay work. This boundary cost — the seeding
plus the committed delta tape — is why the segment count tracks the core count
rather than growing past it (§12): once segments fill the cores the proof is
work-bound, and further splitting only adds boundary cost. A linear-work
prefix-scan that materialises each cumulative seed once removes the `O(segments²)`
term, leaving the committed boundary tape (fresh sVOLE per written item) as the
residual cost that argues for keeping the count near the core count.

### 7.2 The two parallel passes

Both the commit and accumulate passes run as parallel maps over segments followed
by a combine. The commit pass evaluates each segment in the clear over pointer-bit
wires and writes its adjustment slice; those slices can stream to the verifier as
workers finish, overlapping commit compute with serialisation and transmission
(the verifier still needs the whole vector before it samples — §12). The
accumulate pass seeds each worker's challenge stream at the segment's gate offset,
folds its slice into local `(u, v)` and an assertion hash, and the combine sums
them. The last segment additionally binds the chunk's output, trap, and reveals.

### 7.3 Challenge-stream seeking

Because the accumulate fold is linear in challenge weights, each worker draws from
the *same* challenge seed positioned at its own gate offset. Disjoint workers
therefore consume disjoint challenge ranges and their partials sum to the
single-pass result. The memory argument's polynomial constraints fold the same way
over a second, domain-separated challenge stream offset by the constraint count,
and merge across workers.

### 7.4 Single-segment fallback

Planning degrades to one sequential segment whenever a chunk has no marks, is
unbounded, or its layout scan hits a trace it cannot mirror. This is a determinism
safety net, not just an optimisation: both parties fall back identically and
surface the same error at the same protocol point, rather than disagreeing on
layout and deadlocking. It is the local realisation of the §2 determinism
invariant.

---

## 8. Memory consistency

The guest's linear memory is small — low single-digit megabytes — and that bounds
the whole design. The entire memory is held as committed wires for the life of a
call, and the memory-checking argument is checked once over the call.

Consistency turns on per-byte state vm-zk maintains alongside taint: **custody** —
how the proof *names* a byte's value, where taint answers *who knows* it. Custody
is two independent facts. A byte is **bracketed** when it sits inside the memory
argument's open bracket: it was bridged into the multiset — the dangling committed
write-half of §8.5 — at the first symbolic-address access that could reach it, and
from then until the audit every write to it logs a committed tuple. Bracket
membership is monotone within a call; only the audit closes it. A byte is
**routable** when its current value is nameable by both parties as a specific
committed wire or as public cleartext; reads of routable bytes are free and
unlogged even while bracketed. Neither fact is vm-core state: both are derived
state of vm-zk's access log (the `MemoryLog`), pure functions of the shared access
stream, so the parties compute them identically without exchanging them. Routable
reads and unbracketed writes cost only the wires they route (§8.2); bracketed
writes and unroutable reads are proven by one offline memory-checking argument
over committed `(address, value, time)` tuples that hides the address (§8.3). The
committed product covers only that logged traffic — bracketed writes, the reads
that restore routability, and the bridge and audit halves that open and close each
cell's bracket (§8.5).

### 8.1 Full-memory residence

The entire linear memory is carried as committed bit-wires across every chunk of a
call, densely and without eviction. A memory byte is eight IT-MAC wires (§4.1), so
a low-megabyte guest fits in a few hundred megabytes per party. Every cell's wire
is available to every chunk; the working set is never re-committed or re-derived at
a boundary, and the end-of-call audit reads memory straight out of residence.

An unroutable byte (§8.3) is the one exception: its carried wire is dead — the RAM
argument, not the wire, names its current value. Residence still earns its keep for
that byte, holding the prover's cleartext, the last nameable write that opened its
bracket (§8.5), and the value the end-of-call audit reads out. A bracketed byte
that is routable keeps a live wire like any other.

### 8.2 Public addresses and re-anchoring

Any public-addressed access re-anchors the bytes it touches: it restores their
routability, pinning each byte's value to nameable wires at a public address and a
public clock. Re-anchoring never ends bracket membership — only the audit does
(§8.5). A store re-anchors with its operand's wires. A read re-anchors an
unroutable byte by fresh-committing its value as advice — those advice wires ride
the tape, nameable by both parties from then on — while a read of an
already-routable byte simply reads off its existing wire, free and unlogged even
while bracketed (§8.3). A write to a bracketed byte logs its tuple whether or not
the byte was routable: its value wires stay nameable, so it commits no fresh value
advice, but its retire read-half carries a committed timestamp and one comparator —
the cell's last-write time may be private, because an intervening private load's
rewrite may have bumped it. These one-time costs, and the cancellation that erases
publicly matched pairs, are accounted in §8.3.

A routable byte is exactly the free wire routing of the base design. A
public-address load references the committed wire its store produced; across chunks
this rides the carried memory (§8.1), and within a chunk the boundary deltas of §7.1
propagate a segment's writes to the segments after it. Both parties name the same
cell and materialise the same wire, so equality holds by construction — routable
reads and unbracketed writes need no argument and cost only the wires they route.

### 8.3 Private addresses and the multiset argument

A symbolic-address access — one whose address is committed — cannot be routed: the
verifier cannot name the target cell (§5.4). Its two effects on the reachable range
are asymmetric. A symbolic-address **store** makes the range bracketed *and*
unroutable: a value may have changed at an unknown cell, so no byte in the range
may be read off a wire until it is re-anchored (§8.2). A symbolic-address **load**
makes the range bracketed but *leaves it routable*: it changes no value, so public
reads of the range stay free — only writes pay while the bracket is open. This
asymmetry is what keeps private lookups into a public or committed table cheap:
the lookup brackets the table, and subsequent public reads of it remain free wire
routing. Today the reachable range is the whole linear memory; a declared or
range-asserted region is the future refinement *(target)*.

While a byte is bracketed, every write to it logs a committed tuple — including a
concrete store over concrete bytes, whose value changes no taint (surfaced by the
store-observability contract, §5.4): a later private read's tuple must match the
cell's true current value in the multiset, so no value change inside a bracketed
range may go unlogged. Reads divide by routability: a read of an unroutable byte
logs — it is the re-anchoring read of §8.2, fresh-committing its value as advice —
while a read of a routable byte is unlogged even while bracketed, since an unlogged
read leaves the multiset balanced and its value is correct by construction.

Consistency over the logged accesses is one offline memory-checking argument over
committed `(address, value, time)` tuples, following the SpeakUp construction, which
reveals nothing about which cell each tuple names. Every logged access contributes a
**read-half** tuple `(addr, old value, last-write time)` and a **write-half** tuple
`(addr, new value, current clock)` to a single whole-call multiset, indexed by a
call-global clock that is monotone across chunks. Two facts establish consistency:

- **Permutation** — the read multiset equals the write multiset, proven by a grand
  product `∏(eᵢ + r)` over `GF(2^64)` under a verifier challenge `r`, where each
  tuple compresses to one field element `eᵢ`.
- **Timestamp** — every read sources a value written strictly in the past, proven
  by a borrow-chain comparator asserting `t < c` (the terminal audit read is the
  one exception, §8.5). The comparator runs in the
  boolean path and folds into the per-chunk accumulate pass; the clock is public,
  so its per-bit gate selection is free and it costs one multiplication per
  timestamp bit.

**Public accesses cancel.** The multiset notionally spans every access; both
parties drop **publicly matched pairs** before committing to the product: a
read-half cancels the preceding write-half on the same cell whenever no
symbolic-address access lies between them. The matching is public structure — both
sides compute it identically from the shared access log — and a matched pair is
tuple-identical: same public address, the same value wires (that is what wire
routing means), and the same public timestamp, so its two `(e + r)` factors are
equal and dropping both preserves multiset equality exactly. The two custody facts
are the per-byte residue of this predicate, one per intervening access kind: an
intervening symbolic-address *store* may have changed the value — that is
unroutability — while an intervening symbolic-address *load* may have privately
bumped the last-write time, which is why a bracketed write's surviving retire
read-half commits its timestamp (§8.2) even though its value wires still match. A
cell with purely public-address history cancels entirely, including its
initialisation and audit (§8.5), and contributes nothing to the committed product.
A re-anchoring read pays the full one-time cost — fresh-committed old value and
timestamp, one comparator, two product factors — after which its write-half is a
new public anchor and the cell's subsequent history telescopes away for free until
the next symbolic-address access or the audit.

Because each logged access is a self-contained committed tuple, unroutable memory
needs no boundary stitching between segment workers (§7.1) — its consistency is
established globally by the permutation, not handed worker to worker. Routable
bytes, bracketed or not, stitch through the ordinary deltas.

### 8.4 The grand product

The grand product is accelerated: its factors are tiled into groups of a fixed
width (`CHUNK`), each verified as one degree-`d` polynomial constraint (§4.5)
relating consecutive boundary partial products `accₖ`, so only those boundary
products are committed. Entries bridge from the tuples' already-committed bits
(§4.6) — no fresh `GF(2^64)` tape — and a tuple's low and high halves combine into
one entry as `lo + γ·hi` under a second challenge `γ`.

Because `pack` is independent of `γ`, each committed access is reduced at the moment
it occurs to its two packed `Auth64` limbs (`lo` and `hi`), and only those limbs are
retained for the call — on the order of tens of bytes per surviving tuple, in place
of the per-bit tuple wires. The product is assembled from the held limbs once `(γ, r)`
is known, and its tiles fold in parallel: each `accₖ` is committed witness, so no tile
depends on another's intermediate state.

*(open)* Tuple granularity versus re-anchor granularity is unresolved. Custody is
per byte — a `store8` re-anchors one byte of a word — but the packing above treats
an access tuple `(address, value, time)` as two `Auth64` limbs, so a word access
spanning routable and unroutable bytes, or bracketed and unbracketed ones, does not
map onto one tuple cleanly. The two candidates are byte-granular tuples (clean
semantics, roughly `8×` the product factors per wide access) and access-granular
tuples plus a rule that a partial-width access to a mixed range first demotes the
whole range to the stricter state. This is settled together with the zk-ram
`CHUNK`/limb design before the access-log format freezes.

### 8.5 Initialisation and audit

Initialisation and audit are not separate protocol events; they are the two multiset
halves that fail to cancel. Initialisation is a write-half seeding each cell's
starting value, bridged from the held memory wires (§8.1); audit is a read-half for
every cell at the close of the call, read out of the held memory. For a cell with
purely public-address history the two match each other through the cell's telescoped
public chain and drop, so the cell leaves no residue (§8.3). What survives is exactly
the bracket: the last nameable write before the first symbolic-address access that
could reach the cell — initialisation itself, if none intervened — is the dangling
write-half that opens it, and the audit read-half after the cell's last
symbolic-address access closes it, ending the cell's bracket membership (§8). An
audit read commits its timestamp but
omits the comparator: it fires at the terminal clock, where the past-clock ordering is
already implied by the balanced multiset. Because the memory is small and resident,
initialisation and audit range over the held memory and are bounded by the guest
memory size.

### 8.6 The whole-call check

The argument is checked once over the entire call, indexed by the call-global clock
(§8.3). Tuple bits are committed chunk by chunk as execution proceeds; each chunk
packs the access tuples that survive cancellation into limbs (§8.4) and folds their
comparators and gates under that chunk's challenge `χ`. After the final chunk commits,
the verifier samples the permutation challenges `(γ, r)`; the prover then commits the
partial products `accₖ`, the product and the audit fold over the held limbs and
memory, and the verifier runs one polynomial check (§4.5).

Sampling `(γ, r)` after every tuple bit is committed binds the witness before the
challenge is known. The commitment therefore arrives in two phases — the
challenge-independent tuple, gate, and comparator commitments during the call, then
the `accₖ` that depend on `(γ, r)` at the close — and the closing fold spans as many
correlation rounds as the `accₖ` and audit require. Cancellation adds no protocol
message and no phase: it only shrinks which entries produce limbs and factors, leaving
the two-phase structure unchanged.

---

## 9. Input/output

### 9.1 Inputs

Private inputs are staged into linear memory and committed as authenticated bytes;
public inputs are written in the clear and carry no commitment. On the prover a
private input is staged with Private visibility (§5.4); the verifier reserves the
same region as Blind and commits blanks, so the committed wire is identical on
both sides while only the prover holds the cleartext. A third class, **Blind**
inputs, inverts the roles: the bits are supplied by the *verifier*, so the prover
commits placeholder zeros and the verifier's adjustment carries the real pattern.
This role symmetry — a `Private` write on one side is the `Blind` reservation on
the other — is the structural reason the two parties are peers running the same
program. The commitment is fixed at stage time, so later in-place mutation of a
staged region does not change what was committed.

A **standalone commitment** path flushes pending inputs without running a
function: it allocates only the input-commit tape plus a VOPE tail, runs the
commit/challenge/proof exchange with a gate-free accumulate (folding only any
reveal assertions), and installs the committed input and memory wires for a later
call to consume.

### 9.2 Reveals (prove-then-open)

A program may open a memory range to the verifier. The range's cleartext is first
proven correct — each byte's MAC is asserted against the disclosed value, binding
it into the assertion hash (§4.2) — and only then disclosed and its range's memory
taint dropped to Public, the step that lifts the tainted-read guard (§5.4) and
makes the range readable. The prover's cleartext already lives in its memory from
when it was stored; the verifier materialises it on disclosure.

Reveals use an open/wait split. Opening returns a public **handle** — an id from a
lockstep counter both parties advance identically — and a later wait binds the
now-public value into a register. A shared `RevealState` carries the id counter and
a payload map; the prover announces disclosed payloads with the chunk outcome and
the verifier merges them before its own capture, so the two resolve every reveal in
lockstep, and a handle opened in one chunk can be waited on in a later one. A
pending reveal forces an otherwise-public chunk to prove.

### 9.3 Outputs and traps

A function's return value, when symbolic, is bound by asserting its authenticated
wires against the value the prover discloses; a concrete return is already public
and reconstructed locally by the verifier.

A trap is a public outcome. The trap taxonomy splits in two. **Structural traps**
(unreachable, a public out-of-bounds access, an explicit exit) are reached
identically from the shared skeleton and need no constraint — both sides simply
agree the chunk traps there. **Operand-bound traps** (a symbolic divide-by-zero or
a symbolic multiply overflow) depend on hidden operands, so a dedicated pass proves
the trap condition: that the divisor's bits are all zero, or that the operands are
the overflow witness. Trapping ops are tape-free and terminate the chunk. The
security property is that the verifier returns the reason it *proved*, not the one
the prover *announced*, and rejects on any disagreement.

### 9.4 Precompiles

Cryptographic primitives that would be expensive as raw gates are provided as host
calls. A precompile has a fully public fast path — when all inputs are public, both
parties compute the result in the clear with no gates — and an authenticated path
that emits the primitive's circuit once over mixed public/symbolic input wires and
writes the result back to memory. The gate budget of an authenticated precompile is
accounted in capture so segment and challenge layout line up with the circuit
replay emits.

---

## 10. Cost model and accounting

Both parties budget each chunk from the same published per-op gate costs, selecting
the constant-specialised variant whenever an operand is public — exactly the choice
replay makes when emitting the circuit. Costs distinguish **gates**
(multiplications, which advance the challenge stream) from **advice** (committed
witness such as division quotients, which consume tape but draw no challenge), so
segment challenge offsets are computed from gate counts alone. The memory argument
contributes its read-timestamp, comparator, partial-product, and
initialisation/audit costs to the same budget. The public `chunk_cap` knob is measured in accumulated gate-plus-advice
cost, not instruction count, so the doc, the configuration, and capture all use one
unit. Because the budget is a deterministic function of the shared skeleton, both
sides compute identical chunk boundaries, segment marks, tape sizes, and challenge
counts without communicating them.

The asymptotic "linear in symbolic gates" hides a large constant: the inner loop is
a `GF(2^128)` carry-less multiply plus a challenge draw per symbolic gate (and, in
the memory argument, a `GF(2^64)` multiply per grand-product factor). The field
multiply is the throughput bottleneck, and its backend — hardware `pclmulqdq`,
SIMD, or software, selected per target — is the dominant order-of-magnitude factor.
A north-star implementation targets vectorised carry-less multiply with batched,
deferred reduction.

---

## 11. Performance summary

| Axis | Mechanism | Section |
|------|-----------|---------|
| Bounded proving memory | streamed chunks, gate-capped buffers | §6 |
| Resident state | full guest memory held as committed wires (low MB) + the surviving (bracketed) access-tuple limbs | §6, §8 |
| Commitment minimisation | linear IT-MACs; boundary deltas; accelerated grand product; composite poly check | §4.1, §7.1, §8.3 |
| Parallelism | per-segment commit/accumulate, no inter-worker wires; segment count bounded by boundary cost | §7, §12 |
| Rounds | one challenge round/chunk; one closing `(γ, r)` flight for the whole-call memory check (§8.6) | §6.2, §8.6, §12 |
| Fields | `GF(2)` semantics + `GF(2^64)` permutation, one proof | §4.6 |
| Free work | public values, control flow, addresses, precompile inputs; fully-public chunks | §5, §6.3 |

Two qualifications matter. **Memory** holds the entire guest linear memory as
committed wires for the life of a call — a few hundred megabytes at low-megabyte
guest sizes (§8.1) — plus the packed limbs of every access that survives
cancellation: the call's bracketed writes, unroutable reads, and bridge/audit
halves (§8.3–8.5). Resident memory therefore scales with the guest memory size and
the call's bracketed traffic rather than with its total access count or staying
constant per chunk. The
per-chunk gate, mask, and proof buffers remain bounded by the chunk cap. **Proof
size** is constant per chunk — an assertion hash, two field elements, and a fixed
`≤ d_max` coefficient vector when private-memory constraints are present — where
"constant" means independent of trace length and gate count, since `d_max` is a fixed
protocol constant.

The asymptotic shape is: prover/verifier time linear in symbolic gates plus logged
(bracketed) memory accesses; per-chunk proving buffers constant, resident
memory proportional to guest memory size plus the call's bracketed traffic;
proof size constant per chunk; and wall-clock bounded below by
`total real-time work / cores` once segments fill the cores (§12). The recoverable headroom above that floor is round-trip idle, hidden
by pipelining and sized by the window-to-RTT ratio; segment count tracks the core
count and is not itself a performance lever.

---

## 12. Orchestration and the performance floor

The proof is a pipeline whose floor is the work it cannot avoid. Per-gate work
comes in two kinds (§4.2–4.3): the **commit pass**, a data-dependent walk of the
circuit that emits the adjustment bits before the challenge, and the **accumulate
fold**, an associative `(u, v)` reduction of one `GF(2^128)` multiply per gate.
Correlation generation is a third, real-time stream — it cannot be precomputed.

Given enough segments to fill the cores, all three parallelise to work-bound, and
the wall-clock floor is

```
(commit + accumulate + correlation) · gates / cores
```

— the total real-time work over the cores, with nothing below it. The commit
pass's data-dependence would bound a *single* segment by its depth, but that only
surfaces if the cores outnumber the segments; keeping the segment count at or above
the core count makes the commit pass work-bound like the fold, and gadget depths
are shallow enough that this is never the binding constraint.

**Segmentation exists only to reach that floor.** The segment count must be at
least the core count, with light oversubscription so work-stealing smooths
stragglers (a large precompile op is the worst case). Beyond that, finer
segmentation buys no compute and only adds boundary cost — fresh sVOLE per delta
plus, without the linear-work prefix-scan seed, `O(S²)` re-materialisation (§7.1) —
so the segment count *tracks the core count* rather than being a tuning lever. A
fixed target undersized for a high-core host is the one real mistake to avoid; past
`S ≈ cores`, boundary cost only argues for fewer.

**The recoverable headroom above the floor is round-trip idle.** Run sequentially,
the cores stall during each window's commitment→challenge round-trip and its
correlation flush — roughly two round-trips per window — while compute waits on the
network. Pipelining hides these behind adjacent windows' compute; that is the
entire win, and its size is the round-trip fraction: negligible when a window's
compute exceeds the RTT (co-located parties, large windows), dominant when it does
not (a wide-area link, or windows small enough that ~2·RTT rivals their compute).
The lever that sets this fraction is the **window size** — one challenge per
window, decoupled from the Ferret correlation batch — not the segment size.

**The three streams pipeline across a memory window.** Correlation generation, the
commit pass, and the accumulate fold for adjacent windows run concurrently, the
cores split so their wall-clocks equalise. The enabling overlaps:

- *Capture runs ahead* (§5.5) — challenge- and crypto-free, it produces the next
  window's skeleton while the current one proves. It is not free: the trace it
  produces is stored, so capture is a memory-bounded stage that back-pressures
  against the lookahead budget rather than running arbitrarily ahead.
- *Correlation is double-buffered* — the next window's sVOLE is extended while the
  current window's is consumed, taking the Ferret flush off the steady-state
  critical path.
- *Challenges are squeezed under compute* — the verifier samples a window's
  challenge (sampled, not derived, §2) the moment it holds that window's
  commitment, while still folding the previous window, so the round-trip hides
  under compute. Adjustment bits stream to the verifier per segment as commit
  workers finish (§7.2).

A **chunk is this memory window**: the lookahead held resident — the captured
trace, the in-flight correlation, and the live tape. It is sized to keep the
pipeline full — deep enough to cover the challenge round-trip and stage imbalance
— and capped by the RAM that residence consumes. One challenge spans the whole
window, so round-trips are one per window, not one per correlation batch.

**The cross-window serial chain is the artifact to remove.** Carrying
authenticated state forward from one window's accumulate to seed the next (§6.2)
makes the accumulate passes a serial chain. Committing each window boundary
instead — deterministic from capture, the same delta mechanism as segments — makes
the windows independent, so work-stealing spans them. This matters precisely
because a single window's accumulate fold may not saturate the cores: when it does
not, ready folds from neighbouring windows must be available to steal, and the
serial chain forbids exactly that.

The immovable barriers are local: for each region the challenge must follow its
commitment and its accumulate must follow its challenge, and the verifier must
hold a window's whole adjustment vector before it samples. These are one barrier
per window, hidden behind the window's own compute once the window is large
enough. This pipeline is the target orchestration; the current implementation runs
windows sequentially and does not yet overlap them (§13).

---

## 13. Implementation status

Implemented: the execution model and memory tainting (§5), streaming chunks (§6),
segment parallelism with boundary deltas (§7), the boolean proof system (§4.1–4.4),
the input/reveal/output/trap/precompile paths (§9), and cost accounting (§10).

Designed with building blocks in place, not yet wired into the pipeline: the
composite polynomial check and the `GF(2^64)` extension (§4.5–4.6), and the memory
argument's emitters — compression, comparator, and accelerated grand product
(§8.3–8.4). The remaining integration is the memory argument's pipeline wiring:
full-memory residence (§8.1), reducing each surviving access to packed limbs at
capture (§8.4), the initialisation and audit passes (§8.5), the whole-call `(γ, r)`
check and its closing two-phase commitment (§8.6), the `GF(2^64)` sidecar context,
and the polynomial check at verification.

Not yet built: private and committed addresses (§8.3), including per-byte bracket
and routability tracking over the `MemoryLog`, the public-access cancellation
predicate (whose tuple granularity is still open, §8.4), and the
store-observability contract (§5.4); cross-chunk pipelining / correlation
double-buffering (§12); and the linear-work prefix-scan for boundary seeding
(§7.1).

---

## References

- **QuickSilver** — the VOLE-based, designated-verifier multiplication check the
  proof system is built on (§4.2).
- **SpeakUp** — the offline memory-checking construction the RAM argument follows:
  permutation grand product plus timestamp comparator (§8.3), and the accelerated
  partial-product grand product (§8.4).
- **Ferret** — the LPN-based correlation generator sized against the chunk cap
  (§6.1).
