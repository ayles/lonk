use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore, MemoryArtifactStore};
use lonk::{Compiler, CompilerPolicy, Program, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Store(Arc<Mutex<MemoryArtifactStore>>);
impl ArtifactStore for Store {
    fn get(&mut self, key: ArtifactKey, max: usize) -> Result<Option<Vec<u8>>, ArtifactError> {
        self.0.lock().unwrap().get(key, max)
    }
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError> {
        self.0.lock().unwrap().put(key, bytes)
    }
}

#[test]
fn checked_graph_and_selected_value_survive_a_fresh_compiler() {
    let source = "{answer = 42; poison = 1 / 0}";
    let store = Store::default();
    let new = || {
        Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(store.clone()),
        )
        .unwrap()
    };
    let mut first = new();
    assert_eq!(first.evaluate(source, &["answer"]).unwrap(), Value::Int(42));
    assert_eq!(first.stats().checks, 1);
    let mut second = new();
    assert_eq!(
        second.evaluate(source, &["answer"]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(second.stats().checks, 0);
    assert_eq!(second.stats().artifact_hits, 1);
    let mut third = new();
    let program = third.prepare(source).unwrap();
    assert_eq!(program.evaluate(&["answer"]).unwrap(), Value::Int(42));
    assert!(
        program
            .evaluate(&["poison"])
            .unwrap_err()
            .message
            .contains("division")
    );
    assert_eq!(third.stats().checks, 0);
}

#[test]
fn memory_budget_evicts_without_changing_results() {
    let mut compiler = Compiler::new(CompilerPolicy {
        memory_bytes: 2048,
        artifact_bytes: 0,
    });
    for n in 0..30 {
        assert_eq!(
            compiler.evaluate(&n.to_string(), &[]).unwrap(),
            Value::Int(n)
        );
        assert!(compiler.stats().retained_bytes <= 2048);
    }
    assert!(compiler.stats().evictions > 0);
    compiler.clear_memory();
    assert_eq!(compiler.stats().retained_bytes, 0);
}

#[test]
fn changed_source_reuses_prelude_and_exact_repeat_skips_checks() {
    let mut compiler = Compiler::default();
    for source in ["40 + 2", "20 + 22", "40 + 2"] {
        compiler.prepare(source).unwrap();
    }
    assert_eq!(compiler.stats().prelude_checks, 1);
    assert_eq!(compiler.stats().checks, 2);
    assert_eq!(compiler.stats().program_hits, 1);
}

#[test]
fn host_namespaces_isolate_all_persistent_records() {
    let store = Store::default();
    let source = "let twice = fn(x: int) x * 2; { answer = twice(21); unused = 1 / 0 }";
    let mut first = Compiler::with_store(
        b"host-a",
        CompilerPolicy::default(),
        Box::new(store.clone()),
    )
    .unwrap();
    assert_eq!(first.evaluate(source, &["answer"]).unwrap(), Value::Int(42));
    let first_keys: std::collections::BTreeSet<_> =
        store.0.lock().unwrap().records.keys().copied().collect();
    let mut changed = Compiler::with_store(
        b"host-b",
        CompilerPolicy::default(),
        Box::new(store.clone()),
    )
    .unwrap();
    assert_eq!(
        changed.evaluate(source, &["answer"]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(changed.stats().checks, 1);
    assert_eq!(changed.stats().artifact_hits, 0);
    assert_eq!(changed.stats().computation_hits, 0);
    assert!(changed.stats().forced_nodes > 0);
    assert!(store.0.lock().unwrap().records.len() > first_keys.len());
    let mut same =
        Compiler::with_store(b"host-a", CompilerPolicy::default(), Box::new(store)).unwrap();
    assert_eq!(same.evaluate(source, &["answer"]).unwrap(), Value::Int(42));
    assert_eq!(same.stats().checks, 0);
    assert_eq!(same.stats().forced_nodes, 0);
    assert!(
        Compiler::with_store(
            b"",
            CompilerPolicy::default(),
            Box::new(MemoryArtifactStore::default())
        )
        .is_err()
    );
}
