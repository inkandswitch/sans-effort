//! The shape of a routine: one turn at a time.

use core::{future::Future, ops::ControlFlow};

/// A routine that advances one turn at a time.
///
/// `turn` is one iteration of the routine's loop; returning `Break` ends it.
/// `run` repeats `turn` until it breaks. Neither method says anything about
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
/// # Turns and hosts
///
/// A host sees suspensions, not turns. [`run`](Self::run) loops over `turn`
/// inside the one boxed future, so `Break` is reported as
/// [`Status::Complete`](crate::driver::Status::Complete) and `Continue` is not
/// reported at all. If a host must observe a turn boundary, tell it an effect
/// at the top of `turn`.
pub trait Run {
    /// One turn. `Break(())` means the routine is finished.
    fn turn(&mut self) -> impl Future<Output = ControlFlow<()>>;

    /// Turns until one breaks.
    fn run(mut self) -> impl Future<Output = ()>
    where
        Self: Sized,
    {
        async move { while self.turn().await.is_continue() {} }
    }
}
