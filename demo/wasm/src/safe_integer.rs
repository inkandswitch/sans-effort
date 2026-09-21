//! An integer a JS `number` holds exactly.
//!
//! JS has one numeric type, and it represents integers exactly only up to
//! 2^53 − 1. Everything that crosses this skin as a `number` — request ids,
//! millisecond durations, `u64` replies — goes through [`SafeInteger`], whose
//! constructor is the one place the range is checked. Once you hold one, both
//! conversions are total: no `as` cast, no assertion, no rounding anywhere
//! else in the crate.

use core::fmt;
use wasm_bindgen::JsError;

/// A `u64` in `0..=2^53 − 1`: exactly representable as an `f64`, and so as a
/// JS `number`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SafeInteger(u64);

impl SafeInteger {
    /// The largest value: `Number.MAX_SAFE_INTEGER`.
    pub const MAX: Self = Self((1 << 53) - 1);

    /// The value as a `u64`.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why a value is not a safe integer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotSafe {
    /// A `u64` above 2^53 − 1.
    TooLarge(u64),
    /// An `f64` that is not a finite, non-negative integer within range.
    NotAnInteger(f64),
}

impl fmt::Display for NotSafe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotSafe::TooLarge(n) => write!(f, "{n} exceeds Number.MAX_SAFE_INTEGER (2^53 - 1)"),
            NotSafe::NotAnInteger(x) => {
                write!(
                    f,
                    "{x} is not a non-negative integer within Number.MAX_SAFE_INTEGER"
                )
            }
        }
    }
}

impl From<NotSafe> for JsError {
    fn from(e: NotSafe) -> Self {
        JsError::new(&e.to_string())
    }
}

impl TryFrom<u64> for SafeInteger {
    type Error = NotSafe;

    fn try_from(n: u64) -> Result<Self, NotSafe> {
        if n <= Self::MAX.0 {
            Ok(Self(n))
        } else {
            Err(NotSafe::TooLarge(n))
        }
    }
}

impl TryFrom<f64> for SafeInteger {
    type Error = NotSafe;

    fn try_from(x: f64) -> Result<Self, NotSafe> {
        let max: f64 = Self::MAX.into();

        if x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x <= max {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked just above: finite, integral, and within 0..=2^53 - 1"
            )]
            Ok(Self(x as u64))
        } else {
            Err(NotSafe::NotAnInteger(x))
        }
    }
}

/// Total: every safe integer is an exact `f64`.
impl From<SafeInteger> for f64 {
    fn from(n: SafeInteger) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "the constructor guarantees n <= 2^53 - 1, which f64 represents exactly"
        )]
        let exact = n.0 as f64;
        exact
    }
}

impl From<SafeInteger> for u64 {
    fn from(n: SafeInteger) -> u64 {
        n.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn round_trips_within_range() {
        for n in [0, 1, 42, SafeInteger::MAX.get()] {
            let safe = SafeInteger::try_from(n).expect("in range");
            let x: f64 = safe.into();
            assert_eq!(SafeInteger::try_from(x).expect("exact"), safe);
        }
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(
            SafeInteger::try_from(SafeInteger::MAX.get() + 1),
            Err(NotSafe::TooLarge(SafeInteger::MAX.get() + 1))
        );
        for x in [-1.0, 0.5, f64::NAN, f64::INFINITY, 2f64.powi(53)] {
            assert!(SafeInteger::try_from(x).is_err(), "{x}");
        }
    }
}
