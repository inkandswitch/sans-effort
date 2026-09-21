//! Which effect this is.

use wasm_bindgen::prelude::*;

/// Which effect this is.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// How many greetings so far. Reply with `replyNumber` (or `replyU64`
    /// for a value above 2^53).
    Count,
    /// The greeting for `name`. Reply with `replyStr`.
    Lookup,
    /// A line of input. Reply with `replyStr`.
    ReadLine,
    /// Wait `millis`. Reply with `replyUnit`.
    Sleep,
    /// Show `text`. No reply.
    Write,
}
