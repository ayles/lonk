//! Behavioral regression tests for host-supplied modules.
//! A module is a source text plus explicit binding-to-module dependencies chosen by the host; it
//! evaluates to one lazy value. Dependency bindings are immutable, copies are ordinary values,
//! every module compiles once in one graph, and diagnostics name the module they come from.

use lonk::{ModuleSource, Program};
use std::collections::BTreeMap;

struct Module {
    name: &'static str,
    source: &'static str,
    deps: &'static [(&'static str, &'static str)],
}

enum Expect {
    Value(&'static [&'static str], &'static str),
    /// Module name to blame (empty: any) and a needle in the rendered diagnostic.
    Reject(&'static str, &'static str),
    EvalError(&'static [&'static str], &'static str),
}
use Expect::*;

struct Case {
    id: &'static str,
    entry: &'static str,
    modules: &'static [Module],
    expect: Expect,
}

fn build(modules: &[Module]) -> BTreeMap<String, ModuleSource> {
    modules
        .iter()
        .map(|m| {
            let mut source = ModuleSource::new(m.source);
            for (binding, target) in m.deps {
                source = source.with_dependency(*binding, *target);
            }
            (m.name.to_owned(), source)
        })
        .collect()
}

fn check(case: &Case) -> Result<(), String> {
    let program = Program::prepare_modules(case.entry, &build(case.modules));
    match (&case.expect, program) {
        (Reject(module, needle), Err(e)) => {
            let text = e.to_string();
            if (module.is_empty() || e.module == *module) && text.contains(needle) {
                Ok(())
            } else {
                Err(format!("rejected differently: module {}: {text}", e.module))
            }
        }
        (Reject(..), Ok(p)) => Err(format!(
            "accepted; type {}; value {:?}",
            p.result_type(),
            p.evaluate(&[]).map(|v| v.to_string())
        )),
        (_, Err(e)) => Err(format!("unexpectedly rejected: {e}")),
        (Value(path, want), Ok(p)) => match p.evaluate(path) {
            Ok(v) if v.to_string() == *want => Ok(()),
            Ok(v) => Err(format!("value {v} but wanted {want}")),
            Err(e) => Err(format!("evaluation failed: {}", p.render_diagnostic(&e))),
        },
        (EvalError(path, needle), Ok(p)) => match p.evaluate(path) {
            Ok(v) => Err(format!(
                "evaluated to {v} but expected an error containing {needle:?}"
            )),
            Err(e) if p.render_diagnostic(&e).contains(needle) => Ok(()),
            Err(e) => Err(format!("failed differently: {}", p.render_diagnostic(&e))),
        },
    }
}

fn run_table(cases: &[Case]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| check(c).err().map(|e| format!("{}: {e}", c.id)))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

const LINKING: &[Case] = &[
    Case {
        id: "L1 exports stay lazy",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1; poison = 1 / 0 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep.a",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Value(&[], "1"),
    },
    Case {
        id: "L2 an error in a dependency is blamed on the dependency",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "let x = 1; x + \"s\"",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("dep", "operator"),
    },
    Case {
        id: "L3 unknown module in the manifest",
        entry: "main",
        modules: &[Module {
            name: "main",
            source: "d",
            deps: &[("d", "nope")],
        }],
        expect: Reject("main", "unknown host-supplied module"),
    },
    Case {
        id: "L4 three-module cycle",
        entry: "a",
        modules: &[
            Module {
                name: "a",
                source: "b",
                deps: &[("b", "b")],
            },
            Module {
                name: "b",
                source: "c",
                deps: &[("c", "c")],
            },
            Module {
                name: "c",
                source: "a",
                deps: &[("a", "a")],
            },
        ],
        expect: Reject("", "cycle"),
    },
    Case {
        id: "L5 an unrelated module is neither parsed nor checked",
        entry: "main",
        modules: &[
            Module {
                name: "unrelated",
                source: "1 + \"s\"",
                deps: &[],
            },
            Module {
                name: "main",
                source: "42",
                deps: &[],
            },
        ],
        expect: Value(&[], "42"),
    },
    Case {
        id: "L6 a module must produce a value",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("dep", ""),
    },
    Case {
        id: "L7 the entry must exist",
        entry: "missing",
        modules: &[Module {
            name: "main",
            source: "1",
            deps: &[],
        }],
        expect: Reject("", "unknown host-supplied module"),
    },
    Case {
        id: "L8 a shared dependency gives every dependent the same value",
        entry: "main",
        modules: &[
            Module {
                name: "a",
                source: "{ v = [1, 2] }",
                deps: &[],
            },
            Module {
                name: "b",
                source: "a.v",
                deps: &[("a", "a")],
            },
            Module {
                name: "c",
                source: "a.v",
                deps: &[("a", "a")],
            },
            Module {
                name: "main",
                source: "[b, c]",
                deps: &[("b", "b"), ("c", "c")],
            },
        ],
        expect: Value(&[], "[[1,2],[1,2]]"),
    },
    Case {
        id: "L9 a binding must be an identifier",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "1",
                deps: &[],
            },
            Module {
                name: "main",
                source: "1",
                deps: &[("not valid", "dep")],
            },
        ],
        expect: Reject("main", "invalid module binding"),
    },
    Case {
        id: "L10 a binding must not be a keyword",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "1",
                deps: &[],
            },
            Module {
                name: "main",
                source: "1",
                deps: &[("let", "dep")],
            },
        ],
        expect: Reject("main", "invalid module binding"),
    },
    Case {
        id: "L11 an unused dependency binding is not an error",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "1",
                deps: &[],
            },
            Module {
                name: "main",
                source: "2",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Value(&[], "2"),
    },
];

