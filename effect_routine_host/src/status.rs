//! Where the routine stopped.

use crate::code;
use effect_routine::driver::status::Status as DriveStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What one `start` or `reply` reports. [`code`](Self::code) is its wire form.
pub enum Status {
    /// At least one request awaits a reply.
    Awaiting,
    /// The routine returned.
    Complete,
    /// Pending with no request outstanding; will never progress.
    Stalled,
}

impl Status {
    /// The wire code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Status::Awaiting => code::AWAITING,
            Status::Complete => code::COMPLETE,
            Status::Stalled => code::STALLED,
        }
    }
}

impl From<DriveStatus> for Status {
    fn from(status: DriveStatus) -> Self {
        match status {
            DriveStatus::Awaiting => Status::Awaiting,
            DriveStatus::Complete => Status::Complete,
            DriveStatus::Stalled => Status::Stalled,
        }
    }
}
