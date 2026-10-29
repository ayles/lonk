//! Regression tests for module-level checked-graph reuse.
//! With module-local graphs, a root edit rechecks the root only, a dependency
//! body edit with an unchanged interface rechecks that dependency only, an
//! interface change rechecks exactly the affected dependents, and a linked
//! program behaves like a whole-closure check: same values, same diagnostics
//! with their module origins, same generic behaviour across module
//! boundaries, and primitive methods unaffected by import names.
//!
//! Counters: `module_checks` counts modules
//! checked in this compiler, `module_hits` counts modules reused from memory
//! or the artifact store.

use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore, MemoryArtifactStore};
use lonk::{CompileError, Compiler, CompilerPolicy, CompilerStats, ModuleSource, Value};
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
impl Store {
    fn compiler(&self) -> Compiler {
        Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(self.clone()),
        )
        .unwrap()
    }
}

type Modules = BTreeMap<String, ModuleSource>;

fn modules(entries: &[(&str, &str, &[&str])]) -> Modules {
    // A dependency listed as `name` is bound under that same name.
    entries
        .iter()
        .map(|(name, source, deps)| {
            let mut module = ModuleSource::new(*source);
            for dep in deps.iter() {
                module = module.with_dependency(*dep, *dep);
            }
            (name.to_string(), module)
        })
        .collect()
}

fn with(set: &Modules, name: &str, source: &str) -> Modules {
    let mut edited = set.clone();
    let deps = edited[name].dependencies.clone();
    let mut module = ModuleSource::new(source);
    for (binding, target) in deps {
        module = module.with_dependency(binding, target);
    }
    edited.insert(name.to_string(), module);
    edited
}

/// Evaluate in a fresh compiler sharing `store`, as a new process would.
fn restart(
    store: &Store,
    entry: &str,
    set: &Modules,
) -> (Result<Value, CompileError>, CompilerStats) {
    let mut compiler = store.compiler();
    let value = compiler.evaluate_modules(entry, set, &[]);
    (value, compiler.stats().clone())
}

fn language(error: CompileError) -> lonk::ModuleDiagnostic {
    match error {
        CompileError::Language(d) => d,
        CompileError::Infrastructure(e) => panic!("unexpected infrastructure error: {e}"),
    }
}

const LIB: &str = "{value = 1; twice = fn(n: int) -> int { n * 2 }}";

#[test]
fn a_root_edit_rechecks_only_the_root() {
    let store = Store::default();
    let set = modules(&[
        ("lib", LIB, &[]),
        ("main", "lib.twice(lib.value)", &["lib"]),
    ]);
    let (value, stats) = restart(&store, "main", &set);
    assert_eq!(value.unwrap(), Value::Int(2));
    assert_eq!(stats.module_checks, 2, "cold: both modules are checked");
    let edited = with(&set, "main", "lib.twice(lib.value) + 1");
    let (value, stats) = restart(&store, "main", &edited);
    assert_eq!(value.unwrap(), Value::Int(3));
    assert_eq!(stats.module_checks, 1, "only the root is rechecked");
    assert_eq!(stats.module_hits, 1, "the dependency is reused");
}

#[test]
fn a_dependency_body_edit_with_the_same_interface_rechecks_only_that_dependency() {
    let store = Store::default();
    let set = modules(&[
        ("lib", LIB, &[]),
        ("main", "lib.twice(lib.value)", &["lib"]),
    ]);
    assert_eq!(restart(&store, "main", &set).0.unwrap(), Value::Int(2));
    let edited = with(
        &set,
        "lib",
        "{value = 5; twice = fn(n: int) -> int { n + n }}",
    );
    let (value, stats) = restart(&store, "main", &edited);
    assert_eq!(value.unwrap(), Value::Int(10));
    assert_eq!(
        stats.module_checks, 1,
        "the interface {{value: int; twice: fn(int) -> int}} is unchanged"
    );
    assert_eq!(
        stats.module_hits, 1,
        "the root is reused against the same interface"
    );
}

