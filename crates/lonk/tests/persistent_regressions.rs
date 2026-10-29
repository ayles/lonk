//! Regression tests for the persistent preparation cache.
//! Every key must change when any input that can change the result changes:
//! source bytes, module names, dependency binding names and targets, the entry
//! module and the selection path. Diagnostics must keep their module origin
//! across a restart, and every storage problem must surface as an
//! infrastructure error that is never memoized as a language result.

use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore, MemoryArtifactStore};
use lonk::{CompileError, Compiler, CompilerPolicy, ModuleSource, Value};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// A shared in-memory store that simulates process restarts and injects faults.
#[derive(Clone, Default)]
struct Store {
    records: Arc<Mutex<MemoryArtifactStore>>,
    fail_get: Arc<AtomicBool>,
    fail_put: Arc<AtomicBool>,
    puts: Arc<AtomicUsize>,
}

impl ArtifactStore for Store {
    fn get(&mut self, key: ArtifactKey, max: usize) -> Result<Option<Vec<u8>>, ArtifactError> {
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(ArtifactError("simulated read failure".into()));
        }
        self.records.lock().unwrap().get(key, max)
    }
    fn put(&mut self, key: ArtifactKey, bytes: &[u8]) -> Result<(), ArtifactError> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err(ArtifactError("simulated write failure".into()));
        }
        self.puts.fetch_add(1, Ordering::SeqCst);
        self.records.lock().unwrap().put(key, bytes)
    }
}

impl Store {
    fn compiler(&self) -> Compiler {
        Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(self.clone()),
        )
        .unwrap()
    }
    fn compiler_with(&self, policy: CompilerPolicy) -> Compiler {
        Compiler::with_store(b"test-build", policy, Box::new(self.clone())).unwrap()
    }
    fn corrupt(&self, edit: impl Fn(&mut Vec<u8>)) {
        for bytes in self.records.lock().unwrap().records.values_mut() {
            edit(bytes);
        }
    }
    fn clear(&self) {
        self.records.lock().unwrap().records.clear();
    }
    fn count(&self) -> usize {
        self.records.lock().unwrap().records.len()
    }
}

fn modules(entries: &[(&str, &str, &[(&str, &str)])]) -> BTreeMap<String, ModuleSource> {
    entries
        .iter()
        .map(|(name, source, deps)| {
            let mut module = ModuleSource::new(*source);
            for (binding, target) in deps.iter() {
                module = module.with_dependency(*binding, *target);
            }
            (name.to_string(), module)
        })
        .collect()
}

fn language(error: CompileError) -> lonk::ModuleDiagnostic {
    match error {
        CompileError::Language(diagnostic) => diagnostic,
        CompileError::Infrastructure(e) => {
            panic!("expected a language error, got infrastructure: {e}")
        }
    }
}

fn infrastructure(error: CompileError) -> String {
    match error {
        CompileError::Infrastructure(e) => e.to_string(),
        CompileError::Language(d) => panic!("expected an infrastructure error, got language: {d}"),
    }
}

