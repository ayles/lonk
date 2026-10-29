use lonk::{ModuleSource, Program};
use std::collections::BTreeMap;
#[path = "support/module_manifest.rs"]
mod package_tree;

#[test]
fn module_values_types_functions_and_lazy_fields() {
    let modules = BTreeMap::from([
        ("types".into(), ModuleSource::new("{Item=type {n:int}}")),
        (
            "library".into(),
            ModuleSource::new(
                "let Item=types.Item; {make=fn(n:int)->Item { ({n=n}) }; poison=1/0}",
            )
            .with_dependency("types", "types"),
        ),
        (
            "main".into(),
            ModuleSource::new("lib.make(42)").with_dependency("lib", "library"),
        ),
        ("unrelated".into(), ModuleSource::new("invalid !!!")),
    ]);
    let program = Program::prepare_modules("main", &modules).unwrap();
    assert_eq!(program.evaluate(&[]).unwrap().to_string(), r#"{"n":42}"#);
    let broken = Program::prepare_modules("library", &modules).unwrap();
    let error = broken.evaluate(&["poison"]).unwrap_err();
    assert!(broken.render_diagnostic(&error).starts_with("library:1:"));
}

#[test]
fn module_boundaries_and_preparation_errors() {
    for (source, expected) in [
        ("dep=2;dep", "captured"),
        ("let n=0;42", "unused computation"),
        ("false+1", "operator"),
    ] {
        let modules = BTreeMap::from([
            ("dep".into(), ModuleSource::new("1")),
            (
                "main".into(),
                ModuleSource::new(source).with_dependency("dep", "dep"),
            ),
        ]);
        let error = Program::prepare_modules("main", &modules).unwrap_err();
        assert_eq!(error.module, "main");
        assert!(error.to_string().contains(expected), "{error}");
    }
    let cycle = BTreeMap::from([
        ("a".into(), ModuleSource::new("b").with_dependency("b", "b")),
        ("b".into(), ModuleSource::new("a").with_dependency("a", "a")),
    ]);
    assert!(
        Program::prepare_modules("a", &cycle)
            .unwrap_err()
            .to_string()
            .contains("cyclic module interfaces")
    );
    let hidden = BTreeMap::from([
        ("private".into(), ModuleSource::new("let secret=42; secret")),
        (
            "main".into(),
            ModuleSource::new("secret").with_dependency("dep", "private"),
        ),
    ]);
    assert!(
        Program::prepare_modules("main", &hidden)
            .unwrap_err()
            .to_string()
            .contains("unknown name `secret`")
    );
}

#[test]
fn package_tree_and_configuration_experiments() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modules");
    let sources = package_tree::load(&path).unwrap();
    let program = Program::prepare_modules("main", &sources).unwrap_or_else(|e| panic!("{e}"));
    for (path, expected) in [
        ("packages.original_inputs", r#"["cc-1","zlib-1"]"#),
        ("packages.overridden_inputs", r#"["cc-1","zlib-1"]"#),
        ("packages.rebuilt_inputs", r#"["cc-1","zlib-2"]"#),
        ("packages.cross_system", "\"aarch64-linux\""),
        ("packages.public", r#"{"name":"editor","version":"3"}"#),
        ("system.config.firewall", "[2222]"),
        ("system.original.packages", "[]"),
        ("system.errors", "[]"),
        ("system.duplicate_errors", "[\"duplicate service: sshd\"]"),
        ("system.last_hostname", "\"last\""),
        ("system.early_firewall", "[]"),
        ("fixpoint.original_inputs", "[\"zlib-1\"]"),
        ("fixpoint.overlaid_inputs", "[\"zlib-2\"]"),
    ] {
        assert_eq!(
            program
                .evaluate(&path.split('.').collect::<Vec<_>>())
                .unwrap_or_else(|e| panic!("{}", program.render_diagnostic(&e)))
                .to_string(),
            expected,
            "{path}"
        );
    }
}

#[test]
fn experiment_probes_capture_current_limits() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modules/probes");
    for (name, needle) in [
        ("remove-interface", "not known"),
        ("finite-else", "expected"),
        ("recursive-record", "unknown name"),
        ("named-function", "expected"),
    ] {
        let source = std::fs::read_to_string(root.join(format!("{name}.lonk"))).unwrap();
        let error = Program::prepare(&source).unwrap_err();
        assert!(error.message.contains(needle), "{name}: {error}");
    }
    // Dotted type paths and contextual fresh literals are accepted.
    for name in ["dotted-type", "optional-inference"] {
        let source = std::fs::read_to_string(root.join(format!("{name}.lonk"))).unwrap();
        Program::prepare(&source).unwrap_or_else(|e| panic!("{name}: {}", e.render(&source)));
    }
}
