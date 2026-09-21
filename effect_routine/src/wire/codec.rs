//! The byte codec: how a host's view of an effect is written, and how a
//! reply record is read.
//!
//! Little-endian throughout. `str` and `bytes` are a `u32` length then the
//! payload; every method of [`Reader`] returns `None` on short or malformed
//! input rather than panicking.

use alloc::{string::String, vec::Vec};

/// How a [`HostEffect::View`](super::host_effect::HostEffect::View) is written
/// for the byte layer: tag, fields, and, for an awaiting effect, its id.
pub trait Encode {
    /// Append this value's records to `w`.
    fn encode(&self, w: &mut Writer);
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

/// Reads little-endian records; every method returns `None` on short or
/// malformed input.
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

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.bytes.split_at_checked(n)?;
        self.bytes = tail;
        Some(head)
    }

    /// One byte.
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).and_then(|b| b.first().copied())
    }

    /// Four bytes, little-endian.
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
    }

    /// A `u32` length, then that many bytes.
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u32()?).ok()?;
        self.take(len)
    }

    /// A `u32` length, then UTF-8.
    pub fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }

    /// `true` when every byte has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_round_trips() {
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
                assert_eq!(r.u8(), Some(*a));
                assert_eq!(r.u32(), Some(*b));
                assert_eq!(r.u64(), Some(*c));
                assert_eq!(r.bytes(), Some(d.as_slice()));
                assert_eq!(r.str().as_deref(), Some(e.as_str()));
                assert!(r.is_empty());
            });
    }

    #[test]
    fn reader_rejects_short_input() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut r = Reader::new(bytes);
            // Whatever the prefix decodes to, a truncated tail must be `None`,
            // never a panic.
            let _ = r.u8();
            let _ = r.u32();
            let _ = r.u64();
            let _ = r.bytes();
            drop(r.str());
        });
    }
}
