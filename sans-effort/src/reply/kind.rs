//! Which entry of the reply menu a value is.

/// The discriminant of a [`Value`](super::value::Value), without its payload.
///
/// A host that replies by request id rather than through a typed handle
/// names the kind it is sending; the host layer compares that against what
/// the request asked for, and reports both when they differ. Inside the
/// mechanism nothing needs it: a typed handle is the proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// [`Value::Bytes`](super::value::Value::Bytes).
    Bytes,
    /// [`Value::Str`](super::value::Value::Str).
    Str,
    /// [`Value::U64`](super::value::Value::U64).
    U64,
    /// [`Value::Unit`](super::value::Value::Unit).
    Unit,
}
