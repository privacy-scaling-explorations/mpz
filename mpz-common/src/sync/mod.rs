//! Synchronization primitives.

mod mutex;

pub use mutex::{Lock, Mutex, MutexBroker, MutexError};
