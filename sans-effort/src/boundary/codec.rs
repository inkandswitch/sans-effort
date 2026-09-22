//! The byte codec: values written as bytes, and read back.
//!
//! Little-endian throughout. `str` and `bytes` are a `u32` length then the
//! payload. [`Encode`] and [`Decode`] are the two directions: a host's view of
//! an effect is encoded on the way out; a message, a spawn argument, or any
//! reply richer than the four reply kinds is decoded on the way in.
//!
//! Every method of [`Reader`] returns a [`DecodeError`] on short or malformed
//! input rather than panicking, and [`Decode::from_bytes`] rejects trailing
//! bytes, as the ABI does for reply records.
//!
//! A message type implements both, field by field:
//!
//! ```
//! use sans_effort::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};
//!
//! #[derive(Debug, PartialEq)]
//! enum Counter {
//!     Incr(u64),
//!     Reset,
//! }
//!
//! impl Encode for Counter {
//!     fn encode(&self, w: &mut Writer) {
//!         match self {
//!             Counter::Incr(n) => {
//!                 w.u8(0);
//!                 n.encode(w);
//!             }
//!             Counter::Reset => w.u8(1),
//!         }
//!     }
//! }
//!
//! impl Decode for Counter {
//!     fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
//!         match r.u8()? {
//!             0 => Ok(Counter::Incr(u64::decode(r)?)),
//!             1 => Ok(Counter::Reset),
//!             tag => Err(DecodeError::UnknownTag { tag }),
//!         }
//!     }
//! }
//!
//! let bytes = Counter::Incr(3).to_bytes();
//! assert_eq!(Counter::from_bytes(&bytes), Ok(Counter::Incr(3)));
//! assert_eq!(Counter::from_bytes(&[7]), Err(DecodeError::UnknownTag { tag: 7 }));
//! ```

use alloc::{string::String, vec::Vec};

/// A value that can be written as bytes.
///
/// For a [`HostEffect::View`](super::host_effect::HostEffect::View), that is
/// its tag, its fields, and, for an awaiting effect, its request id.
pub trait Encode {
    /// Append this value to `w`.
    fn encode(&self, w: &mut Writer);

    /// This value alone, as bytes.
    #[must_use]
    fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.encode(&mut w);
        w.finish()
    }
}

/// A value that can be read back from bytes: the counterpart to [`Encode`].
///
/// For every type that implements both, `T::from_bytes(&x.to_bytes())` is
/// `Ok(x)`.
pub trait Decode: Sized {
    /// Read one value from the front of `r`, leaving the rest.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`] if the bytes are short or malformed.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError>;

    /// Read exactly one value from `bytes`.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`] if the bytes are short or malformed, or
    /// [`DecodeError::TrailingBytes`] if any are left over.
    fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let value = Self::decode(&mut r)?;
        r.finish()?;
        Ok(value)
    }
}

