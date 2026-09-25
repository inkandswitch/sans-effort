//! What one poll of a routine reported.

use core::task::Poll;

/// Where the routine stopped after a poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// At least one request awaits a reply; reply and resume.
    Awaiting,
    /// The routine returned. Further replies return nothing.
    Complete,
    /// Pending with no request outstanding: waiting on something inside the
    /// process — a channel, a lock — that another routine may complete.
    /// Nothing wakes it; [`resume`](super::Driver::resume) polls it again.
    Idle,
}

impl Status {
    /// `Ready` is `Complete`; `Pending` with requests outstanding is
    /// `Awaiting`; `Pending` with none is `Idle`.
    pub(super) const fn classify(poll: Poll<()>, outstanding: usize) -> Self {
        match poll {
            Poll::Ready(()) => Status::Complete,
            Poll::Pending if outstanding > 0 => Status::Awaiting,
            Poll::Pending => Status::Idle,
        }
    }
}
