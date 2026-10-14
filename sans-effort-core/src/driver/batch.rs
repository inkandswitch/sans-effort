//! The effects of one yield, kept inline when there are few.
//!
//! Almost every yield carries one or two effects — an ask; a line written and
//! then a read. A `Vec` for them would cost an allocation on every resume and
//! reply. A [`Batch`] holds up to two in place and spills to a `Vec` beyond
//! that. The outbox records into one and the driver hands it over whole, so
//! a yield of two effects or fewer allocates nothing.

use alloc::{collections::VecDeque, vec::Vec};
use core::ops::Deref;

/// The effects of one yield, in order: up to two held inline, more in a
/// `Vec`. It derefs to a slice, and iterates by value.
pub struct Batch<T>(Inline<T>);

/// Invariant: `Many` only for three or more, so a batch has one form per
/// length.
enum Inline<T> {
    Empty,
    One([T; 1]),
    Two([T; 2]),
    Many(Vec<T>),
}

impl<T> Batch<T> {
    /// No effects.
    #[must_use]
    pub const fn new() -> Self {
        Self(Inline::Empty)
    }

    /// The effects, in order.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        match &self.0 {
            Inline::Empty => &[],
            Inline::One(one) => one,
            Inline::Two(two) => two,
            Inline::Many(many) => many,
        }
    }

    /// The same batch with each effect transformed, in order. Inline stays
    /// inline: no allocation for two effects or fewer.
    pub fn map<U, F: FnMut(T) -> U>(self, f: F) -> Batch<U> {
        Batch(match self.0 {
            Inline::Empty => Inline::Empty,
            Inline::One(one) => Inline::One(one.map(f)),
            Inline::Two(two) => Inline::Two(two.map(f)),
            Inline::Many(many) => Inline::Many(many.into_iter().map(f).collect()),
        })
    }

    /// Add `effect` after the others. The third moves them all to a `Vec`.
    pub(crate) fn push(&mut self, effect: T) {
        if let Inline::Many(many) = &mut self.0 {
            many.push(effect);
            return;
        }
        self.0 = match core::mem::replace(&mut self.0, Inline::Empty) {
            Inline::Empty => Inline::One([effect]),
            Inline::One([first]) => Inline::Two([first, effect]),
            Inline::Two([first, second]) => {
                let mut many = Vec::with_capacity(4);
                many.extend([first, second, effect]);
                Inline::Many(many)
            }
            Inline::Many(_) => unreachable!("pushed onto above"),
        };
    }
}

impl<T> Default for Batch<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Deref for Batch<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> AsRef<[T]> for Batch<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: PartialEq> PartialEq for Batch<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for Batch<T> {}

impl<T: Clone> Clone for Batch<T> {
    fn clone(&self) -> Self {
        Self(match &self.0 {
            Inline::Empty => Inline::Empty,
            Inline::One(one) => Inline::One(one.clone()),
            Inline::Two(two) => Inline::Two(two.clone()),
            Inline::Many(many) => Inline::Many(many.clone()),
        })
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for Batch<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T> From<Vec<T>> for Batch<T> {
    fn from(effects: Vec<T>) -> Self {
        let mut batch = Self::new();
        for effect in effects {
            batch.push(effect);
        }
        batch
    }
}

impl<T> From<Batch<T>> for Vec<T> {
    fn from(batch: Batch<T>) -> Self {
        match batch.0 {
            Inline::Empty => Self::new(),
            Inline::One(one) => one.into(),
            Inline::Two(two) => two.into(),
            Inline::Many(many) => many,
        }
    }
}

impl<T> From<Batch<T>> for VecDeque<T> {
    fn from(batch: Batch<T>) -> Self {
        Vec::from(batch).into()
    }
}

impl<T> IntoIterator for Batch<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter(match self.0 {
            Inline::Empty => Draining::Many(Vec::new().into_iter()),
            Inline::One(one) => Draining::One(one.into_iter()),
            Inline::Two(two) => Draining::Two(two.into_iter()),
            Inline::Many(many) => Draining::Many(many.into_iter()),
        })
    }
}

impl<'a, T> IntoIterator for &'a Batch<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/// A [`Batch`]'s effects, by value, in order.
pub struct IntoIter<T>(Draining<T>);

enum Draining<T> {
    One(core::array::IntoIter<T, 1>),
    Two(core::array::IntoIter<T, 2>),
    Many(alloc::vec::IntoIter<T>),
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match &mut self.0 {
            Draining::One(one) => one.next(),
            Draining::Two(two) => two.next(),
            Draining::Many(many) => many.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            Draining::One(one) => one.size_hint(),
            Draining::Two(two) => two.size_hint(),
            Draining::Many(many) => many.size_hint(),
        }
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        match &mut self.0 {
            Draining::One(one) => one.next_back(),
            Draining::Two(two) => two.next_back(),
            Draining::Many(many) => many.next_back(),
        }
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

impl<T> core::fmt::Debug for IntoIter<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IntoIter").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Any effects, in any number: through a batch and back, by every route
    /// out, the same effects in the same order.
    #[test]
    fn a_batch_keeps_its_effects_in_order() {
        bolero::check!()
            .with_type::<Vec<u16>>()
            .for_each(|effects| {
                let batch = Batch::from(effects.clone());
                assert_eq!(batch.as_slice(), effects.as_slice());
                let mut pushed = Batch::new();
                for &effect in effects {
                    pushed.push(effect);
                }
                assert_eq!(pushed, batch);
                assert_eq!(batch.len(), effects.len());
                assert_eq!(batch.as_ref(), effects.as_slice());
                assert_eq!((&batch).into_iter().copied().collect::<Vec<_>>(), *effects);
                assert_eq!(batch.clone().into_iter().collect::<Vec<_>>(), *effects);
                assert_eq!(batch.clone().into_iter().len(), effects.len());
                assert_eq!(
                    batch.clone().into_iter().rev().collect::<Vec<_>>(),
                    effects.iter().rev().copied().collect::<Vec<_>>()
                );
                assert_eq!(
                    batch.clone().map(u32::from).as_slice(),
                    effects.iter().copied().map(u32::from).collect::<Vec<_>>()
                );
                assert_eq!(Vec::from(batch.clone()), *effects);
                assert_eq!(VecDeque::from(batch), *effects);
            });
    }

    #[test]
    fn an_empty_batch_is_the_default_and_equals_only_an_empty_one() {
        assert_eq!(Batch::<u8>::default(), Batch::new());
        assert!(Batch::<u8>::new().is_empty());
        assert_ne!(Batch::from(vec![1_u8]), Batch::new());
        assert_eq!(
            alloc::format!("{:?}", Batch::from(vec![1_u8, 2, 3])),
            "[1, 2, 3]"
        );
    }
}