#[test]
fn an_interface_change_rechecks_exactly_the_affected_dependents() {
    let store = Store::default();
    let set = modules(&[
        ("a", "{v = 1}", &[]),
        ("b", "a.v + 1", &["a"]),
        ("c", "{w = 5}", &[]),
        ("main", "b + c.w", &["b", "c"]),
    ]);
    assert_eq!(restart(&store, "main", &set).0.unwrap(), Value::Int(7));
    let edited = with(&set, "a", "{v = 1; extra = true}");
    let (value, stats) = restart(&store, "main", &edited);
    assert_eq!(value.unwrap(), Value::Int(7));
    assert_eq!(stats.module_checks, 2, "a and its interface consumer b");
    assert_eq!(
        stats.module_hits, 2,
        "c does not depend on a, and b's interface (int) is unchanged so main is reused"
    );
}

#[test]
fn a_diamond_shares_one_instance_and_reuses_every_unchanged_module() {
    let store = Store::default();
    let set = modules(&[
        ("base", "{n = 1}", &[]),
        ("left", "base.n + 1", &["base"]),
        ("right", "base.n + 2", &["base"]),
        ("main", "left + right", &["left", "right"]),
    ]);
    let (value, stats) = restart(&store, "main", &set);
    assert_eq!(value.unwrap(), Value::Int(5));
    assert_eq!(
        stats.module_checks, 4,
        "base is checked once, not once per path"
    );
    let edited = with(&set, "main", "left * right");
    let (value, stats) = restart(&store, "main", &edited);
    assert_eq!(value.unwrap(), Value::Int(6));
    assert_eq!(stats.module_checks, 1);
    assert_eq!(stats.module_hits, 3);
}

#[test]
fn a_linked_program_matches_a_whole_closure_check() {
    let set = modules(&[
        (
            "lib",
            "{id = fn<T>(x: T) -> T { x }; strip = fn<R>(v: {x: int, ...R}) -> {...R} { v }; poison = 1 / 0}",
            &[],
        ),
        (
            "main",
            "{a = lib.id({x = 1; y = 2}).y; b = lib.strip({x = 1; y = 2}).y; c = lib.id(3); p = lib.poison}",
            &["lib"],
        ),
    ]);
    let plain = lonk::Program::prepare_modules("main", &set).unwrap();
    let store = Store::default();
    store.compiler().prepare_modules("main", &set).unwrap();
    let mut restarted = store.compiler();
    let linked = restarted.prepare_modules("main", &set).unwrap();
    assert_eq!(
        restarted.stats().module_checks,
        0,
        "everything is loaded and linked"
    );
    assert_eq!(plain.result_type(), linked.result_type());
    // Node counts are not semantic: import aliases may add a node per import.
    assert!(
        linked.node_count() <= plain.node_count() + 2,
        "linking adds at most one alias per import"
    );
    for path in [&["a"][..], &["b"], &["c"]] {
        assert_eq!(
            plain.evaluate(path).unwrap(),
            linked.evaluate(path).unwrap(),
            "{path:?}"
        );
    }
    let expected = plain.evaluate(&["p"]).unwrap_err();
    let actual = linked.evaluate(&["p"]).unwrap_err();
    assert_eq!(expected, actual, "the poisoned field fails identically");
    assert_eq!(
        plain.render_diagnostic(&expected),
        linked.render_diagnostic(&actual),
        "with the same module origin"
    );
}

