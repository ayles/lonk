//! Host adapter regressions for in-session spill.
//! The core never touches a filesystem: this file supplies a directory-backed store and a
//! failing store from outside the crate, checks the store contract on its own, and then checks
//! that a cached evaluation observes exactly what a plain evaluation observes, with a store
//! failure surfacing only as an infrastructure error and never as a language error.

use lonk::{CacheError, CacheKey, CachePolicy, CacheStore, EvaluationError, MemoryStore, Program};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

/// One file per key under a fresh directory; `release` removes the directory.
struct DirectoryStore {
    root: PathBuf,
    released: bool,
}

impl DirectoryStore {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "lonk-cache-adapter-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir(&root).expect("fresh temporary cache directory owned by this test");
        Self {
            root,
            released: false,
        }
    }

    fn path(&self, key: CacheKey) -> PathBuf {
        let name = match key {
            CacheKey::Node(id) => format!("node-{id}.rec"),
            CacheKey::Thunk(id) => format!("thunk-{id}.rec"),
        };
        self.root.join(name)
    }

    fn io(operation: &'static str, key: Option<CacheKey>, error: std::io::Error) -> CacheError {
        CacheError::new(operation, key, error.to_string())
    }
}

impl CacheStore for DirectoryStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        if self.released {
            return Err(CacheError::new("put", Some(key), "store already released"));
        }
        let path = self.path(key);
        if path.exists() {
            return Ok(());
        }
        // Write then rename so a partial record is never observable under its key.
        let partial = path.with_extension("partial");
        fs::write(&partial, record).map_err(|e| Self::io("put", Some(key), e))?;
        fs::rename(&partial, &path).map_err(|e| Self::io("put", Some(key), e))
    }

    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        let path = self.path(key);
        if !path.exists() {
            return Err(CacheError::new("get", Some(key), "record is not stored"));
        }
        fs::read(&path).map_err(|e| Self::io("get", Some(key), e))
    }

    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.path(key)
            .try_exists()
            .map_err(|e| Self::io("contains", Some(key), e))
    }

    fn release(&mut self) -> Result<(), CacheError> {
        self.released = true;
        match fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Self::io("release", None, e)),
        }
    }
}

/// Every write fails; reads of never-written keys fail; nothing is ever retained.
struct FailingStore {
    attempted: BTreeSet<CacheKey>,
}

impl CacheStore for FailingStore {
    fn put(&mut self, key: CacheKey, _record: &[u8]) -> Result<(), CacheError> {
        self.attempted.insert(key);
        Err(CacheError::new("put", Some(key), "disk full"))
    }
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        Err(CacheError::new("get", Some(key), "record is not stored"))
    }
    fn contains(&mut self, _key: CacheKey) -> Result<bool, CacheError> {
        Ok(false)
    }
    fn release(&mut self) -> Result<(), CacheError> {
        Ok(())
    }
}

#[test]
fn directory_store_honours_the_contract() {
    let mut store = DirectoryStore::new("contract");
    let node = CacheKey::Node(42);
    let thunk = CacheKey::Thunk(7);
    assert!(!store.contains(node).unwrap());
    store.put(node, b"first").unwrap();
    store.put(node, b"second").unwrap();
    assert!(store.contains(node).unwrap());
    assert_eq!(
        store.get(node).unwrap(),
        b"first",
        "a key is never rewritten"
    );
    store.put(thunk, &[0, 1, 2, 255]).unwrap();
    assert_eq!(store.get(thunk).unwrap(), vec![0, 1, 2, 255]);
    let missing = store.get(CacheKey::Thunk(8)).unwrap_err();
    assert_eq!(missing.operation, "get");
    assert_eq!(missing.key, Some(CacheKey::Thunk(8)));
    assert_eq!(
        missing.to_string(),
        "cache get failed for thunk 8: record is not stored"
    );
    let root = store.root.clone();
    assert!(root.exists());
    store.release().unwrap();
    assert!(!root.exists(), "release removes every record");
    assert!(
        store.put(node, b"late").is_err(),
        "a released store accepts nothing"
    );
}

