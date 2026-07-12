use derive_builder::Builder;

use crate::softspoken::{CSP, SUPPORTED_K};

const DEFAULT_BATCH_SIZE: usize = 1 << 18;
const DEFAULT_K: usize = 4;

fn validate_k(k: usize) -> Result<(), String> {
    if !SUPPORTED_K.contains(&k) {
        return Err(format!(
            "unsupported SoftSpoken k: {k}, must be one of {SUPPORTED_K:?}"
        ));
    }
    debug_assert_eq!(CSP % k, 0, "k must divide CSP");
    Ok(())
}

macro_rules! softspoken_config {
    (
        $(#[$meta:meta])*
        $name:ident, $builder:ident, $builder_err:ident, k_doc = $k_doc:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Builder)]
        #[builder(build_fn(validate = "Self::validate"))]
        pub struct $name {
            /// Batch size for each flush.
            #[builder(default = "DEFAULT_BATCH_SIZE")]
            batch_size: usize,
            #[doc = $k_doc]
            #[builder(default = "DEFAULT_K")]
            k: usize,
        }

        impl $builder {
            fn validate(&self) -> Result<(), $builder_err> {
                if let Some(k) = self.k {
                    validate_k(k).map_err($builder_err::ValidationError)?;
                }
                Ok(())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    batch_size: DEFAULT_BATCH_SIZE,
                    k: DEFAULT_K,
                }
            }
        }

        impl $name {
            #[doc = concat!("Creates a new builder for ", stringify!($name), ".")]
            pub fn builder() -> $builder {
                $builder::default()
            }

            /// Returns the batch size for each flush.
            pub fn batch_size(&self) -> usize {
                self.batch_size
            }

            /// Returns the `k` parameter.
            pub fn k(&self) -> usize {
                self.k
            }

            /// Returns the number of blocks, `CSP / k`.
            pub fn n_blocks(&self) -> usize {
                CSP / self.k
            }

            /// Returns the number of leaves per block, `2^k`.
            pub fn leaves(&self) -> usize {
                1 << self.k
            }
        }
    };
}

softspoken_config! {
    /// Configuration for a SoftSpoken [`Sender`](super::Sender).
    SenderConfig, SenderConfigBuilder, SenderConfigBuilderError,
    k_doc = "SoftSpoken compute/communication tradeoff parameter.\n\nEach \
        extended OT costs `CSP / k` bits of communication and `2^k / k` times \
        the PRG work of IKNP. Must be one of `{2, 4, 8}`."
}

softspoken_config! {
    /// Configuration for a SoftSpoken [`Receiver`](super::Receiver).
    ReceiverConfig, ReceiverConfigBuilder, ReceiverConfigBuilderError,
    k_doc = "SoftSpoken compute/communication tradeoff parameter. See \
        [`SenderConfig::k`]. Must be one of `{2, 4, 8}`."
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_K, ReceiverConfig, SenderConfig};

    #[test]
    fn builder_validates_k_for_both_configs() {
        for k in [0usize, 1, 3, 5, 16] {
            assert!(
                SenderConfig::builder().k(k).build().is_err(),
                "sender k={k}"
            );
            assert!(
                ReceiverConfig::builder().k(k).build().is_err(),
                "receiver k={k}"
            );
        }
        for k in [2usize, 4, 8] {
            assert!(SenderConfig::builder().k(k).build().is_ok(), "sender k={k}");
            assert!(
                ReceiverConfig::builder().k(k).build().is_ok(),
                "receiver k={k}"
            );
        }
        assert_eq!(SenderConfig::default().k(), DEFAULT_K);
        assert_eq!(ReceiverConfig::default().k(), DEFAULT_K);
    }
}
