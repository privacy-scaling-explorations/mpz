pub trait Role: sealed::Sealed {}

#[derive(Debug, Copy, Clone)]
pub struct Provide;
impl Role for Provide {}

#[derive(Debug, Copy, Clone)]
pub struct Evaluate;
impl Role for Evaluate {}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Provide {}
    impl Sealed for super::Evaluate {}
}
