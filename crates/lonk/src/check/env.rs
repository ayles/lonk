//! Checker branches share a snapshot until a binding is written.
use super::Binding;
use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Default)]
pub(super) struct Env(Arc<BTreeMap<String, Binding>>);

impl Env {
    pub(super) fn new() -> Self {
        Self::default()
    }
}

impl Deref for Env {
    type Target = BTreeMap<String, Binding>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Env {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a> IntoIterator for &'a Env {
    type Item = (&'a String, &'a Binding);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Binding>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Env {
    type Item = (&'a String, &'a mut Binding);
    type IntoIter = std::collections::btree_map::IterMut<'a, String, Binding>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