#[test]
fn memory_store_matches_the_directory_store() {
    let mut memory = MemoryStore::new();
    let mut directory = DirectoryStore::new("parity");
    for (key, record) in [
        (CacheKey::Node(1), b"a".as_slice()),
        (CacheKey::Node(1), b"b".as_slice()),
        (CacheKey::Thunk(1), b"".as_slice()),
    ] {
        memory.put(key, record).unwrap();
        directory.put(key, record).unwrap();
    }
    for key in [CacheKey::Node(1), CacheKey::Thunk(1), CacheKey::Node(2)] {
        assert_eq!(
            memory.contains(key).unwrap(),
            directory.contains(key).unwrap(),
            "{key}"
        );
        assert_eq!(memory.get(key).ok(), directory.get(key).ok(), "{key}");
    }
    assert_eq!(memory.len(), 2);
    directory.release().unwrap();
}

const PROGRAMS: &[(&str, &str)] = &[
    (
        "lazy array around a poison",
        "let xs = [1 / 0, 2, 3]; let ys = [...xs, 4]; [ys.len(), ys[1:][0], ys[3]]",
    ),
    (
        "nested snapshots stay independent",
        "let a = { x = 1; n = { z = 1 } }; let b = a; b.n.z = 9; let c = b; c.x = 5; [a.n.z, b.n.z, a.x, c.x]",
    ),
    (
        "growing registry with repeated lookups",
        "let r: { ... } = {}; let acc: [int] = []; for n in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40] { r = object.with(r, \"k\" + n.to_str(), { v = n; p = 1 / 0 }); acc = [...acc, n * n]; }; [object.len(r), acc.len(), acc[39], match object.lookup(r, \"k7\") { { value: any; } => 1; else => 0; }]",
    ),
    (
        "strings and closures",
        "let make = fn(prefix: str) -> fn(int) -> str { fn(n) prefix + \"-\" + n.to_str() }; let f = make(\"pkg\"); [f(1), f(22)].join(\",\")",
    ),
    (
        "early return in a loop over a slice",
        "let find = fn(xs: [int], want: int) -> int { let i = 0; for x in xs[1:] { if x == want { return i; }; i += 1; }; 0 - 1 }; [find([9, 4, 5, 6], 6), find([9], 1)]",
    ),
];

fn plain(source: &str) -> Result<String, String> {
    let program = Program::prepare(source).unwrap_or_else(|e| panic!("{}", e.render(source)));
    program
        .evaluate(&[])
        .map(|v| v.to_string())
        .map_err(|e| e.message)
}

fn cached(
    source: &str,
    store: Box<dyn CacheStore>,
    budget: usize,
) -> (Result<String, EvaluationError>, lonk::CacheStats) {
    let program = Program::prepare(source).unwrap_or_else(|e| panic!("{}", e.render(source)));
    let mut evaluation = program.evaluator_with_cache(store, CachePolicy::new(budget));
    let result = evaluation.select(&[]).map(|v| v.to_string());
    let stats = evaluation.cache_stats();
    (result, stats)
}

#[test]
fn cached_evaluation_observes_what_plain_evaluation_observes() {
    for (name, source) in PROGRAMS {
        let expected = plain(source);
        for budget in [usize::MAX, 4096, 0] {
            let (result, stats) = cached(source, Box::new(MemoryStore::new()), budget);
            match (&expected, &result) {
                (Ok(want), Ok(got)) => assert_eq!(got, want, "{name} with budget {budget}"),
                (Err(want), Err(EvaluationError::Language(got))) => {
                    assert_eq!(&got.message, want, "{name} with budget {budget}")
                }
                (want, got) => {
                    panic!("{name} with budget {budget}: plain {want:?}, cached {got:?}")
                }
            }
            if stats.evictions > 0 {
                assert!(
                    stats.records_written > 0,
                    "{name}: evictions without records {stats:?}"
                );
            }
            if stats.records_reloaded > 0 {
                assert!(
                    stats.records_written > 0,
                    "{name}: reloads without records {stats:?}"
                );
            }
        }
    }
}

#[test]
fn cached_language_errors_keep_their_class_and_origin() {
    let source = "let xs = [1, 2]; let n = xs[5]; { a = 1; b = n }";
    let expected = plain(source).unwrap_err();
    let (result, _) = cached(source, Box::new(MemoryStore::new()), 0);
    match result {
        Err(EvaluationError::Language(error)) => assert_eq!(error.message, expected),
        other => panic!("expected the language error, got {other:?}"),
    }
}

