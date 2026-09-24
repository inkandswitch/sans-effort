//! The shape of a routine: one step at a time.

use core::{future::Future, ops::ControlFlow};

/// A routine that advances one step at a time.
///
/// `step` is one iteration of the routine's loop; returning `Break` ends it.
/// `run` repeats `step` until it breaks. Neither method says anything about
/// `Send`: whether the returned future can cross threads is a property of the
/// concrete implementor, and Rust infers it at the call site — see
/// [`Driver::new`](crate::driver::Driver::new), which is where the check
/// happens.
///
/// Spelled as `fn … -> impl Future` rather than `async fn` only to silence
/// `async_fn_in_trait`; the two mean exactly the same thing. The lint's point
/// is that a _caller_ cannot add a `Send` bound later; this crate never needs
/// to, because nothing in it is generic over a routine and also spawns it.
/// Implementors may write `async fn`.
///
/// # Steps and Hosts
///
/// A host sees suspensions, not steps. [`run`](Self::run) loops over `step`
/// inside the one boxed future, so `Break` is reported as
/// [`Status::Complete`](crate::driver::status::Status::Complete) and `Continue` is not
/// reported at all. If a host must observe a step boundary, tell it an effect
/// at the top of `step`. (The host's own unit of advance is finer — to the
/// next wait — and is called `resume` on its side to keep the two apart.)
pub trait Run {
    /// One iteration of the loop. `Break(())` means the routine is finished.
    fn step(&mut self) -> impl Future<Output = ControlFlow<()>>;

    /// Steps until one breaks.
    fn run(mut self) -> impl Future<Output = ()>
    where
        Self: Sized,
    {
        async move { while self.step().await.is_continue() {} }
    }
}