#[test]
fn changed_module_source_is_never_served_from_a_stale_artifact() {
    let store = Store::default();
    let old = modules(&[
        ("lib", "{value = 1}", &[]),
        ("main", "lib.value + 1", &[("lib", "lib")]),
    ]);
    let new = modules(&[
        ("lib", "{value = 10}", &[]),
        ("main", "lib.value + 1", &[("lib", "lib")]),
    ]);
    assert_eq!(
        store
            .compiler()
            .evaluate_modules("main", &old, &[])
            .unwrap(),
        Value::Int(2)
    );
    let mut second = store.compiler();
    assert_eq!(
        second.evaluate_modules("main", &new, &[]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        second.stats().checks,
        1,
        "a changed dependency source must be rechecked"
    );
    let mut third = store.compiler();
    assert_eq!(
        third.evaluate_modules("main", &old, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        third.stats().checks,
        0,
        "the unchanged input set is a hit again"
    );
}

#[test]
fn binding_target_and_binding_name_are_part_of_the_key() {
    let store = Store::default();
    let to_a = modules(&[
        ("libA", "{value = 1}", &[]),
        ("libB", "{value = 2}", &[]),
        ("main", "lib.value + 1", &[("lib", "libA")]),
    ]);
    let to_b = modules(&[
        ("libA", "{value = 1}", &[]),
        ("libB", "{value = 2}", &[]),
        ("main", "lib.value + 1", &[("lib", "libB")]),
    ]);
    assert_eq!(
        store
            .compiler()
            .evaluate_modules("main", &to_a, &[])
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        store
            .compiler()
            .evaluate_modules("main", &to_b, &[])
            .unwrap(),
        Value::Int(3)
    );
    let mut again = store.compiler();
    assert_eq!(
        again.evaluate_modules("main", &to_a, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(again.stats().checks, 0);
    // Same sources, same result, one extra binding: the binding map is part of
    // the key, so this is a miss even though every module source is unchanged.
    let extended = modules(&[
        ("libA", "{value = 1}", &[]),
        ("libB", "{value = 2}", &[]),
        (
            "main",
            "lib.value + 1",
            &[("lib", "libA"), ("other", "libB")],
        ),
    ]);
    let mut extra = store.compiler();
    assert_eq!(
        extra.evaluate_modules("main", &extended, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        extra.stats().checks,
        1,
        "a changed binding map is never a hit"
    );
}

#[test]
fn entry_module_is_part_of_the_key() {
    let store = Store::default();
    let set = modules(&[("a", "1", &[]), ("b", "2", &[])]);
    assert_eq!(
        store.compiler().evaluate_modules("a", &set, &[]).unwrap(),
        Value::Int(1)
    );
    let mut second = store.compiler();
    assert_eq!(
        second.evaluate_modules("b", &set, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(second.stats().value_hits, 0);
    assert_eq!(second.stats().checks, 1);
}

#[test]
fn selection_path_is_part_of_the_value_key_and_graph_hits_skip_checking() {
    let store = Store::default();
    let source = "{x = 1; y = 2; poison = 1 / 0}";
    assert_eq!(
        store.compiler().evaluate(source, &["x"]).unwrap(),
        Value::Int(1)
    );
    let mut second = store.compiler();
    assert_eq!(second.evaluate(source, &["y"]).unwrap(), Value::Int(2));
    assert_eq!(
        second.stats().checks,
        0,
        "the checked graph is reused for a new path"
    );
    assert_eq!(second.stats().parses, 0);
    assert!(
        second.stats().artifact_hits >= 1,
        "the prelude and unit artifacts are loaded instead of checking"
    );
    assert_eq!(second.stats().artifact_writes, 1, "the new value is saved");
    let mut third = store.compiler();
    assert_eq!(third.evaluate(source, &["y"]).unwrap(), Value::Int(2));
    assert_eq!(
        third.stats().artifact_hits,
        1,
        "a value hit does not decode the graph"
    );
    assert_eq!(third.stats().checks + third.stats().parses, 0);
}

#[test]
fn changed_source_with_the_same_path_recomputes() {
    let store = Store::default();
    assert_eq!(
        store.compiler().evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1)
    );
    let mut second = store.compiler();
    assert_eq!(second.evaluate("{x = 5}", &["x"]).unwrap(), Value::Int(5));
    assert_eq!(second.stats().checks, 1);
}

#[test]
fn module_diagnostic_origins_survive_a_restart() {
    let store = Store::default();
    let broken = modules(&[("bad", "1 + true", &[]), ("main", "bad", &[("bad", "bad")])]);
    let first = language(
        store
            .compiler()
            .prepare_modules("main", &broken)
            .unwrap_err(),
    );
    assert_eq!(first.module, "bad");
    assert!(
        first.diagnostic.message.contains("does not accept"),
        "{first}"
    );
    let mut second = store.compiler();
    let again = language(second.prepare_modules("main", &broken).unwrap_err());
    assert_eq!(again.module, "bad");
    assert_eq!(
        again.to_string(),
        first.to_string(),
        "the rendered diagnostic is identical after reload"
    );
    assert_eq!(
        second.stats().checks,
        0,
        "a memoized diagnostic is an artifact hit"
    );
    assert!(second.stats().artifact_hits >= 1);

    let poisoned = modules(&[
        ("dep", "{poison = 1 / 0}", &[]),
        ("main", "dep.poison", &[("dep", "dep")]),
    ]);
    let first = language(
        store
            .compiler()
            .evaluate_modules("main", &poisoned, &[])
            .unwrap_err(),
    );
    assert_eq!(
        first.module, "dep",
        "an evaluation failure names the module that holds the failing expression"
    );
    assert!(first.diagnostic.message.contains("division"), "{first}");
    let mut second = store.compiler();
    let again = language(second.evaluate_modules("main", &poisoned, &[]).unwrap_err());
    assert_eq!(again.to_string(), first.to_string());
    assert_eq!(second.stats().checks, 0);
}

#[test]
fn single_source_failures_keep_the_source_origin() {
    let store = Store::default();
    let first = language(
        store
            .compiler()
            .evaluate("{a = 1 / 0}", &["a"])
            .unwrap_err(),
    );
    assert_eq!(first.module, "<source>");
    assert!(first.diagnostic.message.contains("division"), "{first}");
    let mut second = store.compiler();
    assert_eq!(
        language(second.evaluate("{a = 1 / 0}", &["a"]).unwrap_err()).to_string(),
        first.to_string()
    );
    assert_eq!(second.stats().checks, 0);
    let prepare = language(store.compiler().prepare("1 + true").unwrap_err());
    assert_eq!(prepare.module, "<source>");
    let mut third = store.compiler();
    assert_eq!(
        language(third.prepare("1 + true").unwrap_err()).to_string(),
        prepare.to_string()
    );
    assert_eq!(third.stats().checks, 0);
}

#[test]
fn missing_artifacts_are_misses_not_errors() {
    let store = Store::default();
    assert_eq!(
        store.compiler().evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1)
    );
    assert!(store.count() >= 2, "graph and value artifacts were written");
    store.clear();
    let mut second = store.compiler();
    assert_eq!(second.evaluate("{x = 1}", &["x"]).unwrap(), Value::Int(1));
    assert_eq!(second.stats().checks, 1);
    assert_eq!(second.stats().artifact_hits, 0);
}

#[test]
fn storage_failures_are_infrastructure_errors_and_never_memoized() {
    let store = Store::default();
    let mut compiler = store.compiler();
    store.fail_put.store(true, Ordering::SeqCst);
    let message = infrastructure(compiler.evaluate("{x = 1}", &["x"]).unwrap_err());
    assert!(message.contains("write failure"), "{message}");
    store.fail_put.store(false, Ordering::SeqCst);
    assert_eq!(
        compiler.evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1),
        "the same compiler recovers once the store does"
    );

    store.fail_get.store(true, Ordering::SeqCst);
    let mut reader = store.compiler();
    let message = infrastructure(reader.evaluate("{x = 1}", &["x"]).unwrap_err());
    assert!(message.contains("read failure"), "{message}");
    store.fail_get.store(false, Ordering::SeqCst);
    assert_eq!(reader.evaluate("{x = 1}", &["x"]).unwrap(), Value::Int(1));
    assert_eq!(
        reader.stats().checks,
        0,
        "the good artifacts are used after recovery"
    );
}

#[test]
fn corrupt_artifacts_are_infrastructure_errors_and_recompute_after_repair() {
    let source = "{x = 1; poison = 1 / 0}";
    let cases: Vec<(&str, Box<dyn Fn(&mut Vec<u8>)>, &str)> = vec![
        (
            "payload byte flipped",
            Box::new(|b: &mut Vec<u8>| {
                if b.len() > 72 {
                    b[72] ^= 0xff;
                }
            }),
            "checksum",
        ),
        (
            "magic damaged",
            Box::new(|b: &mut Vec<u8>| {
                b[0] ^= 0xff;
            }),
            "header",
        ),
        (
            "identity damaged",
            Box::new(|b: &mut Vec<u8>| {
                b[8] ^= 0xff;
            }),
            "header",
        ),
        (
            "truncated",
            Box::new(|b: &mut Vec<u8>| {
                b.truncate(10);
            }),
            "header",
        ),
        (
            "trailing garbage",
            Box::new(|b: &mut Vec<u8>| {
                b.push(0);
            }),
            "checksum",
        ),
    ];
    for (name, edit, needle) in cases {
        let store = Store::default();
        assert_eq!(
            store.compiler().evaluate(source, &["x"]).unwrap(),
            Value::Int(1)
        );
        store.corrupt(edit);
        let mut damaged = store.compiler();
        let message = infrastructure(damaged.evaluate(source, &["x"]).unwrap_err());
        assert!(message.contains(needle), "{name}: {message}");
        assert_eq!(
            damaged.stats().checks,
            0,
            "{name}: nothing was recomputed behind a corrupt record"
        );
        store.clear();
        assert_eq!(
            damaged.evaluate(source, &["x"]).unwrap(),
            Value::Int(1),
            "{name}: a cleared store recomputes"
        );
        assert_eq!(damaged.stats().checks, 1, "{name}");
    }
}

#[test]
fn swapped_records_fail_the_identity_check() {
    let store = Store::default();
    assert_eq!(
        store.compiler().evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1)
    );
    {
        // Rotate every record one key forward so each sits under a wrong key.
        let mut guard = store.records.lock().unwrap();
        let keys: Vec<ArtifactKey> = guard.records.keys().copied().collect();
        assert!(keys.len() >= 2);
        let values: Vec<Vec<u8>> = keys.iter().map(|k| guard.records[k].clone()).collect();
        for (i, key) in keys.iter().enumerate() {
            guard
                .records
                .insert(*key, values[(i + 1) % values.len()].clone());
        }
    }
    let message = infrastructure(store.compiler().evaluate("{x = 1}", &["x"]).unwrap_err());
    assert!(
        message.contains("header") || message.contains("identity"),
        "{message}"
    );
}

#[test]
fn oversized_artifacts_are_skipped_without_errors() {
    // Below the 72-byte envelope nothing can ever be saved or loaded.
    let store = Store::default();
    let tiny = CompilerPolicy {
        artifact_bytes: 1,
        ..CompilerPolicy::default()
    };
    let mut small = store.compiler_with(tiny);
    assert_eq!(small.evaluate("{x = 1}", &["x"]).unwrap(), Value::Int(1));
    assert_eq!(small.stats().artifact_writes, 0);
    assert_eq!(store.puts.load(Ordering::SeqCst), 0);
    let mut again = store.compiler_with(tiny);
    assert_eq!(again.evaluate("{x = 1}", &["x"]).unwrap(), Value::Int(1));
    assert_eq!(again.stats().checks, 1);

    // A cap that admits the scalar value but not the checked graph: the value
    // is reused across a restart, a new path has to recheck.
    let store = Store::default();
    let fields: Vec<String> = (0..64).map(|i| format!("f{i} = {i}")).collect();
    let source = format!("{{{}}}", fields.join("; "));
    let cap = CompilerPolicy {
        artifact_bytes: 200,
        ..CompilerPolicy::default()
    };
    let mut first = store.compiler_with(cap);
    assert_eq!(first.evaluate(&source, &["f1"]).unwrap(), Value::Int(1));
    assert_eq!(
        first.stats().artifact_writes,
        1,
        "only the scalar value fits"
    );
    let mut second = store.compiler_with(cap);
    assert_eq!(second.evaluate(&source, &["f1"]).unwrap(), Value::Int(1));
    assert_eq!(second.stats().checks, 0, "the value artifact is reused");
    let mut third = store.compiler_with(cap);
    assert_eq!(third.evaluate(&source, &["f2"]).unwrap(), Value::Int(2));
    assert_eq!(
        third.stats().checks,
        1,
        "no graph artifact exists, so a new path rechecks"
    );
}

#[test]
fn repeated_runs_never_rewrite_an_existing_record() {
    let store = Store::default();
    assert_eq!(
        store.compiler().evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1)
    );
    let written = store.puts.load(Ordering::SeqCst);
    assert!(written >= 2);
    assert_eq!(
        store.compiler().evaluate("{x = 1}", &["x"]).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        store.puts.load(Ordering::SeqCst),
        written,
        "a restart with hits writes nothing"
    );
}

#[test]
fn zero_memory_budget_keeps_results_and_counters_honest() {
    let mut compiler = Compiler::new(CompilerPolicy {
        memory_bytes: 0,
        artifact_bytes: 0,
    });
    for _ in 0..2 {
        assert_eq!(compiler.evaluate("{x = 1}", &["x"]).unwrap(), Value::Int(1));
    }
    assert_eq!(compiler.stats().retained_bytes, 0);
    assert_eq!(
        compiler.stats().checks,
        2,
        "nothing is retained, so every run checks"
    );
    assert_eq!(
        compiler.stats().prelude_checks,
        2,
        "the prelude is released with a zero budget"
    );
    assert_eq!(
        compiler.stats().program_hits + compiler.stats().value_hits,
        0
    );
}
