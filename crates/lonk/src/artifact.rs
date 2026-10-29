//! Restart-safe artifacts. Storage and its disk budget belong to the embedder.
//!
//! Accept bytes only from a trusted producer using this build of lonk. Keys
//! prevent stale reuse and checksums detect damage; neither authenticates a
//! writer. This is not an API for executing uploaded bytecode.
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactKey(pub [u8; 32]);

impl fmt::Display for ArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactError(pub String);
impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ArtifactError {}

/// Records are immutable and writes must be atomic. Missing or over-limit
/// entries return `None`; other storage failures are explicit errors. Hosts
/// should enforce `max_bytes` before allocating a read buffer. Dropping records
/// is safe: a miss recomputes from the supplied immutable inputs.
pub trait ArtifactStore {
    fn get(&mut self, key: ArtifactKey, max_bytes: usize)
    -> Result<Option<Vec<u8>>, ArtifactError>;
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError>;
}

/// Reference adapter for tests. Retains bytes in RAM; it does not reduce memory.
#[derive(Default)]
pub struct MemoryArtifactStore {
    pub records: BTreeMap<ArtifactKey, Vec<u8>>,
}
impl ArtifactStore for MemoryArtifactStore {
    fn get(
        &mut self,
        key: ArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ArtifactError> {
        Ok(self
            .records
            .get(&key)
            .filter(|b| b.len() <= max_bytes)
            .cloned())
    }
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError> {
        self.records.insert(key, bytes.to_vec());
        Ok(())
    }
}

pub(crate) fn key(kind: u8, parts: &[&[u8]]) -> ArtifactKey {
    let mut hash = blake3::Hasher::new();
    hash.update(b"lonk-artifact-v2");
    hash.update(&[kind]);
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    ArtifactKey(*hash.finalize().as_bytes())
}