#[test]
fn generic_interfaces_cross_module_boundaries_after_a_root_edit() {
    let store = Store::default();
    let lib = "{id = fn<T>(x: T) -> T { x }; pair = fn<T>(a: T, b: T) -> [T] { [a, b] }; strip = fn<R>(v: {x: int, ...R}) -> {...R} { v }; Box = type <T> {item: T}}";
    let set = modules(&[("lib", lib, &[]), ("main", "lib.id(1)", &["lib"])]);
    assert_eq!(restart(&store, "main", &set).0.unwrap(), Value::Int(1));
    // Every use below checks the root against a cached, foreign-id interface.
    let cases: &[(&str, Value)] = &[
        ("lib.id(\"s\").len()", Value::Int(1)),
        ("lib.pair(1, 2)[1]", Value::Int(2)),
        ("lib.strip({x = 1; y = 7}).y", Value::Int(7)),
        ("let b: lib.Box<int> = {item = 4}; b.item", Value::Int(4)),
        (
            "let grow = fn<R>(v: {x: int, ...R}) -> {...R} { lib.strip(v) }; grow({x = 1; z = 9}).z",
            Value::Int(9),
        ),
    ];
    for (source, expected) in cases {
        let (value, stats) = restart(&store, "main", &with(&set, "main", source));
        assert_eq!(value.unwrap(), *expected, "{source}");
        assert_eq!(stats.module_checks, 1, "{source}: only the root is checked");
    }
    // Inference conflicts against an imported generic must be diagnosed, not
    // looked up in a registry that does not know the foreign parameter.
    let (value, _) = restart(&store, "main", &with(&set, "main", "lib.pair(1, \"s\")"));
    let error = language(value.unwrap_err());
    assert_eq!(error.module, "main");
    assert!(
        error
            .diagnostic
            .message
            .contains("conflicting inference for `T`"),
        "{error}"
    );
    // A captured-row rule violation against an imported row generic too.
    let (value, _) = restart(
        &store,
        "main",
        &with(
            &set,
            "main",
            "let f = fn<R>(v: {x: int, ...R}) -> {x: int, ...R} { let w = lib.strip(v); {x = 1; ...w} }; f({x = 2; y = 3}).y",
        ),
    );
    assert_eq!(value.unwrap(), Value::Int(3));
}

#[test]
fn diagnostics_in_cached_dependencies_keep_their_origin() {
    let store = Store::default();
    let set = modules(&[
        ("lib", "{value = 1 / 0}", &[]),
        ("main", "lib.value", &["lib"]),
    ]);
    let first = language(restart(&store, "main", &set).0.unwrap_err());
    assert_eq!(first.module, "lib");
    assert!(first.diagnostic.message.contains("division"), "{first}");
    let (value, stats) = restart(&store, "main", &with(&set, "main", "lib.value + 1"));
    let again = language(value.unwrap_err());
    assert_eq!(again.module, "lib");
    assert_eq!(
        again.diagnostic.span, first.diagnostic.span,
        "the span is in lib's source, unchanged"
    );
    assert_eq!(stats.module_checks, 1);
    // A root type error against cached dependencies names the root.
    let (value, stats) = restart(&store, "main", &with(&set, "main", "lib.value + true"));
    let root = language(value.unwrap_err());
    assert_eq!(root.module, "main");
    assert!(
        root.diagnostic.message.contains("does not accept"),
        "{root}"
    );
    assert_eq!(stats.module_hits, 1);
}

#[test]
fn import_names_never_change_primitive_method_dispatch() {
    let store = Store::default();
    let set = modules(&[
        ("array", "{len = fn(x: int) -> int { 99 }}", &[]),
        ("object", "{keys = fn(x: int) -> int { 7 }}", &[]),
        (
            "main",
            "let xs = [1, 2, 3]; xs.len() + \"ab\".len() + array.len(0) + object.keys(0)",
            &["array", "object"],
        ),
    ]);
    assert_eq!(
        restart(&store, "main", &set).0.unwrap(),
        Value::Int(3 + 2 + 99 + 7)
    );
    let (value, stats) = restart(
        &store,
        "main",
        &with(&set, "main", "[1].len() + array.len(0)"),
    );
    assert_eq!(value.unwrap(), Value::Int(100));
    assert_eq!(stats.module_checks, 1);
}

#[test]
fn value_hits_and_graph_hits_touch_no_module_counters() {
    let store = Store::default();
    let set = modules(&[("lib", LIB, &[]), ("main", "lib.value", &["lib"])]);
    assert_eq!(restart(&store, "main", &set).0.unwrap(), Value::Int(1));
    let (value, stats) = restart(&store, "main", &set);
    assert_eq!(value.unwrap(), Value::Int(1));
    assert_eq!(
        stats.module_checks + stats.module_hits,
        0,
        "a selected-value hit never looks at modules"
    );
    let mut compiler = store.compiler();
    compiler.prepare_modules("main", &set).unwrap();
    assert_eq!(compiler.stats().module_checks, 0);
    assert_eq!(
        compiler.stats().module_hits,
        2,
        "both modules load from the store"
    );
    assert_eq!(
        compiler.stats().prelude_checks,
        0,
        "nothing needed the prelude"
    );
}
