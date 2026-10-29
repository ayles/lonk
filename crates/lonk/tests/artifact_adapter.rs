//! Exercise persistence and incomplete writes in the test single-writer host.
#[path = "support/file_cache.rs"]
mod file_cache;
use lonk::artifact::{ArtifactKey, ArtifactStore};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "lonk-artifact-adapter-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self, limit: u64) -> file_cache::FileCache {
        file_cache::FileCache::new(self.0.clone(), limit).unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn restart_bounds_admission_and_immutable_keys() {
    let dir = Directory::new();
    let a = ArtifactKey([1; 32]);
    let b = ArtifactKey([2; 32]);
    {
        let mut store = dir.open(128);
        store.put(a, b"original").unwrap();
        store.put(a, b"replacement").unwrap();
        store.put(b, &[0; 128]).unwrap();
    }
    let mut restarted = dir.open(128);
    assert_eq!(restarted.get(a, 64).unwrap(), Some(b"original".to_vec()));
    assert_eq!(restarted.get(a, 7).unwrap(), None);
    assert_eq!(restarted.get(b, 1024).unwrap(), None);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    assert!(fs::metadata(dir.0.join("artifacts-v1.log")).unwrap().len() <= 128);
}

#[test]
fn every_partial_final_append_preserves_previous_records_without_truncation() {
    let original = Directory::new();
    let a = ArtifactKey([1; 32]);
    let b = ArtifactKey([2; 32]);
    let log = original.0.join("artifacts-v1.log");
    let first_end;
    {
        let mut store = original.open(4096);
        store.put(a, b"first").unwrap();
        first_end = fs::metadata(&log).unwrap().len() as usize;
        store.put(b, b"second payload").unwrap();
    }
    let bytes = fs::read(&log).unwrap();
    for end in first_end + 1..bytes.len() {
        let dir = Directory::new();
        let target = dir.0.join("artifacts-v1.log");
        fs::write(&target, &bytes[..end]).unwrap();
        let mut store = dir.open(4096);
        assert_eq!(store.get(a, 100).unwrap(), Some(b"first".to_vec()));
        assert_eq!(store.get(b, 100).unwrap(), None);
        store.put(b, b"retry").unwrap();
        assert_eq!(fs::read(target).unwrap(), bytes[..end]);
    }
}

#[test]
fn corrupt_committed_frame_is_an_error() {
    let dir = Directory::new();
    {
        dir.open(4096).put(ArtifactKey([1; 32]), b"data").unwrap();
    }
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(dir.0.join("artifacts-v1.log"))
        .unwrap();
    file.write_all(&[0; 64]).unwrap();
    assert!(file_cache::FileCache::new(dir.0.clone(), 4096).is_err());
}

#[test]
fn empty_payload_round_trips_and_excess_length_disables_writes() {
    let dir = Directory::new();
    let key = ArtifactKey([9; 32]);
    {
        dir.open(4096).put(key, b"").unwrap();
    }
    assert_eq!(dir.open(4096).get(key, 0).unwrap(), Some(Vec::new()));
    let path = dir.0.join("artifacts-v1.log");
    let mut bytes = fs::read(&path).unwrap();
    bytes[40..48].copy_from_slice(&1_000_000u64.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let mut store = dir.open(4096);
    assert_eq!(store.get(key, 4096).unwrap(), None);
    store.put(ArtifactKey([8; 32]), b"later").unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