#[test]
fn store_failures_are_infrastructure_errors_never_language_errors() {
    for (name, source) in PROGRAMS {
        let expected = plain(source);
        let (result, stats) = cached(
            source,
            Box::new(FailingStore {
                attempted: BTreeSet::new(),
            }),
            0,
        );
        match (&expected, &result) {
            (Ok(want), Ok(got)) => assert_eq!(got, want, "{name}: no eviction attempted"),
            (_, Err(EvaluationError::Infrastructure(error))) => {
                assert_eq!(error.operation, "put", "{name}: {error}");
                assert_eq!(
                    stats.records_written, 0,
                    "{name}: a failed put must not count as written"
                );
            }
            (Err(want), Err(EvaluationError::Language(got))) => {
                assert_eq!(&got.message, want, "{name}")
            }
            (want, got) => panic!("{name}: plain {want:?}, cached {got:?}"),
        }
    }
}

#[test]
fn directory_store_works_end_to_end() {
    let (name, source) = PROGRAMS[2];
    let expected = plain(source).unwrap();
    let store = DirectoryStore::new("end-to-end");
    let root = store.root.clone();
    let (result, stats) = cached(source, Box::new(store), 0);
    assert_eq!(
        result.map_err(|e| e.to_string()).unwrap(),
        expected,
        "{name}"
    );
    if stats.records_written > 0 {
        assert!(
            fs::read_dir(&root).map(|d| d.count()).unwrap_or(0) > 0 || !root.exists(),
            "records were written somewhere observable or already released"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// Fails the first `failures` writes, then behaves like a memory store.
struct FlakyStore {
    inner: MemoryStore,
    failures: usize,
    failed: usize,
}

impl CacheStore for FlakyStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        if self.failed < self.failures {
            self.failed += 1;
            return Err(CacheError::new("put", Some(key), "transient failure"));
        }
        self.inner.put(key, record)
    }
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        self.inner.get(key)
    }
    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.inner.contains(key)
    }
    fn release(&mut self) -> Result<(), CacheError> {
        self.inner.release()
    }
}

/// Accepts writes but hands back corrupted bytes, or nothing at all.
struct CorruptStore {
    inner: MemoryStore,
    lose: bool,
}

impl CacheStore for CorruptStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        self.inner.put(key, record)
    }
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        if self.lose {
            return Err(CacheError::new("get", Some(key), "record vanished"));
        }
        let mut bytes = self.inner.get(key)?;
        for byte in bytes.iter_mut() {
            *byte = !*byte;
        }
        Ok(bytes)
    }
    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.inner.contains(key)
    }
    fn release(&mut self) -> Result<(), CacheError> {
        self.inner.release()
    }
}

/// Releases nothing successfully.
struct StuckStore(MemoryStore);

impl CacheStore for StuckStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        self.0.put(key, record)
    }
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        self.0.get(key)
    }
    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.0.contains(key)
    }
    fn release(&mut self) -> Result<(), CacheError> {
        Err(CacheError::new("release", None, "directory busy"))
    }
}

