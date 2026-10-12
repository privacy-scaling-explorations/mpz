use derive_builder::Builder;

/// An empirically chosen default value which provides the best performance.
const DEFAULT_BATCH_SIZE: usize = 200_000;

/// Prover configuration.
#[derive(Debug, Clone, Builder)]
pub struct ProverConfig {
    /// Target number of AND gates per consistency-check batch.
    ///
    /// The actual size of a batch may exceed this value slightly because
    /// individual calls are never split across batches. If the last call in
    /// a batch would exceed the target size, the entire call is included in
    /// that batch.
    #[builder(default = "DEFAULT_BATCH_SIZE")]
    batch_size: usize,
    /// Generate gate OTs for each execution batch instead of all queued calls.
    ///
    /// This bounds gate-OT storage by the ready batch (individual circuits are
    /// not split), at the cost of interleaving OT communication with execution.
    /// Both parties must use the same setting. During execution this VM must be
    /// the only active consumer of its OT source: any shared handles must be
    /// dropped or parked, since OT flush may require all consumers to
    /// participate. Eager preprocessing remains the default so independent
    /// shared consumers may finish at different times without an additional
    /// collective flush.
    #[builder(default)]
    batch_gate_ots: bool,
}

impl Default for ProverConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            batch_gate_ots: false,
        }
    }
}

impl ProverConfig {
    /// Creates a new builder for ProverConfig.
    pub fn builder() -> ProverConfigBuilder {
        ProverConfigBuilder::default()
    }

    /// Returns target number of AND gates per consistency-check batch.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns whether gate OTs are generated per execution batch.
    pub fn batch_gate_ots(&self) -> bool {
        self.batch_gate_ots
    }
}

/// Verifier configuration.
#[derive(Debug, Clone, Builder)]
pub struct VerifierConfig {
    /// Target number of AND gates per consistency-check batch.
    ///
    /// The actual size of a batch may exceed this value slightly because
    /// individual calls are never split across batches. If the last call in
    /// a batch would exceed the target size, the entire call is included in
    /// that batch.
    #[builder(default = "DEFAULT_BATCH_SIZE")]
    batch_size: usize,
    /// Generate gate OTs for each execution batch instead of all queued calls.
    ///
    /// This bounds gate-OT storage by the ready batch (individual circuits are
    /// not split), at the cost of interleaving OT communication with execution.
    /// Both parties must use the same setting. During execution this VM must be
    /// the only active consumer of its OT source: any shared handles must be
    /// dropped or parked, since OT flush may require all consumers to
    /// participate. Eager preprocessing remains the default so independent
    /// shared consumers may finish at different times without an additional
    /// collective flush.
    #[builder(default)]
    batch_gate_ots: bool,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            batch_gate_ots: false,
        }
    }
}

impl VerifierConfig {
    /// Creates a new builder for VerifierConfig.
    pub fn builder() -> VerifierConfigBuilder {
        VerifierConfigBuilder::default()
    }

    /// Returns target number of AND gates per consistency-check batch.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns whether gate OTs are generated per execution batch.
    pub fn batch_gate_ots(&self) -> bool {
        self.batch_gate_ots
    }
}
