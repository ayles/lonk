//! Host-provided storage for in-session spill of ready values.
//!
//! The core encodes records and decides what to evict; the host decides where the
//! bytes live. Nothing here touches a filesystem. Keys are scoped to one evaluation
//! session and a key is never rewritten within it, so a store may deduplicate or
//! compress freely. A store failure is an infrastructure failure of the embedder,
//! never a language error: it is reported through [`EvaluationError::Infrastructure`]
//! and is never memoized as a failed value.
use crate::Diagnostic;
use std::fmt;

/// Version id of a persistent container node. A node id names immutable content:
/// every node whose content changes receives a fresh id, even when it is uniquely
/// owned, so a stored record stays valid for every value that still references it.
pub type NodeId = u64;

/// Id of a thunk whose ready string may leave memory. Scalars stay inline in their
/// thunks and closures keep only metadata, so today only strings use this kind.
pub type ThunkKey = u64;

/// What a record describes. Both kinds are session scoped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheKey {
    Node(NodeId),
    Thunk(ThunkKey),
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(id) => write!(f, "node {id}"),
            Self::Thunk(id) => write!(f, "thunk {id}"),
        }
    }
}

/// A store operation failed. The embedder owns the cause; the evaluation that hit
/// it has no language-level meaning and must not be reported as one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheError {
    /// The store operation: `put`, `get`, `contains` or `release`.
    pub operation: &'static str,
    /// The key involved, if the operation had one.
    pub key: Option<CacheKey>,
    /// The host's description of the failure.
    pub message: String,
}

impl CacheError {
    pub fn new(operation: &'static str, key: Option<CacheKey>, message: impl Into<String>) -> Self {
        Self {
            operation,
            key,
            message: message.into(),
        }
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.key {
            Some(key) => write!(
                f,
                "cache {} failed for {key}: {}",
                self.operation, self.message
            ),
            None => write!(f, "cache {} failed: {}", self.operation, self.message),
        }
    }
}

impl std::error::Error for CacheError {}

/// Storage for one evaluation session. Implementations are free to keep records
/// in memory, in files, or anywhere else; they must retain every record until
/// [`CacheStore::release`] is called. Records are immutable and the host is
/// trusted to return the identical bytes it accepted: the decoder guards against
/// structurally malformed data (kinds, versions, bounds, ordering, depth, cycles)
/// and reports it as an infrastructure failure, but it is not a cryptographic
/// integrity check.
pub trait CacheStore {
    /// Store a record. A key is never rewritten within a session: storing a key
    /// that already exists is a successful no-op.
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError>;

    /// Load a record stored earlier in this session. A missing key is an error,
    /// never a silent miss: the core never recomputes a spilled ready value.
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError>;

    /// Whether a record is already stored, so persistence of a shared subtree can
    /// stop at a node the store already holds. May answer `false` for a key that
    /// was never written; must answer `true` for every key `put` accepted.
    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError>;

    /// Drop every record. Called once when the evaluation session ends, and the
    /// result is reported: a `Drop` implementation may release on a best-effort
    /// basis but cannot claim successful cleanup.
    fn release(&mut self) -> Result<(), CacheError>;
}

/// Limits for in-session spill. The budget bounds the summed sizes of ready
/// strings and containers reachable from memoized roots, as estimated by the
/// core at successful force boundaries (a duplicate reachable through two roots
/// is counted twice, conservatively). It excludes the active working set of a
/// demand in progress, values already exported to the host, and all graph,
/// frame, thunk and closure metadata, which stay resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    pub resident_payload_budget: usize,
}

impl CachePolicy {
    pub const fn new(resident_payload_budget: usize) -> Self {
        Self {
            resident_payload_budget,
        }
    }
}

/// Counters a session reports; relative comparisons are the intended use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub records_written: u64,
    pub bytes_written: u64,
    pub records_reloaded: u64,
    pub evictions: u64,
    pub resident_payload_bytes: usize,
}

/// The result of a cached evaluation: a language outcome, or a failure of the
/// embedder's storage that carries no language meaning.
#[derive(Debug)]
pub enum EvaluationError {
    Language(Diagnostic),
    Infrastructure(CacheError),
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(diagnostic) => write!(f, "{}", diagnostic.message),
            Self::Infrastructure(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for EvaluationError {}

impl From<Diagnostic> for EvaluationError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Language(diagnostic)
    }
}

impl From<CacheError> for EvaluationError {
    fn from(error: CacheError) -> Self {
        Self::Infrastructure(error)
    }
}

/// A store that keeps records in memory. The reference implementation for tests
/// and for hosts that only want the eviction accounting.
#[derive(Debug, Default)]
pub struct MemoryStore {
    records: std::collections::BTreeMap<CacheKey, Vec<u8>>,
    released: bool,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl CacheStore for MemoryStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        if self.released {
            return Err(CacheError::new("put", Some(key), "store already released"));
        }
        self.records.entry(key).or_insert_with(|| record.to_vec());
        Ok(())
    }

    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        self.records
            .get(&key)
            .cloned()
            .ok_or_else(|| CacheError::new("get", Some(key), "record is not stored"))
    }

    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        Ok(self.records.contains_key(&key))
    }

    fn release(&mut self) -> Result<(), CacheError> {
        self.records.clear();
        self.released = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_keeps_first_record_and_reports_misses() {
        let mut store = MemoryStore::new();
        let key = CacheKey::Node(7);
        assert!(!store.contains(key).unwrap());
        store.put(key, b"first").unwrap();
        store.put(key, b"second").unwrap();
        assert!(store.contains(key).unwrap());
        assert_eq!(store.get(key).unwrap(), b"first");
        let missing = store.get(CacheKey::Thunk(1)).unwrap_err();
        assert_eq!(missing.operation, "get");
        assert_eq!(
            missing.to_string(),
            "cache get failed for thunk 1: record is not stored"
        );
        store.release().unwrap();
        assert!(store.is_empty());
        assert!(store.put(key, b"late").is_err());
    }
}