/// Appends little-endian records to a buffer.
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    /// An empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// One byte.
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    /// Four bytes, little-endian.
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    /// A `u32` length, then the bytes.
    ///
    /// # Panics
    ///
    /// If `b` is 4 GiB or longer: the wire carries a `u32` length, and a
    /// payload that size is a bug, not a case.
    #[expect(
        clippy::expect_used,
        reason = "a >4 GiB record is a programming error, not a runtime case"
    )]
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(u32::try_from(b.len()).expect("payload shorter than 4 GiB"));
        self.0.extend_from_slice(b);
    }

    /// A `u32` length, then UTF-8.
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// The buffer.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// Reads little-endian records from the front of a slice.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Read from the start of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let (head, tail) = self
            .bytes
            .split_at_checked(n)
            .ok_or(DecodeError::Truncated {
                needed: n,
                available: self.bytes.len(),
            })?;
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// One byte.
    ///
    /// # Errors
    ///
    /// [`DecodeError::Truncated`] if no bytes are left.
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        self.array().map(|[b]| b)
    }

    /// Four bytes, little-endian.
    ///
    /// # Errors
    ///
    /// [`DecodeError::Truncated`] if fewer than four bytes are left.
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        self.array().map(u32::from_le_bytes)
    }

    /// Eight bytes, little-endian.
    ///
    /// # Errors
    ///
    /// [`DecodeError::Truncated`] if fewer than eight bytes are left.
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        self.array().map(u64::from_le_bytes)
    }

    /// A `u32` length, then that many bytes.
    ///
    /// # Errors
    ///
    /// [`DecodeError::Truncated`] if the length or the payload is short;
    /// [`DecodeError::TooLong`] if the length does not fit this target's
    /// `usize`.
    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()?;
        let n = usize::try_from(len).map_err(|_| DecodeError::TooLong { len })?;
        self.take(n)
    }

    /// A `u32` length, then UTF-8.
    ///
    /// # Errors
    ///
    /// As [`bytes`](Self::bytes), or [`DecodeError::InvalidUtf8`].
    pub fn str(&mut self) -> Result<String, DecodeError> {
        let bytes = self.bytes()?;
        core::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|e| DecodeError::InvalidUtf8 {
                valid_up_to: e.valid_up_to(),
            })
    }

    /// `true` when every byte has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// End of input: every byte must have been consumed.
    ///
    /// # Errors
    ///
    /// [`DecodeError::TrailingBytes`] if any are left.
    pub const fn finish(self) -> Result<(), DecodeError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes {
                remaining: self.bytes.len(),
            })
        }
    }
}

impl Encode for u8 {
    fn encode(&self, w: &mut Writer) {
        w.u8(*self);
    }
}

impl Decode for u8 {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u8()
    }
}

impl Encode for u32 {
    fn encode(&self, w: &mut Writer) {
        w.u32(*self);
    }
}

impl Decode for u32 {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u32()
    }
}

impl Encode for u64 {
    fn encode(&self, w: &mut Writer) {
        w.u64(*self);
    }
}

impl Decode for u64 {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u64()
    }
}

impl Encode for () {
    fn encode(&self, _: &mut Writer) {}
}

impl Decode for () {
    fn decode(_: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(())
    }
}

impl Encode for str {
    fn encode(&self, w: &mut Writer) {
        w.str(self);
    }
}

impl Encode for String {
    fn encode(&self, w: &mut Writer) {
        w.str(self);
    }
}

impl Decode for String {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.str()
    }
}

impl Encode for [u8] {
    fn encode(&self, w: &mut Writer) {
        w.bytes(self);
    }
}

impl Encode for Vec<u8> {
    fn encode(&self, w: &mut Writer) {
        w.bytes(self);
    }
}

impl Decode for Vec<u8> {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.bytes().map(<[u8]>::to_vec)
    }
}