const IMMUTABILITY: &[Case] = &[
    Case {
        id: "I1 writing a field of a dependency",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep.a = 2; dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("main", ""),
    },
    Case {
        id: "I2 borrowing a dependency",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let f = fn(v: &{ a: int }) { v.a += 1; }; f(&dep); dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("main", ""),
    },
    Case {
        id: "I3 a copy is an ordinary value",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let mine = dep; mine.a = 2; [mine.a, dep.a]",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Value(&[], "[2,1]"),
    },
    Case {
        id: "I4 shadowing a dependency binding",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let dep = 5; dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Value(&[], "5"),
    },
    Case {
        id: "I5 a writing method on a dependency's object",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ counter = { n = 0; bump = fn(&self) { self.n += 1; }; } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep.counter.bump(); dep.counter.n",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("main", ""),
    },
    Case {
        id: "I5b works on a copy",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ counter = { n = 0; bump = fn(&self) { self.n += 1; }; } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let c = dep.counter; c.bump(); [c.n, dep.counter.n]",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Value(&[], "[1,0]"),
    },
    Case {
        id: "I6 a host binding named like a prelude namespace cannot change builtin members",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "{ a = 1 }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "[1].len()",
                deps: &[("array", "dep")],
            },
        ],
        expect: Value(&[], "1"),
    },
];

