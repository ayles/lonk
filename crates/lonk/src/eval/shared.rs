//! Persistent AVL containers. Cloning a root is constant time; writes use
//! Rc::make_mut along one balanced path and keep other subtrees shared.
//! Payloads are thunk IDs, never demanded or copied by container operations.
// These assertions describe private AVL invariants, never unchecked source
// input. The evaluator validates indices/ranges before entering the container;
// rotations and joins establish nonempty children from cached subtree heights.
#![allow(clippy::expect_used)]
use super::ThunkId;
use std::{cell::Cell, cmp::Ordering, rc::Rc};
mod storage;

type Link<T> = Option<Rc<Node<T>>>;

#[derive(Clone, Debug)]
struct Node<T> {
    // A nonzero id names immutable stored content. Every mutation clears it,
    // including mutations of uniquely owned and freshly reloaded nodes.
    stored_id: Cell<u64>,
    value: T,
    left: Link<T>,
    right: Link<T>,
    height: usize,
    len: usize,
    bytes: usize,
}

fn height<T>(tree: &Link<T>) -> usize {
    tree.as_ref().map_or(0, |n| n.height)
}

fn len<T>(tree: &Link<T>) -> usize {
    tree.as_ref().map_or(0, |n| n.len)
}

trait Payload: Clone {
    fn heap_bytes(&self) -> usize {
        0
    }
}
impl Payload for ThunkId {}

impl<T: Payload> Node<T> {
    fn new(value: T, left: Link<T>, right: Link<T>) -> Rc<Self> {
        let mut node = Self {
            stored_id: Cell::new(0),
            value,
            left,
            right,
            height: 0,
            len: 0,
            bytes: 0,
        };
        node.refresh();
        Rc::new(node)
    }

    fn refresh(&mut self) {
        self.height = 1 + height(&self.left).max(height(&self.right));
        self.len = 1 + len(&self.left) + len(&self.right);
        // A shared DAG can represent a huge logical sequence compactly.
        // Conservative duplicate accounting must not overflow.
        self.bytes = std::mem::size_of::<Self>()
            .saturating_add(2 * std::mem::size_of::<usize>())
            .saturating_add(self.value.heap_bytes())
            .saturating_add(self.left.as_ref().map_or(0, |n| n.bytes))
            .saturating_add(self.right.as_ref().map_or(0, |n| n.bytes));
    }
}

fn node_mut<T: Payload>(root: &mut Rc<Node<T>>) -> &mut Node<T> {
    let node = Rc::make_mut(root);
    node.stored_id.set(0);
    node
}

fn rotate_left<T: Payload>(root: &mut Rc<Node<T>>) {
    let node = node_mut(root);
    let mut pivot = node.right.take().expect("right child for rotation");
    node.right = node_mut(&mut pivot).left.take();
    node.refresh();
    let previous = std::mem::replace(root, pivot);
    let node = node_mut(root);
    node.left = Some(previous);
    node.refresh();
}

fn rotate_right<T: Payload>(root: &mut Rc<Node<T>>) {
    let node = node_mut(root);
    let mut pivot = node.left.take().expect("left child for rotation");
    node.left = node_mut(&mut pivot).right.take();
    node.refresh();
    let previous = std::mem::replace(root, pivot);
    let node = node_mut(root);
    node.right = Some(previous);
    node.refresh();
}

fn balance<T: Payload>(root: &mut Rc<Node<T>>) {
    node_mut(root).refresh();
    if height(&root.left) > height(&root.right) + 1 {
        let left = node_mut(root).left.as_mut().expect("tall left child");
        if height(&left.right) > height(&left.left) {
            rotate_left(left);
        }
        rotate_right(root);
    } else if height(&root.right) > height(&root.left) + 1 {
        let right = node_mut(root).right.as_mut().expect("tall right child");
        if height(&right.left) > height(&right.right) {
            rotate_right(right);
        }
        rotate_left(root);
    }
}

