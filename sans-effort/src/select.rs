//! Await two futures; resolve with whichever finishes first.
//!
//! [`join`](crate::join::join) waits for both. A routine often needs the
//! other shape — a message or a timeout, work or a signal to stop:
//!
//! ```text
//!   select(receive(), sleep(timeout))
//!     poll ─▶ Receive·4 recorded, Pending
//!     poll ─▶ Sleep·5 recorded,   Pending      one batch: [Receive·4, Sleep·5]
//!     … host replies 4 … poll ─▶ a = Ready(msg)  ─▶ Ready(Left(msg))
//!                                 Sleep·5 is dropped: its id is closed
//! ```
//!
//! The loser is dropped when `select` returns. Behind a driver its request is
//! reported in the yield's [`closed`](crate::driver::Yield::closed), so a
//! host can stop that work; a late reply to it is discarded. Dropping it
//! means "no longer needed", not "did not happen": the losing effect may
//! already be under way.
//!
//! Both are converted into futures when `select` starts, `a` first, so two
//! [`Ask`](crate::driver::ask::Ask)s are recorded in argument order. `a` is
//! polled first on every poll, so if both are ready at once, `a` wins. Put
//! the branch that should win ties first.

use core::{
    future::{Future, IntoFuture, poll_fn},
    pin::pin,
    task::Poll,
};

/// Poll `a` and `b` together; resolve with the first to finish, dropping the
/// other.
///
/// ```
/// use sans_effort::{select::{Either, select}, testing::run_now};
///
/// let first = run_now(select(async { 1 }, core::future::pending::<&str>()));
/// assert_eq!(first, Either::Left(1));
/// ```
pub async fn select<A: IntoFuture, B: IntoFuture>(a: A, b: B) -> Either<A::Output, B::Output> {
    let mut a = pin!(a.into_future());
    let mut b = pin!(b.into_future());

    poll_fn(|cx| {
        if let Poll::Ready(v) = a.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(v));
        }

        if let Poll::Ready(v) = b.as_mut().poll(cx) {
            return Poll::Ready(Either::Right(v));
        }

        Poll::Pending
    })
    .await
}

/// Which of two futures finished first, and its output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Either<A, B> {
    /// The first future finished.
    Left(A),
    /// The second future finished.
    Right(B),
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::{
        driver::{Driver, outbox::Outbox},
        reply::handle::ReplyHandle,
        step::Step,
    };
    use alloc::{string::String, vec::Vec};
    use core::ops::ControlFlow;

    enum Effect {
        Ask(ReplyHandle<String>),
        Won(Either<String, String>),
    }

    /// Races two requests and reports which answer arrived first.
    struct Race(Outbox<Effect>);

    impl Step for Race {
        async fn step(&mut self) -> ControlFlow<()> {
            let won = select(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
            self.0.tell(Effect::Won(won));
            ControlFlow::Break(())
        }
    }

    fn race() -> (Driver<Effect>, ReplyHandle<String>, ReplyHandle<String>) {
        let mut driver = Driver::new(|outbox| Race(outbox).run());
        let step = driver.resume();
        assert!(step.closed().is_empty(), "both requests are outstanding");
        let [a, b]: [ReplyHandle<String>; 2] = step
            .into_iter()
            .filter_map(|e| match e {
                Effect::Ask(r) => Some(r),
                Effect::Won(_) => None,
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("both branches recorded a request in the first batch");
        (driver, a, b)
    }

    fn won(effects: &[Effect]) -> Option<&Either<String, String>> {
        effects.iter().find_map(|e| match e {
            Effect::Won(w) => Some(w),
            Effect::Ask(_) => None,
        })
    }

    #[test]
    fn whichever_is_answered_first_wins_and_the_loser_closes() {
        for answer_left in [true, false] {
            let (mut driver, a, b) = race();

            let (winner, late) = if answer_left { (a, b) } else { (b, a) };
            let loser = late.id();
            let step = driver.reply(winner, String::from("x"));

            let want = if answer_left {
                Either::Left(String::from("x"))
            } else {
                Either::Right(String::from("x"))
            };
            assert_eq!(won(step.effects()), Some(&want));
            assert_eq!(step.closed(), [loser], "the loser is closed exactly once");
            assert!(driver.is_finished());

            assert!(
                driver.reply(late, String::from("late")).is_empty(),
                "a late reply to the loser is discarded"
            );
        }
    }

    #[test]
    fn ties_go_to_the_first() {
        let tie = crate::testing::run_now(select(async { 'a' }, async { 'b' }));
        assert_eq!(tie, Either::Left('a'));
    }
}
