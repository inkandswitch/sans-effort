//! `Send` is declared on the capability traits' futures, so a context that
//! could not produce `Send` futures cannot implement a capability at all.
//!
//! That is what lets a routine generic over its context prove the children
//! it spawns are `Send`. A context over a value tied to its thread — an
//! `Rc`, a `JsValue` — keeps that value behind a task of its own and talks to
//! it over a channel instead. `ui/not_send.rs` pins the rejection, checked
//! with `trybuild`: it happens at the impl, not at some later spawn site.

#[test]
fn a_context_whose_futures_are_not_send_cannot_implement_a_capability() {
    trybuild::TestCases::new().compile_fail("tests/ui/not_send.rs");
}
