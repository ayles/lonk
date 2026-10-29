//! Compact records preserve the persistent DAG rather than flatten snapshots.
use super::*;
use crate::cache::{CacheError, CacheKey};
use crate::eval::storage::{Storage, invalid};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

// IDs are process-unique, but records remain session-local: their payloads name
// session thunks. Allocate IDs only for nodes actually written to a store.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

trait Record: Payload {
    const TAG: u8;
    fn encode(&self, bytes: &mut Vec<u8>);
    fn decode(bytes: &[u8], key: CacheKey, thunks: usize) -> Result<Self, CacheError>;
    fn ordered(&self, _left: &Link<Self>, _right: &Link<Self>) -> bool {
        true
    }
}

fn thunk(bytes: &[u8], key: CacheKey, thunks: usize) -> Result<ThunkId, CacheError> {
    let value = word(bytes, key)?;
    usize::try_from(value)
        .ok()
        .filter(|id| *id < thunks)
        .ok_or_else(|| invalid(key, "invalid thunk reference"))
}

fn word(bytes: &[u8], key: CacheKey) -> Result<u64, CacheError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| invalid(key, "invalid record length"))?;
    Ok(u64::from_le_bytes(bytes))
}

impl Record for ThunkId {
    const TAG: u8 = 1;
    fn encode(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(*self as u64).to_le_bytes());
    }
    fn decode(bytes: &[u8], key: CacheKey, thunks: usize) -> Result<Self, CacheError> {
        thunk(bytes, key, thunks)
    }
}

impl Record for Entry {
    const TAG: u8 = 2;
    fn encode(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(self.value as u64).to_le_bytes());
        bytes.extend_from_slice(self.key.as_bytes());
    }
    fn decode(bytes: &[u8], key: CacheKey, thunks: usize) -> Result<Self, CacheError> {
        let (id, name) = bytes
            .split_at_checked(8)
            .ok_or_else(|| invalid(key, "truncated field entry"))?;
        Ok(Self {
            value: thunk(id, key, thunks)?,
            key: std::str::from_utf8(name)
                .map_err(|_| invalid(key, "invalid field name UTF-8"))?
                .into(),
        })
    }
    fn ordered(&self, left: &Link<Self>, right: &Link<Self>) -> bool {
        let mut before = left.as_ref();
        while let Some(node) = before {
            if node.right.is_none() {
                break;
            }
            before = node.right.as_ref();
        }
        let mut after = right.as_ref();
        while let Some(node) = after {
            if node.left.is_none() {
                break;
            }
            after = node.left.as_ref();
        }
        before.is_none_or(|n| n.value.key < self.key)
            && after.is_none_or(|n| n.value.key > self.key)
    }
}

fn store<T: Record>(tree: &Link<T>, storage: &mut Storage) -> Result<u64, CacheError> {
    let Some(node) = tree else { return Ok(0) };
    let mut id = node.stored_id.get();
    if id == 0 {
        id = NEXT_ID
            .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |id| {
                id.checked_add(1)
            })
            .map_err(|_| CacheError {
                operation: "encode",
                key: None,
                message: "container version IDs exhausted".into(),
            })?;
        node.stored_id.set(id);
    }
    let key = CacheKey::Node(id);
    if storage.contains(key)? {
        return Ok(id);
    }
    let left = store(&node.left, storage)?;
    let right = store(&node.right, storage)?;
    let mut bytes = b"LKN1".to_vec();
    bytes.push(T::TAG);
    bytes.extend_from_slice(&left.to_le_bytes());
    bytes.extend_from_slice(&right.to_le_bytes());
    node.value.encode(&mut bytes);
    storage.put(key, &bytes)?;
    Ok(id)
}

fn load<T: Record>(id: u64, storage: &mut Storage, thunks: usize) -> Result<Link<T>, CacheError> {
    fn visit<T: Record>(
        id: u64,
        storage: &mut Storage,
        thunks: usize,
        depth: usize,
        nodes: &mut HashMap<u64, Rc<Node<T>>>,
    ) -> Result<Link<T>, CacheError> {
        if id == 0 {
            return Ok(None);
        }
        if let Some(node) = nodes.get(&id) {
            return Ok(Some(node.clone()));
        }
        let key = CacheKey::Node(id);
        // A valid AVL tree fitting in usize has fewer than 96 levels. This
        // also bounds malformed cycles before recursion consumes the stack.
        if depth >= 96 {
            return Err(invalid(key, "container depth or cycle is invalid"));
        }
        let bytes = storage.get(key)?;
        if bytes.len() < 21 || &bytes[..4] != b"LKN1" || bytes[4] != T::TAG {
            return Err(invalid(key, "invalid container record kind or version"));
        }
        let left_id = word(&bytes[5..13], key)?;
        let right_id = word(&bytes[13..21], key)?;
        let value = T::decode(&bytes[21..], key, thunks)?;
        let left = visit(left_id, storage, thunks, depth + 1, nodes)?;
        let right = visit(right_id, storage, thunks, depth + 1, nodes)?;
        if height(&left).abs_diff(height(&right)) > 1
            || len(&left)
                .checked_add(len(&right))
                .and_then(|n| n.checked_add(1))
                .is_none()
            || !value.ordered(&left, &right)
        {
            return Err(invalid(key, "invalid container ordering, size or balance"));
        }
        let node = Node::new(value, left, right);
        node.stored_id.set(id);
        nodes.insert(id, node.clone());
        Ok(Some(node))
    }
    visit(id, storage, thunks, 0, &mut HashMap::new())
}

