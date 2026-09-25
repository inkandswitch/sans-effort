//! A routine that needs less than the greeter.

use crate::PAUSE;
use alloc::format;
use core::ops::ControlFlow;
use sans_effort::step::Step;
use sans_effort::{console::WriteLine, time::Sleep};

/// Ticks `n` times, pausing between, and never reads a line, looks anything
/// up, or counts.
///
/// The bound _is_ the permission. A `Ticker` cannot ask for input whatever
/// its context could offer, and a context that offers only `Sleep + WriteLine`
/// can run a `Ticker` but not a [`Greeter`](crate::greeter::Greeter) —
/// checked where it is built, and, under a reifying context with a
/// host-chosen vocabulary, visible on the wire as tags that can never appear.
/// See `greeter_boundary::Quiet`.
#[derive(Debug)]
pub struct Ticker<C: Sleep + WriteLine> {
    ctx: C,
    remaining: u32,
}

impl<C: Sleep + WriteLine> Ticker<C> {
    /// Tick `n` times through `ctx`.
    pub const fn new(ctx: C, n: u32) -> Self {
        Self { ctx, remaining: n }
    }
}

impl<C: Sleep + WriteLine> Step for Ticker<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.remaining == 0 {
            return ControlFlow::Break(());
        }

        self.remaining -= 1;
        self.ctx
            .write_line(format!("tick ({} left)", self.remaining));
        self.ctx.sleep(PAUSE).await;
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{Call, transcript};

    #[test]
    fn ticks_n_times_and_never_reads() {
        bolero::check!().with_type::<u8>().for_each(|n| {
            let calls = transcript(|ctx| Ticker::new(ctx, u32::from(*n)), &[]);
            assert_eq!(calls.len(), 2 * usize::from(*n));
            assert!(
                calls
                    .iter()
                    .all(|c| matches!(c, Call::Write(_) | Call::Sleep(_)))
            );
        });
    }
}
