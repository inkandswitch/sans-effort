//! Where the routine stopped.

use crate::contract;
use sans_effort_core::driver::status::Status as DriveStatus;

/// What one `resume` or `reply` reports. [`code`](Self::code) is its wire form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// At least one request awaits a reply.
    Awaiting,
    /// The routine returned.
    Complete,
    /// Pending with no request outstanding: waiting on something inside the
    /// process, such as a channel another routine sends on. `resume` it once
    /// that may have changed.
    Idle,
}

impl Status {
    /// The wire code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Status::Awaiting => contract::AWAITING,
            Status::Complete => contract::COMPLETE,
            Status::Idle => contract::IDLE,
        }
    }
}

impl From<DriveStatus> for Status {
    fn from(status: DriveStatus) -> Self {
        match status {
            DriveStatus::Awaiting => Status::Awaiting,
            DriveStatus::Complete => Status::Complete,
            DriveStatus::Idle => Status::Idle,
        }
    }
}
