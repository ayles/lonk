//! Session spill mechanics. Only the embedder's CacheStore performs I/O.
use super::{Evaluation, Ready, State, ThunkId};
use crate::cache::{CacheError, CacheKey, CachePolicy, CacheStats, CacheStore};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub(super) enum Stored {
    String(CacheKey),
    Fields(u64),
    Items(u64),
}

pub(super) struct Storage {
    pub store: Box<dyn CacheStore>,
    pub policy: CachePolicy,
    pub stats: CacheStats,
    resident_bytes: u128,
    pub candidates: VecDeque<ThunkId>,
}

impl Storage {
    pub fn new(store: Box<dyn CacheStore>, policy: CachePolicy) -> Self {
        Self {
            store,
            policy,
            stats: CacheStats::default(),
            resident_bytes: 0,
            candidates: VecDeque::new(),
        }
    }

    pub fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.store.contains(key)
    }

    pub fn put(&mut self, key: CacheKey, bytes: &[u8]) -> Result<(), CacheError> {
        self.store.put(key, bytes)?;
        self.stats.records_written += 1;
        self.stats.bytes_written += bytes.len() as u64;
        Ok(())
    }

    pub fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        let bytes = self.store.get(key)?;
        self.stats.records_reloaded += 1;
        Ok(bytes)
    }

    fn report_resident(&mut self) {
        self.stats.resident_payload_bytes = self.resident_bytes.min(usize::MAX as u128) as usize;
    }
}

pub(super) fn invalid(key: CacheKey, message: impl Into<String>) -> CacheError {
    CacheError {
        operation: "decode",
        key: Some(key),
        message: message.into(),
    }
}

impl Ready {
    pub(super) fn payload_bytes(&self) -> usize {
        match self {
            Self::Str(s) => s.len() + 2 * std::mem::size_of::<usize>(),
            Self::Record(fields) => fields.resident_bytes(),
            Self::Array(items) => items.resident_bytes(),
            // These are inline values or execution metadata, not spillable data.
            _ => 0,
        }
    }

    fn store(&self, id: ThunkId, storage: &mut Storage) -> Result<Stored, CacheError> {
        match self {
            Self::Str(s) => {
                let key = CacheKey::Thunk(id as u64);
                if !storage.contains(key)? {
                    let mut record = b"LKS1".to_vec();
                    record.extend_from_slice(s.as_bytes());
                    storage.put(key, &record)?;
                }
                Ok(Stored::String(key))
            }
            Self::Record(fields) => fields.store(storage).map(Stored::Fields),
            Self::Array(items) => items.store(storage).map(Stored::Items),
            _ => unreachable!("only heap data enters the eviction queue"),
        }
    }
}

impl Stored {
    pub(super) fn load(self, storage: &mut Storage, thunks: usize) -> Result<Ready, CacheError> {
        match self {
            Self::String(key) => {
                let bytes = storage.get(key)?;
                let text = bytes
                    .strip_prefix(b"LKS1")
                    .ok_or_else(|| invalid(key, "invalid string record"))?;
                let text =
                    std::str::from_utf8(text).map_err(|_| invalid(key, "invalid string UTF-8"))?;
                Ok(Ready::Str(text.into()))
            }
            Self::Fields(root) => super::Fields::load(root, storage, thunks).map(Ready::Record),
            Self::Items(root) => super::Items::load(root, storage, thunks).map(Ready::Array),
        }
    }
}

impl Evaluation<'_> {
    pub(super) fn register_ready(&mut self, id: ThunkId, value: &Ready) {
        if let Some(storage) = &mut self.storage {
            let bytes = value.payload_bytes();
            if bytes != 0 {
                storage.resident_bytes += bytes as u128;
                storage.report_resident();
                storage.candidates.push_back(id);
            }
        }
    }

    // A demand chain is allowed its working set. Evict at the boundary, when
    // no suspended caller is retrying a partially read group of dependencies.
    pub(super) fn trim_cache(&mut self) -> Result<(), CacheError> {
        let Some(storage) = &mut self.storage else {
            return Ok(());
        };
        while storage.resident_bytes > storage.policy.resident_payload_budget as u128 {
            let Some(&id) = storage.candidates.front() else {
                break;
            };
            let State::Ready(value) = &self.thunks[id].state else {
                storage.candidates.pop_front();
                continue;
            };
            let bytes = value.payload_bytes();
            // Publish the spilled state only after every child and root record
            // is durable according to the host's contract. Failed writes leave
            // the original ready value intact and can be retried.
            let stored = value.store(id, storage)?;
            self.thunks[id].state = State::Spilled(stored);
            storage.candidates.pop_front();
            storage.stats.evictions += 1;
            storage.resident_bytes -= bytes as u128;
            storage.report_resident();
        }
        Ok(())
    }
}