/// A record program whose fields are selected one at a time, so each selection is a
/// force boundary at which a zero budget evicts everything ready.
const STAGED: &str = "\
let base = { name = \"zlib\"; version = 1; deps = [\"cc-1\", \"libc\"]; poison = 1 / 0 };
let wide = { ...base; extra = \"dropped\" };
let narrow: { name: str; version: int } = wide;
let updated = { ...narrow; version = 2 };
let tools = { double = fn(n: int) -> int { n * 2 }; label = fn(p: { name: str; version: int }) -> str { p.name + \"-\" + p.version.to_str() } };
let boxed = { tools = tools; data = base };
{
    first = base.deps;
    again = base.deps[1:];
    name = base.name;
    after_update = [narrow.version, updated.version, updated.name];
    projected = narrow;
    called = boxed.tools.label(boxed.data);
    doubled = boxed.tools.double(21);
    failing = base.poison;
    failing_again = base.poison + 1;
}";

fn staged_plain(path: &str) -> Result<String, String> {
    Program::prepare(STAGED)
        .unwrap_or_else(|e| panic!("{}", e.render(STAGED)))
        .evaluate(&[path])
        .map(|v| v.to_string())
        .map_err(|e| e.message)
}

#[test]
fn staged_selections_survive_eviction_and_reload() {
    let program = Program::prepare(STAGED).unwrap_or_else(|e| panic!("{}", e.render(STAGED)));
    let mut evaluation =
        program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
    // Order matters: each earlier selection leaves ready values behind that a zero
    // budget evicts, so later selections must reload and then copy, project or call.
    for (index, path) in [
        "first",
        "again",
        "name",
        "after_update",
        "projected",
        "called",
        "doubled",
    ]
    .iter()
    .enumerate()
    {
        let got = evaluation
            .select(&[path])
            .map_err(|e| e.to_string())
            .unwrap()
            .to_string();
        assert_eq!(got, staged_plain(path).unwrap(), "{path}");
        // A zero budget must leave nothing resident after a successful selection, must
        // have written records, and from the second selection on must have reloaded.
        let stats = evaluation.cache_stats();
        assert!(
            stats.evictions > 0,
            "{path}: no eviction at budget 0: {stats:?}"
        );
        assert!(
            stats.records_written > 0,
            "{path}: evictions without records: {stats:?}"
        );
        assert_eq!(
            stats.resident_payload_bytes, 0,
            "{path}: payload left resident: {stats:?}"
        );
        if index > 0 {
            assert!(
                stats.records_reloaded > 0,
                "{path}: later selections must reload: {stats:?}"
            );
        }
    }
    // A failing payload is memoized as a language failure, before and after spilling.
    let first = evaluation.select(&["failing"]);
    let forced = evaluation.forced_nodes();
    let second = evaluation.select(&["failing"]);
    for result in [first, second] {
        match result {
            Err(EvaluationError::Language(error)) => {
                assert_eq!(error.message, staged_plain("failing").unwrap_err())
            }
            other => panic!("expected the memoized language failure, got {other:?}"),
        }
    }
    assert_eq!(
        evaluation.forced_nodes(),
        forced,
        "a memoized failure forces nothing new"
    );
    match evaluation.select(&["failing_again"]) {
        Err(EvaluationError::Language(error)) => {
            assert_eq!(error.message, staged_plain("failing_again").unwrap_err())
        }
        other => panic!("expected the dependent language failure, got {other:?}"),
    }
    let stats = evaluation.cache_stats();
    assert!(
        stats.evictions > 0 && stats.records_written > 0 && stats.records_reloaded > 0,
        "{stats:?}"
    );
    evaluation.close().unwrap();
}

#[test]
fn transient_put_failures_are_not_memoized_and_retry_succeeds() {
    let program = Program::prepare(STAGED).unwrap_or_else(|e| panic!("{}", e.render(STAGED)));
    let mut evaluation = program.evaluator_with_cache(
        Box::new(FlakyStore {
            inner: MemoryStore::new(),
            failures: 1,
            failed: 0,
        }),
        CachePolicy::new(0),
    );
    let mut infrastructure = 0;
    for path in ["first", "again", "name", "after_update", "called"] {
        // Retry the same selection until the store recovers; the value must be the
        // plain one and never a language error.
        let mut attempts = 0;
        loop {
            attempts += 1;
            match evaluation.select(&[path]) {
                Ok(value) => {
                    assert_eq!(value.to_string(), staged_plain(path).unwrap(), "{path}");
                    break;
                }
                Err(EvaluationError::Infrastructure(error)) => {
                    infrastructure += 1;
                    assert_eq!(error.operation, "put", "{path}: {error}");
                    assert!(
                        attempts < 4,
                        "{path}: no progress after a transient failure"
                    );
                }
                Err(EvaluationError::Language(error)) => {
                    panic!("{path}: language error {}", error.message)
                }
            }
        }
    }
    let stats = evaluation.cache_stats();
    assert_eq!(
        infrastructure, 1,
        "exactly the first eviction write fails and is retried: {stats:?}"
    );
    assert!(
        stats.evictions > 0 && stats.records_written > 0,
        "{stats:?}"
    );
    evaluation.close().unwrap();
}

#[test]
fn corrupt_or_missing_records_are_infrastructure_failures() {
    for lose in [false, true] {
        let program = Program::prepare(STAGED).unwrap_or_else(|e| panic!("{}", e.render(STAGED)));
        let mut evaluation = program.evaluator_with_cache(
            Box::new(CorruptStore {
                inner: MemoryStore::new(),
                lose,
            }),
            CachePolicy::new(0),
        );
        let mut outcomes = Vec::new();
        for path in ["first", "again", "name", "after_update", "called"] {
            match evaluation.select(&[path]) {
                Ok(value) => outcomes.push(format!("ok {path} {value}")),
                Err(EvaluationError::Infrastructure(error)) => {
                    outcomes.push(format!("infrastructure {path} {}", error.operation));
                    assert!(
                        matches!(error.operation, "get" | "decode"),
                        "lose={lose} {path}: {error}"
                    );
                }
                Err(EvaluationError::Language(error)) => {
                    panic!(
                        "lose={lose} {path}: a bad record surfaced as a language error: {}",
                        error.message
                    )
                }
            }
        }
        let stats = evaluation.cache_stats();
        let failures = outcomes
            .iter()
            .filter(|o| o.starts_with("infrastructure"))
            .count();
        assert!(
            stats.records_written > 0,
            "lose={lose}: nothing was ever spilled: {stats:?}"
        );
        assert!(
            failures >= 1,
            "lose={lose}: a bad store must surface at least one storage failure: {outcomes:?} {stats:?}"
        );
        drop(evaluation);
    }
}

#[test]
fn release_failure_is_surfaced_by_close() {
    let program = Program::prepare("let xs = [1, 2, 3]; [xs.len(), xs[1:]]").unwrap();
    let mut evaluation = program.evaluator_with_cache(
        Box::new(StuckStore(MemoryStore::new())),
        CachePolicy::new(0),
    );
    assert_eq!(
        evaluation
            .select(&[])
            .map_err(|e| e.to_string())
            .unwrap()
            .to_string(),
        "[3,[2,3]]"
    );
    let error = evaluation.close().unwrap_err();
    assert_eq!(error.operation, "release");
    assert_eq!(error.to_string(), "cache release failed: directory busy");
}

/// Fails the first `failures` reads of string records only, so a record reloads fine
/// and the failure lands on a string read inside a pending demand chain.
struct ReadFlakyStore {
    inner: MemoryStore,
    failures: usize,
    failed: usize,
}

impl CacheStore for ReadFlakyStore {
    fn put(&mut self, key: CacheKey, record: &[u8]) -> Result<(), CacheError> {
        self.inner.put(key, record)
    }
    fn get(&mut self, key: CacheKey) -> Result<Vec<u8>, CacheError> {
        if matches!(key, CacheKey::Thunk(_)) && self.failed < self.failures {
            self.failed += 1;
            return Err(CacheError::new("get", Some(key), "transient read failure"));
        }
        self.inner.get(key)
    }
    fn contains(&mut self, key: CacheKey) -> Result<bool, CacheError> {
        self.inner.contains(key)
    }
    fn release(&mut self) -> Result<(), CacheError> {
        self.inner.release()
    }
}

#[test]
fn transient_read_failure_inside_a_pending_chain_is_retried_without_memoized_damage() {
    // `warm` forces and then spills the record and its name string. `later` is an
    // unevaluated dependent whose chain reloads the record (fine) and then the
    // string (fails once): the chain must be reset, not memoized as a failure or
    // reported as a demand cycle, and the retry must produce the plain value.
    let source = "let base = { name = \"zlib\"; deps = [\"cc-1\", \"libc\"] }; { warm = base.name; later = base.name + \"-\" + base.deps[0]; count = base.deps.len() }";
    let program = Program::prepare(source).unwrap_or_else(|e| panic!("{}", e.render(source)));
    let plain = |path: &str| program.evaluate(&[path]).map(|v| v.to_string()).unwrap();
    let mut evaluation = program.evaluator_with_cache(
        Box::new(ReadFlakyStore {
            inner: MemoryStore::new(),
            failures: 1,
            failed: 0,
        }),
        CachePolicy::new(0),
    );
    assert_eq!(
        evaluation
            .select(&["warm"])
            .map_err(|e| e.to_string())
            .unwrap()
            .to_string(),
        plain("warm")
    );
    let stats = evaluation.cache_stats();
    assert!(
        stats.evictions > 0 && stats.records_written > 0,
        "warm must spill: {stats:?}"
    );
    let first = evaluation.select(&["later"]);
    let error = match first {
        Err(EvaluationError::Infrastructure(error)) => error,
        other => panic!(
            "the first dependent selection must hit the transient read failure, got {other:?}"
        ),
    };
    assert_eq!(error.operation, "get");
    assert!(matches!(error.key, Some(CacheKey::Thunk(_))), "{error}");
    for _ in 0..2 {
        assert_eq!(
            evaluation
                .select(&["later"])
                .map_err(|e| e.to_string())
                .unwrap()
                .to_string(),
            plain("later"),
            "retry after the store recovered"
        );
    }
    assert_eq!(
        evaluation
            .select(&["count"])
            .map_err(|e| e.to_string())
            .unwrap()
            .to_string(),
        plain("count")
    );
    assert_eq!(
        evaluation
            .select(&["warm"])
            .map_err(|e| e.to_string())
            .unwrap()
            .to_string(),
        plain("warm")
    );
    let stats = evaluation.cache_stats();
    assert!(stats.records_reloaded > 0, "{stats:?}");
    assert_eq!(stats.resident_payload_bytes, 0, "{stats:?}");
    evaluation.close().unwrap();
}