/// Join ordered sequences whose heights can differ arbitrarily.
fn join<T: Payload>(left: Link<T>, value: T, right: Link<T>) -> Rc<Node<T>> {
    if height(&left) > height(&right) + 1 {
        let mut root = left.expect("tall left tree");
        let node = node_mut(&mut root);
        node.right = Some(join(node.right.take(), value, right));
        balance(&mut root);
        root
    } else if height(&right) > height(&left) + 1 {
        let mut root = right.expect("tall right tree");
        let node = node_mut(&mut root);
        node.left = Some(join(left, value, node.left.take()));
        balance(&mut root);
        root
    } else {
        Node::new(value, left, right)
    }
}

fn pop_first<T: Payload>(tree: &mut Link<T>) -> T {
    let mut root = tree.take().expect("nonempty tree");
    let node = node_mut(&mut root);
    if node.left.is_none() {
        let value = node.value.clone();
        *tree = node.right.take();
        value
    } else {
        let value = pop_first(&mut node.left);
        balance(&mut root);
        *tree = Some(root);
        value
    }
}

fn concat<T: Payload>(left: Link<T>, mut right: Link<T>) -> Link<T> {
    if left.is_none() {
        right
    } else if right.is_none() {
        left
    } else {
        let value = pop_first(&mut right);
        Some(join(left, value, right))
    }
}

struct Iter<'a, T> {
    stack: Vec<&'a Node<T>>,
}

impl<'a, T> Iter<'a, T> {
    fn new(tree: &'a Link<T>) -> Self {
        let mut iter = Self { stack: Vec::new() };
        iter.descend(tree);
        iter
    }

    fn descend(&mut self, mut tree: &'a Link<T>) {
        while let Some(node) = tree {
            self.stack.push(node);
            tree = &node.left;
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.descend(&node.right);
        Some(&node.value)
    }
}

#[derive(Clone, Debug)]
struct Entry {
    key: Rc<str>,
    value: ThunkId,
}

impl Payload for Entry {
    fn heap_bytes(&self) -> usize {
        self.key.len() + 2 * std::mem::size_of::<usize>()
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Fields(Link<Entry>);

impl Fields {
    pub fn resident_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.bytes)
    }
    pub fn len(&self) -> usize {
        len(&self.0)
    }

    pub fn get(&self, key: &str) -> Option<&ThunkId> {
        let mut tree = &self.0;
        while let Some(node) = tree {
            match key.cmp(&node.value.key) {
                Ordering::Less => tree = &node.left,
                Ordering::Equal => return Some(&node.value.value),
                Ordering::Greater => tree = &node.right,
            }
        }
        None
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: impl Into<Rc<str>>, value: ThunkId) {
        fn insert(tree: &mut Link<Entry>, entry: Entry) {
            let Some(root) = tree else {
                *tree = Some(Node::new(entry, None, None));
                return;
            };
            let node = node_mut(root);
            match entry.key.cmp(&node.value.key) {
                Ordering::Less => insert(&mut node.left, entry),
                Ordering::Equal => {
                    node.value.value = entry.value;
                    return;
                }
                Ordering::Greater => insert(&mut node.right, entry),
            }
            balance(root);
        }
        insert(
            &mut self.0,
            Entry {
                key: key.into(),
                value,
            },
        );
    }

    pub fn remove(&mut self, key: &str) {
        fn remove(tree: &mut Link<Entry>, key: &str) {
            let Some(root) = tree else { return };
            let node = node_mut(root);
            match key.cmp(&node.value.key) {
                Ordering::Less => remove(&mut node.left, key),
                Ordering::Greater => remove(&mut node.right, key),
                Ordering::Equal => {
                    if node.left.is_none() {
                        *tree = node.right.take();
                        return;
                    }
                    if node.right.is_none() {
                        *tree = node.left.take();
                        return;
                    }
                    node.value = pop_first(&mut node.right);
                }
            }
            balance(root);
        }
        // Absent removal should also preserve root identity.
        if self.contains_key(key) {
            remove(&mut self.0, key);
        }
    }

