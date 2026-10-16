use derive_builder::Builder;

/// An empirically chosen default value which provides the best performance.
const DEFAULT_BATCH_SIZE: usize = 200_000;

/// Maximum number of AND-gate OTs materialized in one chunk.
///
/// `call_raw` pre-registers gate OTs up to this while calls are submitted, and
/// `execute` tops the window up during execution. The consistency check still
/// fires every `batch_size` gates *within* a chunk, so this only bounds the OT
/// materialization window, not the check cadence.
const DEFAULT_CHUNK_CAP: usize = 15_000_000;

/// Prover configuration.
#[derive(Debug, Clone, Builder)]
#[builder(build_fn(validate = "Self::validate"))]
pub struct ProverConfig {
    /// Target number of AND gates per consistency-check batch.
    ///
    /// The actual size of a batch may exceed this value slightly because
    /// individual calls are never split across batches. If the last call in
    /// a batch would exceed the target size, the entire call is included in
    /// that batch.
    #[builder(default = "DEFAULT_BATCH_SIZE")]
    batch_size: usize,
    /// Maximum number of AND-gate OTs materialized in one chunk.
    ///
    /// Bounds the OT buffer high-water independently of `batch_size` (which
    /// bounds the consistency-check cadence). Must be at least `batch_size` and
    /// match the verifier's value.
    #[builder(default = "DEFAULT_CHUNK_CAP")]
    chunk_cap: usize,
}

impl Default for ProverConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            chunk_cap: DEFAULT_CHUNK_CAP,
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

    /// Returns the maximum number of AND-gate OTs materialized in one chunk.
    pub fn chunk_cap(&self) -> usize {
        self.chunk_cap
    }
}

impl ProverConfigBuilder {
    fn validate(&self) -> Result<(), String> {
        if self.batch_size == Some(0) {
            return Err("batch_size must be greater than zero".to_string());
        }
        if self.chunk_cap == Some(0) {
            return Err("chunk_cap must be greater than zero".to_string());
        }

        // A chunk must be able to hold at least a full consistency-check batch.
        let batch_size = self.batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
        let chunk_cap = self.chunk_cap.unwrap_or(DEFAULT_CHUNK_CAP);
        if chunk_cap < batch_size {
            return Err("chunk_cap must be at least batch_size".to_string());
        }

        Ok(())
    }
}

/// Verifier configuration.
#[derive(Debug, Clone, Builder)]
#[builder(build_fn(validate = "Self::validate"))]
pub struct VerifierConfig {
    /// Target number of AND gates per consistency-check batch.
    ///
    /// The actual size of a batch may exceed this value slightly because
    /// individual calls are never split across batches. If the last call in
    /// a batch would exceed the target size, the entire call is included in
    /// that batch.
    #[builder(default = "DEFAULT_BATCH_SIZE")]
    batch_size: usize,
    /// Maximum number of AND-gate OTs materialized in one chunk.
    ///
    /// Bounds the OT buffer high-water independently of `batch_size` (which
    /// bounds the consistency-check cadence). Must be at least `batch_size` and
    /// match the prover's value.
    #[builder(default = "DEFAULT_CHUNK_CAP")]
    chunk_cap: usize,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            chunk_cap: DEFAULT_CHUNK_CAP,
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

    /// Returns the maximum number of AND-gate OTs materialized in one chunk.
    pub fn chunk_cap(&self) -> usize {
        self.chunk_cap
    }
}

impl VerifierConfigBuilder {
    fn validate(&self) -> Result<(), String> {
        if self.batch_size == Some(0) {
            return Err("batch_size must be greater than zero".to_string());
        }
        if self.chunk_cap == Some(0) {
            return Err("chunk_cap must be greater than zero".to_string());
        }

        // A chunk must be able to hold at least a full consistency-check batch.
        let batch_size = self.batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
        let chunk_cap = self.chunk_cap.unwrap_or(DEFAULT_CHUNK_CAP);
        if chunk_cap < batch_size {
            return Err("chunk_cap must be at least batch_size".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_batch_size() {
        assert!(ProverConfig::builder().batch_size(0).build().is_err());
        assert!(VerifierConfig::builder().batch_size(0).build().is_err());
    }

    #[test]
    fn rejects_zero_chunk_cap() {
        assert!(ProverConfig::builder().chunk_cap(0).build().is_err());
        assert!(VerifierConfig::builder().chunk_cap(0).build().is_err());
    }

    #[test]
    fn rejects_chunk_cap_below_batch_size() {
        assert!(
            ProverConfig::builder()
                .batch_size(200)
                .chunk_cap(199)
                .build()
                .is_err()
        );
        assert!(
            VerifierConfig::builder()
                .batch_size(200)
                .chunk_cap(199)
                .build()
                .is_err()
        );
    }

    #[test]
    fn accepts_chunk_cap_equal_to_batch_size() {
        assert!(
            ProverConfig::builder()
                .batch_size(200)
                .chunk_cap(200)
                .build()
                .is_ok()
        );
        assert!(
            VerifierConfig::builder()
                .batch_size(200)
                .chunk_cap(200)
                .build()
                .is_ok()
        );
    }

    #[test]
    fn compares_against_defaulted_field() {
        // `chunk_cap` is left unset, so it defaults to `DEFAULT_CHUNK_CAP`;
        // raising `batch_size` above it must still be rejected.
        assert!(
            ProverConfig::builder()
                .batch_size(DEFAULT_CHUNK_CAP + 1)
                .build()
                .is_err()
        );
        assert!(
            VerifierConfig::builder()
                .batch_size(DEFAULT_CHUNK_CAP + 1)
                .build()
                .is_err()
        );
    }

    #[test]
    fn defaults_are_valid() {
        let prover = ProverConfig::builder().build().unwrap();
        assert!(prover.chunk_cap() >= prover.batch_size());

        let verifier = VerifierConfig::builder().build().unwrap();
        assert!(verifier.chunk_cap() >= verifier.batch_size());

        let prover = ProverConfig::default();
        assert!(prover.chunk_cap() >= prover.batch_size());

        let verifier = VerifierConfig::default();
        assert!(verifier.chunk_cap() >= verifier.batch_size());
    }
}
