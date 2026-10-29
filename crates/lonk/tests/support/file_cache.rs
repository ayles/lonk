//! Test single-writer host adapter. Core language code performs no I/O.
//!
//! Packing records avoids an inode and filesystem block per scalar. The index
//! uses host RAM proportional to record count. Complete frames are immutable;
//! an interrupted final append is ignored and makes this log read-only. No
//! truncation, concurrent-writer support, eviction or crash durability is claimed.
use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

const START: &[u8; 8] = b"LONKREC1";
const END: &[u8; 8] = b"LONKEND1";
const HEADER: u64 = 48;
const OVERHEAD: u64 = HEADER + 8;

pub struct FileCache {
    file: fs::File,
    index: BTreeMap<ArtifactKey, (u64, u64)>,
    limit: u64,
    used: u64,
    end: u64,
    writable: bool,
}
impl FileCache {
    pub fn new(directory: PathBuf, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let used = fs::read_dir(&directory)?.try_fold(0u64, |total, entry| {
            Ok::<_, io::Error>(total.saturating_add(entry?.metadata()?.len()))
        })?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("artifacts-v1.log"))?;
        let length = file.metadata()?.len();
        let mut index = BTreeMap::new();
        let mut at = 0;
        while length.saturating_sub(at) >= OVERHEAD {
            file.seek(SeekFrom::Start(at))?;
            let mut header = [0u8; HEADER as usize];
            file.read_exact(&mut header)?;
            if &header[..8] != START {
                return Err(io::Error::other("invalid artifact log frame"));
            }
            let mut length_bytes = [0; 8];
            length_bytes.copy_from_slice(&header[40..]);
            let size = u64::from_le_bytes(length_bytes);
            let Some(end) = at.checked_add(OVERHEAD).and_then(|n| n.checked_add(size)) else {
                return Err(io::Error::other("artifact log length overflow"));
            };
            if end > length {
                break;
            }
            file.seek(SeekFrom::Start(end - 8))?;
            let mut footer = [0; 8];
            file.read_exact(&mut footer)?;
            if &footer != END {
                return Err(io::Error::other("invalid artifact log commit marker"));
            }
            let mut key = ArtifactKey([0; 32]);
            key.0.copy_from_slice(&header[8..40]);
            index.entry(key).or_insert((at + HEADER, size));
            at = end;
        }
        Ok(Self {
            file,
            index,
            limit,
            used,
            end: at,
            writable: at == length,
        })
    }
}
impl ArtifactStore for FileCache {
    fn get(
        &mut self,
        key: ArtifactKey,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ArtifactError> {
        let Some(&(offset, length)) = self.index.get(&key) else {
            return Ok(None);
        };
        if length > max_bytes as u64 {
            return Ok(None);
        }
        let result = (|| {
            self.file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length as usize];
            self.file.read_exact(&mut bytes)?;
            Ok(Some(bytes))
        })();
        result.map_err(|e: io::Error| ArtifactError(e.to_string()))
    }
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError> {
        if self.index.contains_key(&key) || !self.writable {
            return Ok(());
        }
        let size = bytes.len() as u64;
        let added = size.saturating_add(OVERHEAD);
        if self.used.saturating_add(added) > self.limit {
            return Ok(());
        }
        let result = (|| {
            self.file.seek(SeekFrom::Start(self.end))?;
            self.file.write_all(START)?;
            self.file.write_all(&key.0)?;
            self.file.write_all(&size.to_le_bytes())?;
            self.file.write_all(bytes)?;
            self.file.write_all(END)
        })();
        match result {
            Ok(()) => {
                self.index.insert(key, (self.end + HEADER, size));
                self.end += added;
                self.used += added;
                Ok(())
            }
            Err(error) => {
                self.writable = false;
                Err(ArtifactError(error.to_string()))
            }
        }
    }
}
