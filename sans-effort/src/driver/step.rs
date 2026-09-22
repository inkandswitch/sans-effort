//! What one call into a driver produced.

use alloc::{collections::VecDeque, vec::Vec};

/// The result of one [`start`](super::Driver::start) or
/// [`reply`](super::Driver::reply): the effects the routine recorded before
/// its next wait, and the ids of requests it abandoned along the way.
///
/// It iterates over the effects, so a host that has nothing to cancel uses it
/// like the `Vec` it replaces:
///
/// ```
/// # use sans_effort::driver::step::Step;
/// # use std::collections::VecDeque;
/// let step = Step::new(vec!["WriteLine", "ReadLine"], vec![]);
/// let queue: VecDeque<_> = step.into();
/// assert_eq!(queue, ["WriteLine", "ReadLine"]);
/// ```
///
/// A host whose effects cost something to perform — a timer, a network
/// request — reads [`closed`](Self::closed) and stops the work for those ids.
/// A closed id means the routine no longer needs the reply, not that the
/// effect did not happen.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "a step holds effects to perform and ids whose work can stop"]
pub struct Step<T> {
    effects: Vec<T>,
    closed: Vec<u64>,
}

impl<T> Step<T> {
    /// A step from its parts.
    pub const fn new(effects: Vec<T>, closed: Vec<u64>) -> Self {
        Self { effects, closed }
    }

    /// The effects recorded, in order.
    #[must_use]
    pub fn effects(&self) -> &[T] {
        &self.effects
    }

    /// Ids of requests the routine abandoned unanswered. A reply to one is
    /// discarded.
    #[must_use]
    pub fn closed(&self) -> &[u64] {
        &self.closed
    }

    /// `true` if there are no effects and nothing was closed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.closed.is_empty()
    }

    /// The effects and the closed ids.
    #[must_use]
    pub fn into_parts(self) -> (Vec<T>, Vec<u64>) {
        (self.effects, self.closed)
    }

    /// The same step with each effect transformed; the closed ids are kept.
    pub fn map<U, F: FnMut(T) -> U>(self, f: F) -> Step<U> {
        Step {
            effects: self.effects.into_iter().map(f).collect(),
            closed: self.closed,
        }
    }
}

impl<T> Default for Step<T> {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl<T> IntoIterator for Step<T> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.effects.into_iter()
    }
}

impl<T> From<Step<T>> for Vec<T> {
    fn from(step: Step<T>) -> Self {
        step.effects
    }
}

impl<T> From<Step<T>> for VecDeque<T> {
    fn from(step: Step<T>) -> Self {
        step.effects.into()
    }
}