/// Why bytes could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DecodeError {
    /// The input ended early.
    #[error("input ended early: needed {needed} bytes, {available} left")]
    Truncated {
        /// Bytes the next field needed.
        needed: usize,
        /// Bytes that were left.
        available: usize,
    },

    /// A length prefix does not fit this target's `usize`.
    #[error("length {len} does not fit this target")]
    TooLong {
        /// The length read.
        len: u32,
    },

    /// A `str` field is not UTF-8.
    #[error("invalid UTF-8 after {valid_up_to} valid bytes")]
    InvalidUtf8 {
        /// Bytes of the field that were valid UTF-8.
        valid_up_to: usize,
    },

    /// A tag byte names no known variant.
    #[error("unknown tag {tag}")]
    UnknownTag {
        /// The tag read.
        tag: u8,
    },

    /// Bytes were left after a complete value.
    #[error("{remaining} trailing bytes after a complete value")]
    TrailingBytes {
        /// Bytes left over.
        remaining: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Debug;

    fn round_trips<T: Encode + Decode + PartialEq + Debug>(value: &T) {
        assert_eq!(T::from_bytes(&value.to_bytes()).as_ref(), Ok(value));
    }

    /// Every proper prefix of a value's encoding is `Truncated`, never a
    /// panic and never a different value.
    fn prefixes_are_truncated<T: Encode + Decode + Debug>(value: &T) {
        let mut prefix = value.to_bytes();
        while prefix.pop().is_some() {
            assert!(
                matches!(T::from_bytes(&prefix), Err(DecodeError::Truncated { .. })),
                "a {}-byte prefix of {value:?}",
                prefix.len()
            );
        }
    }

    #[test]
    fn primitives_round_trip() {
        bolero::check!()
            .with_type::<(u8, u32, u64, String, Vec<u8>)>()
            .for_each(|(a, b, c, d, e)| {
                round_trips(a);
                round_trips(b);
                round_trips(c);
                round_trips(d);
                round_trips(e);
                round_trips(&());
            });
    }

    #[test]
    fn writer_and_reader_agree_field_by_field() {
        bolero::check!()
            .with_type::<(u8, u32, u64, Vec<u8>, String)>()
            .for_each(|(a, b, c, d, e)| {
                let mut w = Writer::new();
                w.u8(*a);
                w.u32(*b);
                w.u64(*c);
                w.bytes(d);
                w.str(e);

                let bytes = w.finish();
                let mut r = Reader::new(&bytes);
                assert_eq!(r.u8(), Ok(*a));
                assert_eq!(r.u32(), Ok(*b));
                assert_eq!(r.u64(), Ok(*c));
                assert_eq!(r.bytes(), Ok(d.as_slice()));
                assert_eq!(r.str().as_deref(), Ok(e.as_str()));
                assert_eq!(r.finish(), Ok(()));
            });
    }

    #[test]
    fn truncated_input_is_an_error_never_a_panic() {
        bolero::check!()
            .with_type::<(u32, u64, String, Vec<u8>)>()
            .for_each(|(a, b, c, d)| {
                prefixes_are_truncated(a);
                prefixes_are_truncated(b);
                prefixes_are_truncated(c);
                prefixes_are_truncated(d);
            });
    }

    #[test]
    fn trailing_bytes_are_counted() {
        bolero::check!()
            .with_type::<(String, Vec<u8>)>()
            .for_each(|(value, extra)| {
                let mut bytes = value.to_bytes();
                bytes.extend_from_slice(extra);
                let got = String::from_bytes(&bytes);
                if extra.is_empty() {
                    assert_eq!(got.as_ref(), Ok(value));
                } else {
                    assert_eq!(
                        got,
                        Err(DecodeError::TrailingBytes {
                            remaining: extra.len()
                        })
                    );
                }
            });
    }

    #[test]
    fn arbitrary_bytes_decode_canonically_or_not_at_all() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            // Fixed-width values decode exactly when the length is right.
            assert_eq!(u8::from_bytes(bytes).is_ok(), bytes.len() == 1);
            assert_eq!(u32::from_bytes(bytes).is_ok(), bytes.len() == 4);
            assert_eq!(u64::from_bytes(bytes).is_ok(), bytes.len() == 8);
            // Whatever decodes re-encodes to exactly the input: one encoding
            // per value, so a transcript compares byte for byte.
            if let Ok(s) = String::from_bytes(bytes) {
                assert_eq!(&s.to_bytes(), bytes);
            }
            if let Ok(v) = Vec::<u8>::from_bytes(bytes) {
                assert_eq!(&v.to_bytes(), bytes);
            }
        });
    }

    #[test]
    fn invalid_utf8_says_where() {
        let mut w = Writer::new();
        w.bytes(&[b'o', b'k', 0xff]);
        assert_eq!(
            String::from_bytes(&w.finish()),
            Err(DecodeError::InvalidUtf8 { valid_up_to: 2 })
        );
    }

    #[test]
    fn unit_is_zero_bytes() {
        assert!(().to_bytes().is_empty());
        assert_eq!(
            <()>::from_bytes(&[0]),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        );
    }
}
