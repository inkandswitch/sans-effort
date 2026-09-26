//! What [`Sleep`](super::Sleep) records under a reifying context.

use crate::ask::Ask;
use core::time::Duration;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Wake after a duration. Awaits `unit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sleep(pub Duration);

impl Sleep {
    /// The duration in whole milliseconds, as it crosses the wire; saturates
    /// at `u64::MAX`.
    #[must_use]
    pub fn millis(&self) -> u64 {
        u64::try_from(self.0.as_millis()).unwrap_or(u64::MAX)
    }
}

impl Ask for Sleep {
    type Reply = ();
}

/// Whole milliseconds, `u64`. Sub-millisecond precision does not cross.
impl Encode for Sleep {
    fn encode(&self, w: &mut Writer) {
        w.u64(self.millis());
    }
}

impl Decode for Sleep {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u64().map(|ms| Sleep(Duration::from_millis(ms)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_milliseconds_round_trip() {
        bolero::check!().with_type::<u64>().for_each(|ms| {
            let sleep = Sleep(Duration::from_millis(*ms));
            assert_eq!(Sleep::from_bytes(&sleep.to_bytes()), Ok(sleep));
        });
    }

    #[test]
    fn durations_past_u64_millis_saturate() {
        assert_eq!(Sleep(Duration::MAX).millis(), u64::MAX);
    }
}
