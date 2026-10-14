//! The shape of a routine: one step at a time.

use core::{future::Future, ops::ControlFlow};

/// A routine that advances one step at a time.
///
/// `step` is one iteration of the routine's loop; returning `Break` ends it.
/// `run` repeats `step` until it breaks. Implementors write `async fn step`.
/// Whether the future is `Send` is left to the implementor: Rust infers it,
/// and [`Driver::new`](crate::driver::Driver::new) is where it is checked.
///
/// # Steps and Hosts
///
/// A host sees suspensions, not steps. [`run`](Self::run) loops over `step`
/// inside the one boxed future, so `Break` is reported as
/// [`Status::Complete`](crate::driver::status::Status::Complete) and
/// `Continue` is not reported at all. If a host must observe a step boundary,
/// tell it an effect at the top of `step`.
pub trait Step {
    /// One iteration of the loop. `Break(())` means the routine is finished.
    // Spelled `fn … -> impl Future`, not `async fn`, only for
    // `async_fn_in_trait`: that lint guards callers who would add a `Send`
    // bound, and nothing here is generic over a routine and spawns it.
    fn step(&mut self) -> impl Future<Output = ControlFlow<()>>;

    /// Steps until one breaks.
    fn run(mut self) -> impl Future<Output = ()>
    where
        Self: Sized,
    {
        async move { while self.step().await.is_continue() {} }
    }
}
