//! Await two futures at once.
//!
//! The reason request ids exist on the wire. Two lazy requests polled in the
//! same turn record two effects in one batch, and the host may reply to them
//! in either order; `join` is the smallest routine shape that does so.
//! `no_std` has no `join!`, so this is the whole of it: poll both, keep what
//! finished, return when neither is left.
//!
//! ```text
//!   join(lookup(name), count())
//!     poll ─▶ Lookup·1 recorded, Pending
//!     poll ─▶ Count·2 recorded,  Pending        one batch: [Lookup·1, Count·2]
//!     … host replies 2 … poll ─▶ b = Some(n), a still Pending
//!     … host replies 1 … poll ─▶ a = Some(g)  ─▶ Ready((g, n))
//! ```

use core::{
    future::{Future, poll_fn},
    pin::pin,
    task::Poll,
};

/// Poll `a` and `b` together; resolve when both have.
pub async fn join<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let mut a = pin!(a);
    let mut b = pin!(b);
    let mut ra = None;
    let mut rb = None;

    poll_fn(|cx| {
        if ra.is_none()
            && let Poll::Ready(v) = a.as_mut().poll(cx)
        {
            ra = Some(v);
        }

        if rb.is_none()
            && let Poll::Ready(v) = b.as_mut().poll(cx)
        {
            rb = Some(v);
        }

        match (ra.take(), rb.take()) {
            (Some(x), Some(y)) => Poll::Ready((x, y)),
            (x, y) => {
                ra = x;
                rb = y;
                Poll::Pending
            }
        }
    })
    .await
}
