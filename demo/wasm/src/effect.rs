//! One effect out of a batch, as JS sees it.

use crate::{
    kind::Kind,
    safe_integer::{NotSafe, SafeInteger},
};
use greeter_wire::View;
use wasm_bindgen::prelude::*;

/// carry them.
#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct Effect {
    kind: Kind,
    id: Option<SafeInteger>,
    name: Option<String>,
    text: Option<String>,
    millis: Option<SafeInteger>,
}

#[wasm_bindgen]
impl Effect {
    /// Which effect.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The request id, if this effect awaits a reply. A plain `number`: ids
    /// count up from 1 per routine and never approach 2^53.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn id(&self) -> Option<f64> {
        self.id.map(f64::from)
    }

    /// The name to look up, for `Lookup`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// The text to show, for `Write`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.text.clone()
    }

    /// How long to wait in milliseconds, for `Sleep`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn millis(&self) -> Option<f64> {
        self.millis.map(f64::from)
    }
}

impl TryFrom<View> for Effect {
    type Error = NotSafe;

    /// Fails only if an id or a duration exceeds 2^53 − 1, which the routine
    /// never produces; the check lives here so the getters can be total.
    fn try_from(view: View) -> Result<Self, Self::Error> {
        let blank = Self {
            kind: Kind::Write,
            id: None,
            name: None,
            text: None,
            millis: None,
        };

        Ok(match view {
            View::Count { id } => Self {
                kind: Kind::Count,
                id: Some(id.try_into()?),
                ..blank
            },
            View::Lookup { name, id } => Self {
                kind: Kind::Lookup,
                id: Some(id.try_into()?),
                name: Some(name),
                ..blank
            },
            View::ReadLine { id } => Self {
                kind: Kind::ReadLine,
                id: Some(id.try_into()?),
                ..blank
            },
            View::Sleep { millis, id } => Self {
                kind: Kind::Sleep,
                id: Some(id.try_into()?),
                millis: Some(millis.try_into()?),
                ..blank
            },
            View::Write { text } => Self {
                text: Some(text),
                ..blank
            },
        })
    }
}
