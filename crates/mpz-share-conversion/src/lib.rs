use mpz_ole::OLEError;
use thiserror::Error;

mod m2a;
mod role;

pub use m2a::M2A;
pub use role::{Evaluate, Provide, Role};

#[derive(Debug, Error)]
pub enum ShareConversionError {
    #[error(transparent)]
    OLE(#[from] OLEError),
}

