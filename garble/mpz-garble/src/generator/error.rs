use mpz_garble_core::ValueError;

use crate::value::ValueRef;

/// Errors that can occur while performing the role of a generator
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum GeneratorError {
    #[error(transparent)]
    CoreError(#[from] mpz_garble_core::GeneratorError),
    // TODO: Fix the size of this error
    #[error(transparent)]
    OTError(Box<mpz_ot::OTError>),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ValueError(#[from] ValueError),
    #[error("missing encoding for value")]
    MissingEncoding(ValueRef),
    #[error(transparent)]
    EncodingRegistryError(#[from] crate::memory::EncodingMemoryError),
}

impl From<mpz_ot::OTError> for GeneratorError {
    fn from(err: mpz_ot::OTError) -> Self {
        Self::OTError(Box::new(err))
    }
}
