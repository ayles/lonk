//! Regression tests for content-addressed evaluation reuse across edits
//!. A demanded scalar computation whose code and inputs are unchanged
//! must be served from the store after an edit elsewhere, forcing fewer nodes
//! than a cold run, while every outcome stays identical to a plain
//! `Program::prepare`/`evaluate`. Counters:
//! `computation_hits`, `computation_writes` and cumulative `forced_nodes`.

use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore, MemoryArtifactStore};
use lonk::{CompileError, Compiler, CompilerPolicy, CompilerStats, ModuleSource, Program, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

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

/// Evaluate in a fresh compiler sharing `store`, as a new process would, and
/// check the outcome against a plain evaluation of the same source.
fn run(store: &Store, source: &str, path: &[&str]) -> (Result<Value, String>, CompilerStats) {
    let mut compiler = Compiler::with_store(
        b"test-build",
        CompilerPolicy::default(),
        Box::new(store.clone()),
    )
    .unwrap();
    let actual = compiler.evaluate(source, path).map_err(|e| match e {
        CompileError::Language(d) => d.diagnostic.message,
        CompileError::Infrastructure(e) => panic!("infrastructure: {e}"),
    });
    let plain = Program::prepare(source)
        .map_err(|e| e.message)
        .and_then(|p| p.evaluate(path).map_err(|e| e.message));
    assert_eq!(
        actual, plain,
        "cached outcome differs from plain evaluation for {path:?}\n{source}"
    );
    (actual, compiler.stats().clone())
}

fn value(outcome: &Result<Value, String>) -> Value {
    outcome
        .clone()
        .unwrap_or_else(|e| panic!("unexpected failure: {e}"))
}

/// A nontrivial scalar workload: a loop, a recursive function and a closure,
/// all demanded through one field.
const WORK: &str =
    "let sum = fn(xs: [int]) -> int { let total = 0; for x in xs { total += x; }; total };
let fact = fn(n: int) -> int { if n == 0 { 1 } else { n * fact(n - 1) } };
let make = fn(k: int) -> fn(int) -> int { fn(n: int) n + k };
let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
let add = make(100);";

fn program(sibling: &str) -> String {
    format!("{WORK}\n{{answer = add(sum(data)) + fact(6); sibling = {sibling}}}")
}

#[test]
fn an_unrelated_sibling_edit_keeps_the_answer_cached() {
    let store = Store::default();
    let (cold, cold_stats) = run(&store, &program("1"), &["answer"]);
    assert_eq!(value(&cold), Value::Int(956));
    assert!(
        cold_stats.computation_writes >= 1,
        "scalar results are written"
    );
    let (warm, warm_stats) = run(&store, &program("2 + 2"), &["answer"]);
    assert_eq!(value(&warm), Value::Int(956));
    assert!(
        warm_stats.computation_hits >= 1,
        "the unchanged computation is a hit"
    );
    assert!(
        warm_stats.forced_nodes < cold_stats.forced_nodes,
        "warm forced {} nodes, cold forced {}",
        warm_stats.forced_nodes,
        cold_stats.forced_nodes
    );
}

#[test]
fn comments_and_whitespace_never_change_identities() {
    let store = Store::default();
    let (_, cold) = run(&store, &program("1"), &["answer"]);
    let spaced = program("1")
        .replace("let total = 0;", "let total = 0; // running total\n")
        .replace("fact(6)", "fact(  6  ) /* six */");
    let (warm, stats) = run(&store, &spaced, &["answer"]);
    assert_eq!(value(&warm), Value::Int(956));
    assert!(stats.computation_hits >= 1);
    assert!(stats.forced_nodes < cold.forced_nodes);
}

#[test]
fn a_changed_used_dependency_recomputes_and_an_ignored_poison_does_not() {
    let store = Store::default();
    let _ = run(&store, &program("1"), &["answer"]);
    let changed = program("1").replace("let add = make(100);", "let add = make(200);");
    let (value_after, _) = run(&store, &changed, &["answer"]);
    assert_eq!(
        value(&value_after),
        Value::Int(1056),
        "a changed captured input changes the result"
    );
    let (_, cold) = run(&store, &program("1 / 0"), &["answer"]);
    let (warm, stats) = run(&store, &program("2 / 0"), &["answer"]);
    assert_eq!(value(&warm), Value::Int(956));
    assert!(
        stats.computation_hits >= 1,
        "an edited poison sibling is never demanded"
    );
    assert!(stats.forced_nodes <= cold.forced_nodes);
}

#[test]
fn keys_are_built_without_forcing_arguments() {
    let store = Store::default();
    let source = "let ignore = fn(x: int, y: int) -> int { x }; {a = ignore(41, 1 / 0) + 1; b = 1}";
    let (first, _) = run(&store, source, &["a"]);
    assert_eq!(value(&first), Value::Int(42));
    let (second, stats) = run(&store, &source.replace("b = 1", "b = 2"), &["a"]);
    assert_eq!(
        value(&second),
        Value::Int(42),
        "a poisoned unused argument is never forced for a key"
    );
    assert!(stats.computation_hits >= 1);
}

#[test]
fn generic_substitutions_and_captures_keep_distinct_identities() {
    let store = Store::default();
    let source = "let len_of = fn<T>(xs: [T]) -> int { xs.len() }; let make = fn(k: int) -> fn(int) -> int { fn(n: int) n + k };
let add1 = make(1); let add2 = make(2);
{ints = len_of([1, 2]); strs = len_of([\"x\"]); one = add1(40); two = add2(40); sibling = 0}";
    for (path, expected) in [("ints", 2), ("strs", 1), ("one", 41), ("two", 42)] {
        assert_eq!(
            value(&run(&store, source, &[path]).0),
            Value::Int(expected),
            "{path}"
        );
    }
    let edited = source.replace("sibling = 0", "sibling = 1");
    for (path, expected) in [("ints", 2), ("strs", 1), ("one", 41), ("two", 42)] {
        let (outcome, stats) = run(&store, &edited, &[path]);
        assert_eq!(
            value(&outcome),
            Value::Int(expected),
            "{path} after the sibling edit"
        );
        assert!(stats.computation_hits >= 1, "{path} is a hit");
    }
}

#[test]
fn dynamic_selection_depends_on_the_computed_key() {
    let store = Store::default();
    // A computed object key needs lookup plus refinement; an int index is direct.
    let source = "let r = {x = 1; y = 2}; let key = \"y\"; let found = object.lookup(r, key);
let picked = match found { {value: int; ...} => found.value; else => 0; };
let i = 1 + 1; {a = picked + [10, 20, 30][i]; sibling = 0}";
    assert_eq!(value(&run(&store, source, &["a"]).0), Value::Int(32));
    let (warm, stats) = run(
        &store,
        &source.replace("sibling = 0", "sibling = 5"),
        &["a"],
    );
    assert_eq!(value(&warm), Value::Int(32));
    assert!(stats.computation_hits >= 1);
    let (changed, _) = run(
        &store,
        &source.replace("let i = 1 + 1;", "let i = 1 + 0;"),
        &["a"],
    );
    assert_eq!(
        value(&changed),
        Value::Int(22),
        "a changed index expression changes the selection"
    );
    let (other_key, _) = run(
        &store,
        &source.replace("let key = \"y\";", "let key = \"x\";"),
        &["a"],
    );
    assert_eq!(
        value(&other_key),
        Value::Int(31),
        "a changed key expression changes the lookup"
    );
}

#[test]
fn repeated_fresh_instances_converge_on_hits() {
    let store = Store::default();
    let (_, first) = run(&store, &program("1"), &["answer"]);
    let (_, second) = run(&store, &program("1"), &["answer"]);
    let (_, third) = run(&store, &program("3"), &["answer"]);
    assert!(second.forced_nodes <= first.forced_nodes);
    assert!(third.forced_nodes < first.forced_nodes);
    assert!(third.computation_hits >= 1);
}

#[test]
fn a_root_edit_reuses_computations_from_an_unchanged_dependency_module() {
    let store = Store::default();
    let mut modules: BTreeMap<String, ModuleSource> = BTreeMap::new();
    modules.insert(
        "lib".into(),
        ModuleSource::new(format!("{WORK}\n{{answer = add(sum(data)) + fact(6)}}")),
    );
    let root = |extra: &str| {
        ModuleSource::new(format!("lib.answer + {extra}")).with_dependency("lib", "lib")
    };
    let evaluate = |root_source: &str| {
        let mut set = modules.clone();
        set.insert("main".into(), root(root_source));
        let mut compiler = Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(store.clone()),
        )
        .unwrap();
        let actual = compiler.evaluate_modules("main", &set, &[]).unwrap();
        let plain = Program::prepare_modules("main", &set)
            .unwrap()
            .evaluate(&[])
            .unwrap();
        assert_eq!(actual, plain);
        (actual, compiler.stats().clone())
    };
    let (cold, cold_stats) = evaluate("0");
    assert_eq!(cold, Value::Int(956));
    let (warm, warm_stats) = evaluate("1");
    assert_eq!(warm, Value::Int(957));
    assert_eq!(warm_stats.module_checks, 1, "only the root is rechecked");
    assert!(
        warm_stats.computation_hits >= 1,
        "lib's scalar answer is reused"
    );
    assert!(warm_stats.forced_nodes < cold_stats.forced_nodes);
}

// ---------------------------------------------------------------------------
// Storage failures inside the fine computation path.
// ---------------------------------------------------------------------------

/// A store that records every requested and written key and can fail chosen
/// keys, so scalar computation records can be targeted without knowing the
/// internal key layout.
#[derive(Clone, Default)]
struct Traced {
    records: Arc<Mutex<MemoryArtifactStore>>,
    gets: Arc<Mutex<Vec<ArtifactKey>>>,
    puts: Arc<Mutex<Vec<ArtifactKey>>>,
    fail_get: Arc<Mutex<std::collections::BTreeSet<ArtifactKey>>>,
    fail_put: Arc<Mutex<std::collections::BTreeSet<ArtifactKey>>>,
}
impl ArtifactStore for Traced {
    fn get(&mut self, key: ArtifactKey, max: usize) -> Result<Option<Vec<u8>>, ArtifactError> {
        self.gets.lock().unwrap().push(key);
        if self.fail_get.lock().unwrap().contains(&key) {
            return Err(ArtifactError("simulated scalar read failure".into()));
        }
        self.records.lock().unwrap().get(key, max)
    }
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError> {
        self.puts.lock().unwrap().push(key);
        if self.fail_put.lock().unwrap().contains(&key) {
            return Err(ArtifactError("simulated scalar write failure".into()));
        }
        self.records.lock().unwrap().put(key, bytes)
    }
}
impl Traced {
    fn compiler(&self) -> Compiler {
        Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(self.clone()),
        )
        .unwrap()
    }
    fn remove(&self, key: ArtifactKey) -> Option<Vec<u8>> {
        self.records.lock().unwrap().records.remove(&key)
    }
    fn restore(&self, key: ArtifactKey, bytes: Vec<u8>) {
        self.records.lock().unwrap().records.insert(key, bytes);
    }
    fn gets_since(&self, from: usize) -> Vec<ArtifactKey> {
        self.gets.lock().unwrap()[from..].to_vec()
    }
    fn puts_since(&self, from: usize) -> Vec<ArtifactKey> {
        self.puts.lock().unwrap()[from..].to_vec()
    }
    fn get_count(&self) -> usize {
        self.gets.lock().unwrap().len()
    }
    fn put_count(&self) -> usize {
        self.puts.lock().unwrap().len()
    }
}

fn infrastructure_message(error: CompileError) -> String {
    match error {
        CompileError::Infrastructure(e) => e.to_string(),
        CompileError::Language(d) => panic!("expected an infrastructure error, got language: {d}"),
    }
}

#[test]
fn storage_failures_at_scalar_computations_are_infrastructure_errors_that_recover() {
    let store = Traced::default();
    let source = program("1");
    let path = &["answer"];

    // Reference runs: preparation alone writes the prepared artifacts; the
    // evaluation afterwards writes scalar computation records and, last, the
    // exact selected output.
    store.compiler().prepare(&source).unwrap();
    let prepared: std::collections::BTreeSet<ArtifactKey> =
        store.puts_since(0).into_iter().collect();
    let before = store.put_count();
    let mut reference = store.compiler();
    assert_eq!(reference.evaluate(&source, path).unwrap(), Value::Int(956));
    let reference_forced = reference.stats().forced_nodes;
    let written = store.puts_since(before);
    let output_key = *written.last().expect("the selected output is written last");
    let scalars: Vec<ArtifactKey> = written[..written.len() - 1]
        .iter()
        .copied()
        .filter(|k| !prepared.contains(k) && *k != output_key)
        .collect();
    assert!(
        !scalars.is_empty(),
        "the workload must persist scalar computations"
    );

    // A missing exact output forces the boundary and the scalar record hits.
    store.remove(output_key);
    let gets_before = store.get_count();
    let mut replay = store.compiler();
    assert_eq!(replay.evaluate(&source, path).unwrap(), Value::Int(956));
    assert!(
        replay.stats().computation_hits >= 1,
        "scalar records serve the replay"
    );
    assert!(replay.stats().forced_nodes < reference_forced);
    let target = store
        .gets_since(gets_before)
        .into_iter()
        .find(|k| scalars.contains(k))
        .expect("the replay requests a scalar record");

    // Read failure on that record: infrastructure error, then recovery on the
    // same compiler with the same value and no language error memoized.
    store.remove(output_key);
    store.fail_get.lock().unwrap().insert(target);
    let mut reader = store.compiler();
    let message = infrastructure_message(reader.evaluate(&source, path).unwrap_err());
    assert!(message.contains("scalar read failure"), "{message}");
    store.fail_get.lock().unwrap().clear();
    assert_eq!(reader.evaluate(&source, path).unwrap(), Value::Int(956));
    assert!(reader.stats().computation_hits >= 1);

    // Write failure on that record after it and the output are gone: the
    // computation succeeds but the store does not, which is reported; once
    // writes work the same compiler completes and persists again.
    store.remove(output_key);
    let saved: Vec<(ArtifactKey, Vec<u8>)> = scalars
        .iter()
        .filter_map(|k| store.remove(*k).map(|b| (*k, b)))
        .collect();
    assert!(!saved.is_empty());
    store.fail_put.lock().unwrap().insert(target);
    let mut writer = store.compiler();
    let message = infrastructure_message(writer.evaluate(&source, path).unwrap_err());
    assert!(message.contains("scalar write failure"), "{message}");
    store.fail_put.lock().unwrap().clear();
    assert_eq!(writer.evaluate(&source, path).unwrap(), Value::Int(956));
    assert!(writer.stats().computation_writes >= 1);
    for (key, bytes) in saved {
        store.restore(key, bytes);
    }

    // A corrupt scalar record is an infrastructure error; repairing it
    // restores hits on the same compiler, and removing it recomputes.
    store.remove(output_key);
    let original = store
        .remove(target)
        .expect("the targeted scalar record exists");
    let mut damaged = original.clone();
    let index = if damaged.len() > 72 {
        72
    } else {
        damaged.len() - 1
    };
    damaged[index] ^= 0xff;
    store.restore(target, damaged);
    let mut repaired = store.compiler();
    let message = infrastructure_message(repaired.evaluate(&source, path).unwrap_err());
    assert!(
        message.contains("checksum") || message.contains("artifact"),
        "{message}"
    );
    store.restore(target, original);
    assert_eq!(repaired.evaluate(&source, path).unwrap(), Value::Int(956));
    assert!(repaired.stats().computation_hits >= 1);
    store.remove(output_key);
    store.remove(target);
    let mut recomputed = store.compiler();
    assert_eq!(recomputed.evaluate(&source, path).unwrap(), Value::Int(956));
    assert!(
        recomputed.stats().computation_writes >= 1,
        "the missing record is recomputed and rewritten"
    );
}