    pub fn extend(&mut self, more: Self) {
        if self.0.is_none() {
            *self = more;
        } else if !self
            .0
            .as_ref()
            .zip(more.0.as_ref())
            .is_some_and(|(a, b)| Rc::ptr_eq(a, b))
        {
            for entry in Iter::new(&more.0) {
                self.insert(entry.key.clone(), entry.value);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ThunkId)> {
        Iter::new(&self.0).map(|entry| (entry.key.as_ref(), &entry.value))
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = &ThunkId> {
        self.iter().map(|(_, v)| v)
    }
}

impl<K: Into<Rc<str>>> FromIterator<(K, ThunkId)> for Fields {
    fn from_iter<I: IntoIterator<Item = (K, ThunkId)>>(values: I) -> Self {
        let mut fields = Self::default();
        for (key, value) in values {
            fields.insert(key, value);
        }
        fields
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Items(Link<ThunkId>);

impl Items {
    pub fn resident_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.bytes)
    }
    pub fn len(&self) -> usize {
        len(&self.0)
    }

    pub fn get(&self, mut index: usize) -> Option<&ThunkId> {
        let mut tree = &self.0;
        while let Some(node) = tree {
            let left = len(&node.left);
            match index.cmp(&left) {
                Ordering::Less => tree = &node.left,
                Ordering::Equal => return Some(&node.value),
                Ordering::Greater => {
                    index -= left + 1;
                    tree = &node.right;
                }
            }
        }
        None
    }

    pub fn set(&mut self, index: usize, value: ThunkId) {
        fn set(tree: &mut Link<ThunkId>, index: usize, value: ThunkId) {
            let node = node_mut(tree.as_mut().expect("checked array index"));
            let left = len(&node.left);
            match index.cmp(&left) {
                Ordering::Less => set(&mut node.left, index, value),
                Ordering::Equal => node.value = value,
                Ordering::Greater => set(&mut node.right, index - left - 1, value),
            }
        }
        set(&mut self.0, index, value);
    }

    pub fn push(&mut self, value: ThunkId) {
        debug_assert!(
            self.len().checked_add(1).is_some(),
            "caller checks logical array length"
        );
        self.0 = Some(join(self.0.take(), value, None));
    }

    pub fn extend(&mut self, more: Self) {
        debug_assert!(
            self.len().checked_add(more.len()).is_some(),
            "caller checks logical array length"
        );
        self.0 = concat(self.0.take(), more.0);
    }

    pub fn slice(&self, start: usize, end: usize) -> Self {
        fn slice(tree: &Link<ThunkId>, start: usize, end: usize) -> Link<ThunkId> {
            if start == end {
                return None;
            }
            if start == 0 && end == len(tree) {
                return tree.clone();
            }
            let node = tree.as_ref().expect("checked slice range");
            let at = len(&node.left);
            if end <= at {
                return slice(&node.left, start, end);
            }
            if start > at {
                return slice(&node.right, start - at - 1, end - at - 1);
            }
            Some(join(
                slice(&node.left, start, at),
                node.value,
                slice(&node.right, 0, end - at - 1),
            ))
        }
        Self(slice(&self.0, start, end))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ThunkId> {
        Iter::new(&self.0)
    }
}

impl FromIterator<ThunkId> for Items {
    fn from_iter<I: IntoIterator<Item = ThunkId>>(values: I) -> Self {
        fn build(values: &mut impl Iterator<Item = ThunkId>, length: usize) -> Link<ThunkId> {
            if length == 0 {
                return None;
            }
            let left = length / 2;
            let before = build(values, left);
            let value = values.next().expect("known iterator length");
            let after = build(values, length - left - 1);
            Some(Node::new(value, before, after))
        }
        let values: Vec<_> = values.into_iter().collect();
        let length = values.len();
        Self(build(&mut values.into_iter(), length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn validate<T>(tree: &Link<T>) -> (usize, usize) {
        let Some(node) = tree else { return (0, 0) };
        let (lh, ll) = validate(&node.left);
        let (rh, rl) = validate(&node.right);
        assert!(lh.abs_diff(rh) <= 1);
        assert_eq!(node.height, 1 + lh.max(rh));
        assert_eq!(node.len, 1 + ll + rl);
        (node.height, node.len)
    }

    fn shared<T>(a: &Link<T>, b: &Link<T>) -> usize {
        fn addresses<T>(tree: &Link<T>, result: &mut std::collections::BTreeSet<usize>) {
            if let Some(node) = tree {
                result.insert(Rc::as_ptr(node) as usize);
                addresses(&node.left, result);
                addresses(&node.right, result);
            }
        }
        let mut aa = Default::default();
        let mut bb = Default::default();
        addresses(a, &mut aa);
        addresses(b, &mut bb);
        aa.intersection(&bb).count()
    }

    #[test]
    fn ordered_map_matches_reference_through_snapshots_and_removals() {
        let mut actual = Fields::default();
        let mut expected = BTreeMap::new();
        let mut snapshots = Vec::new();
        let mut seed = 17usize;
        for i in 0..4000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = format!("{:04}", (seed >> 8) % 257);
            if seed % 3 == 0 {
                actual.remove(&key);
                expected.remove(&key);
            } else {
                actual.insert(key.clone(), i);
                expected.insert(key, i);
            }
            validate(&actual.0);
            assert_eq!(
                actual
                    .iter()
                    .map(|(k, v)| (k.to_owned(), *v))
                    .collect::<BTreeMap<_, _>>(),
                expected
            );
            if i % 99 == 0 {
                snapshots.push((actual.clone(), expected.clone()));
            }
        }
        for (actual, expected) in snapshots {
            validate(&actual.0);
            assert_eq!(
                actual
                    .iter()
                    .map(|(k, v)| (k.to_owned(), *v))
                    .collect::<BTreeMap<_, _>>(),
                expected
            );
        }
    }

    #[test]
    fn writes_copy_paths_and_slices_share_their_interiors() {
        let fields: Fields = (0..4096).map(|i| (format!("{i:04}"), i)).collect();
        let mut edited = fields.clone();
        assert!(Rc::ptr_eq(
            fields.0.as_ref().unwrap(),
            edited.0.as_ref().unwrap()
        ));
        edited.insert("2000", 9999);
        assert_eq!(fields.get("2000"), Some(&2000));
        assert!(shared(&fields.0, &edited.0) >= 4096 - 20);
        edited.remove("1000");
        assert!(fields.contains_key("1000"));
        assert!(!edited.contains_key("1000"));
        assert!(shared(&fields.0, &edited.0) >= 4096 - 40);
        validate(&edited.0);

        let items: Items = (0..4096).collect();
        let mut edited = items.clone();
        edited.set(2000, 9999);
        assert_eq!(items.get(2000), Some(&2000));
        assert!(shared(&items.0, &edited.0) >= 4096 - 20);
        let middle = items.slice(1000, 3000);
        validate(&middle.0);
        assert_eq!(
            middle.iter().copied().collect::<Vec<_>>(),
            (1000..3000).collect::<Vec<_>>()
        );
        assert!(shared(&items.0, &middle.0) >= 2000 - 40);
    }

    #[test]
    fn array_concats_and_slices_match_flat_arrays_and_stay_balanced() {
        for size in 0..80 {
            let items: Items = (0..size).collect();
            for start in 0..=size {
                for end in start..=size {
                    let part = items.slice(start, end);
                    validate(&part.0);
                    assert_eq!(
                        part.iter().copied().collect::<Vec<_>>(),
                        (start..end).collect::<Vec<_>>()
                    );
                }
            }
        }
        let mut actual = Items::default();
        let mut expected = Vec::new();
        for i in 0..500 {
            let snapshot = actual.clone();
            let mut prefix: Items = (0..i % 7).collect();
            prefix.extend(actual);
            prefix.extend((0..i % 13).collect());
            prefix.push(i);
            actual = prefix;
            let old = expected;
            expected = (0..i % 7)
                .chain(old.iter().copied())
                .chain(0..i % 13)
                .chain([i])
                .collect();
            validate(&actual.0);
            assert_eq!(actual.iter().copied().collect::<Vec<_>>(), expected);
            assert_eq!(snapshot.iter().copied().collect::<Vec<_>>(), old);
        }
    }
}