impl Fields {
    pub(in crate::eval) fn store(&self, storage: &mut Storage) -> Result<u64, CacheError> {
        store(&self.0, storage)
    }
    pub(in crate::eval) fn load(
        id: u64,
        storage: &mut Storage,
        thunks: usize,
    ) -> Result<Self, CacheError> {
        load(id, storage, thunks).map(Self)
    }
}

impl Items {
    pub(in crate::eval) fn store(&self, storage: &mut Storage) -> Result<u64, CacheError> {
        store(&self.0, storage)
    }
    pub(in crate::eval) fn load(
        id: u64,
        storage: &mut Storage,
        thunks: usize,
    ) -> Result<Self, CacheError> {
        load(id, storage, thunks).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CachePolicy, MemoryStore};

    fn storage() -> Storage {
        Storage::new(
            Box::new(MemoryStore::default()),
            CachePolicy {
                resident_payload_budget: 0,
            },
        )
    }

    #[test]
    fn compact_dags_with_huge_logical_sizes_do_not_overflow_estimates() {
        let mut items: Items = [0].into_iter().collect();
        for _ in 0..usize::BITS - 2 {
            items.extend(items.clone());
        }
        assert_eq!(items.len(), 1usize << (usize::BITS - 2));
        assert_eq!(items.get(items.len() - 1), Some(&0));
        assert_eq!(items.resident_bytes(), usize::MAX);
        let mut storage = storage();
        let root = items.store(&mut storage).unwrap();
        assert!(storage.stats.records_written < 10_000);
        let loaded = Items::load(root, &mut storage, 1).unwrap();
        assert_eq!(loaded.len(), items.len());
        assert_eq!(loaded.get(loaded.len() - 1), Some(&0));
    }

    #[test]
    fn snapshots_write_only_changed_paths_and_reloaded_writes_get_new_ids() {
        let mut storage = storage();
        let original: Items = (0..4096).collect();
        let first = original.store(&mut storage).unwrap();
        assert_eq!(storage.stats.records_written, 4096);
        assert_eq!(storage.stats.bytes_written, 4096 * 29);
        let mut edited = original.clone();
        edited.set(2000, 4999);
        let second = edited.store(&mut storage).unwrap();
        assert_ne!(first, second);
        assert!(storage.stats.records_written <= 4096 + 13);
        let loaded = Items::load(first, &mut storage, 5000).unwrap();
        assert_eq!(loaded.get(2000), Some(&2000));
        // Uniquely owned reloaded paths must invalidate stored IDs too.
        let mut loaded = loaded;
        loaded.set(2000, 4500);
        let third = loaded.store(&mut storage).unwrap();
        assert_ne!(first, third);
        assert_eq!(
            Items::load(first, &mut storage, 5000).unwrap().get(2000),
            Some(&2000)
        );
        assert_eq!(
            Items::load(second, &mut storage, 5000).unwrap().get(2000),
            Some(&4999)
        );
        assert_eq!(
            Items::load(third, &mut storage, 5000).unwrap().get(2000),
            Some(&4500)
        );
    }

    #[test]
    fn map_rotations_removals_and_array_slices_keep_stored_versions_immutable() {
        let mut storage = storage();
        let mut fields = Fields::default();
        let mut expected = std::collections::BTreeMap::new();
        let mut versions = Vec::new();
        for i in 0..160 {
            let key = format!("{:03}", i % 53);
            if i % 4 == 0 {
                fields.remove(&key);
                expected.remove(&key);
            } else {
                fields.insert(key.clone(), i);
                expected.insert(key, i);
            }
            let id = fields.store(&mut storage).unwrap();
            versions.push((id, expected.clone()));
            // Force the next edit to operate on uniquely owned restored nodes.
            fields = Fields::load(id, &mut storage, 200).unwrap();
        }
        for (id, expected) in versions {
            let loaded = Fields::load(id, &mut storage, 200).unwrap();
            assert_eq!(
                loaded
                    .iter()
                    .map(|(k, v)| (k.to_owned(), *v))
                    .collect::<std::collections::BTreeMap<_, _>>(),
                expected
            );
        }
        let mut items: Items = (0..128).collect();
        let original = items.store(&mut storage).unwrap();
        items.extend(items.clone());
        let doubled = items.store(&mut storage).unwrap();
        let middle = items.slice(31, 200).store(&mut storage).unwrap();
        assert_eq!(
            Items::load(original, &mut storage, 200)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            (0..128).collect::<Vec<_>>()
        );
        assert_eq!(
            Items::load(doubled, &mut storage, 200)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            (0..128).cycle().take(256).collect::<Vec<_>>()
        );
        assert_eq!(
            Items::load(middle, &mut storage, 200)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            (0..128).cycle().skip(31).take(169).collect::<Vec<_>>()
        );
    }
}