const EXPORTS: &[Case] = &[
    Case {
        id: "E1 a type export checks fresh literals exactly",
        entry: "main",
        modules: &[
            Module {
                name: "types",
                source: "{ P = type { name: str } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let P = types.P; let p: P = { name = \"x\"; extra = 1 }; p",
                deps: &[("types", "types")],
            },
        ],
        expect: Reject("main", ""),
    },
    Case {
        id: "E1b a dotted path as a type annotation",
        entry: "main",
        modules: &[
            Module {
                name: "types",
                source: "{ P = type { name: str } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let p: types.P = { name = \"x\" }; p",
                deps: &[("types", "types")],
            },
        ],
        expect: Value(&[], r#"{"name":"x"}"#),
    },
    Case {
        id: "E2 a generic export instantiates at use",
        entry: "main",
        modules: &[
            Module {
                name: "lib",
                source: "{ id = fn<T>(x: T) -> T { x } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "[lib.id<int>(1), lib.id(\"s\").len()]",
                deps: &[("lib", "lib")],
            },
        ],
        expect: Value(&[], "[1,1]"),
    },
    Case {
        id: "E3 defaults travel with the export",
        entry: "main",
        modules: &[
            Module {
                name: "lib",
                source: "{ mk = fn(n: int = 7) -> int { n } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "[lib.mk(), lib.mk(1)]",
                deps: &[("lib", "lib")],
            },
        ],
        expect: Value(&[], "[7,1]"),
    },
    Case {
        id: "E4 row preservation through an exported helper",
        entry: "main",
        modules: &[
            Module {
                name: "lib",
                source: "{ make = fn<R>(v: { n: int; ...R }) -> { n: int; ...R } { ({ ...v; n = v.n + 1 }) } }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let v = lib.make({ n = 1; tag = \"t\" }); [v.tag, v.n.to_str()]",
                deps: &[("lib", "lib")],
            },
        ],
        expect: Value(&[], r#"["t","2"]"#),
    },
    Case {
        id: "E5 a variant type export is matched exhaustively",
        entry: "main",
        modules: &[
            Module {
                name: "types",
                source: "{ Mode = type (\"a\" | \"b\") }",
                deps: &[],
            },
            Module {
                name: "main",
                source: "let Mode = types.Mode; let m: Mode = \"a\"; match m { \"a\" => 1; \"b\" => 2; }",
                deps: &[("types", "types")],
            },
        ],
        expect: Value(&[], "1"),
    },
    Case {
        id: "E6 a module value of unresolved type is rejected",
        entry: "main",
        modules: &[
            Module {
                name: "dep",
                source: "[]",
                deps: &[],
            },
            Module {
                name: "main",
                source: "dep",
                deps: &[("dep", "dep")],
            },
        ],
        expect: Reject("dep", "resolved type"),
    },
];

/// The Nix overlay question: a fixpoint package set as a library pattern.
const FIXPOINT: &[Case] = &[
    Case {
        id: "F1 knot-tying through a library fix, with an overlay",
        entry: "main",
        modules: &[Module {
            name: "main",
            source: r#"
let Packages = type { cc: str; zlib: str; editor: str };
let fix = fn(f: fn(Packages) -> Packages) -> Packages { f(fix(f)) };
let base = fn(p: Packages) -> Packages { ({ cc = "cc-1"; zlib = "zlib-1 with " + p.cc; editor = "editor with " + p.zlib }) };
let overlay = fn(p: Packages) -> Packages { ({ ...base(p); zlib = "zlib-2 with " + p.cc }) };
[fix(base).editor, fix(overlay).editor, fix(overlay).cc]
"#,
            deps: &[],
        }],
        expect: Value(
            &[],
            r#"["editor with zlib-1 with cc-1","editor with zlib-2 with cc-1","cc-1"]"#,
        ),
    },
    Case {
        id: "F2 the same across modules",
        entry: "main",
        modules: &[
            Module {
                name: "pkgs",
                source: r#"
let Packages = type { cc: str; zlib: str; editor: str };
let fix = fn(f: fn(Packages) -> Packages) -> Packages { f(fix(f)) };
{
    Packages = Packages;
    fix = fix;
    base = fn(p: Packages) -> Packages { ({ cc = "cc-1"; zlib = "zlib-1 with " + p.cc; editor = "editor with " + p.zlib }) };
}
"#,
                deps: &[],
            },
            Module {
                name: "main",
                source: r#"
let Packages = pkgs.Packages;
let base = pkgs.base;
let overlay = fn(p: Packages) -> Packages { ({ ...base(p); zlib = "zlib-2 with " + p.cc }) };
pkgs.fix(overlay).editor
"#,
                deps: &[("pkgs", "pkgs")],
            },
        ],
        expect: Value(&[], r#""editor with zlib-2 with cc-1""#),
    },
    Case {
        id: "F3 a self-dependent field diverges instead of hanging the checker",
        entry: "main",
        modules: &[Module {
            name: "main",
            source: r#"
let Packages = type { a: str; b: str };
let fix = fn(f: fn(Packages) -> Packages) -> Packages { f(fix(f)) };
let bad = fn(p: Packages) -> Packages { ({ a = p.b; b = p.a }) };
fix(bad).a
"#,
            deps: &[],
        }],
        expect: EvalError(&[], ""),
    },
];

#[test]
fn linking() {
    run_table(LINKING);
}

#[test]
fn immutability_and_copies() {
    run_table(IMMUTABILITY);
}

#[test]
fn exports() {
    run_table(EXPORTS);
}

#[test]
fn fixpoint_package_sets() {
    run_table(FIXPOINT);
}

#[test]
fn diagnostics_name_their_module() {
    let modules = build(&[
        Module {
            name: "dep",
            source: "{ bad = 1 / 0; fine = 1 }",
            deps: &[],
        },
        Module {
            name: "main",
            source: "[dep.fine, dep.bad]",
            deps: &[("dep", "dep")],
        },
    ]);
    let program = Program::prepare_modules("main", &modules).unwrap();
    let error = program.evaluate(&[]).unwrap_err();
    let rendered = program.render_diagnostic(&error);
    assert!(rendered.starts_with("dep:"), "{rendered}");

    let modules = build(&[Module {
        name: "main",
        source: "[1].take(5)",
        deps: &[],
    }]);
    let program = Program::prepare_modules("main", &modules).unwrap();
    let error = program.evaluate(&[]).unwrap_err();
    let rendered = program.render_diagnostic(&error);
    // Documented: a failure inside prelude code is rendered against the prelude source.
    assert!(rendered.starts_with("<stdlib>:"), "{rendered}");
}
