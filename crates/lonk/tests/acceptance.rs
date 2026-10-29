//! Behavioral regression tests for the first checked slice.
//! Every case is a whole program. The division-by-zero canary makes "must not force" observable.
//! The formatter tests at the end run every program here through format, reparse and evaluate.

use lonk::{Program, Syntax};

enum Expect {
    /// Prepares, and the selected path materializes to this canonical text.
    Value(&'static [&'static str], &'static str),
    /// Preparation fails with a message containing this text.
    Reject(&'static str),
    /// Prepares, and selecting the path fails at evaluation with a message containing this text.
    EvalError(&'static [&'static str], &'static str),
}
use Expect::*;

struct Case {
    id: &'static str,
    source: &'static str,
    expect: Expect,
}

fn check(case: &Case) -> Result<(), String> {
    let program = Program::prepare(case.source);
    match (&case.expect, program) {
        (Reject(needle), Err(e)) => {
            if e.message.contains(needle) {
                Ok(())
            } else {
                Err(format!(
                    "rejected for another reason: {}",
                    e.render(case.source)
                ))
            }
        }
        (Reject(_), Ok(p)) => Err(format!(
            "accepted; type {}; value {:?}",
            p.result_type(),
            p.evaluate(&[]).map(|v| v.to_string())
        )),
        (_, Err(e)) => Err(format!("unexpectedly rejected: {}", e.render(case.source))),
        (Value(path, want), Ok(p)) => match p.evaluate(path) {
            Ok(v) if v.to_string() == *want => Ok(()),
            Ok(v) => Err(format!("value {v} but wanted {want}")),
            Err(e) => Err(format!("evaluation failed: {}", e.render(case.source))),
        },
        (EvalError(path, needle), Ok(p)) => match p.evaluate(path) {
            Ok(v) => Err(format!(
                "evaluated to {v} but an error containing {needle:?} was expected"
            )),
            Err(e) if e.message.contains(needle) => Ok(()),
            Err(e) => Err(format!(
                "failed for another reason: {}",
                e.render(case.source)
            )),
        },
    }
}

fn run_table(cases: &[Case]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| {
            check(c)
                .err()
                .map(|e| format!("{}: {e}\n    {}", c.id, c.source.trim()))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

fn snapshots_and_versions_cases() -> Vec<Case> {
    vec![
        Case {
            id: "A1",
            source: "let a = { nested = { count = 1 } }; let b = a; b.nested.count += 1; { original = a.nested.count; changed = b.nested.count }",
            expect: Value(&[], r#"{"changed":2,"original":1}"#),
        },
        Case {
            id: "A2",
            source: "let a = { n = 1 }; let b = a; a.n = 2; { x = a.n; y = b.n }",
            expect: Value(&[], r#"{"x":2,"y":1}"#),
        },
        Case {
            id: "A3",
            source: "let a = { inner = { v = 1 } }; let b = { left = a; right = a }; b.left.inner.v = 2; { l = b.left.inner.v; r = b.right.inner.v; o = a.inner.v }",
            expect: Value(&[], r#"{"l":2,"o":1,"r":1}"#),
        },
        Case {
            id: "A4",
            source: "let a = { n = 1 }; a = { n = a.n + 1 }; a.n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A5",
            source: "let xs = [1, 2]; let ys = xs; ys[0] = 10; { a = xs[0]; b = ys[0] }",
            expect: Value(&[], r#"{"a":1,"b":10}"#),
        },
        Case {
            id: "A6",
            source: r#"let pkg: { version: str; url?: str } = { version = "1.0" }; pkg.url = "pkg-" + pkg.version; pkg.version = "2.0"; { version = pkg.version; url = pkg.url }"#,
            expect: Value(&[], r#"{"url":"pkg-1.0","version":"2.0"}"#),
        },
        Case {
            id: "A7",
            source: "let p = { x = 1 }; p.x = 2; let q = p; p.x = 3; { a = q.x; b = p.x }",
            expect: Value(&[], r#"{"a":2,"b":3}"#),
        },
        Case {
            id: "A8",
            source: "let p = { x = 1 }; let q = p; p = { x = 2 }; { a = q.x; b = p.x }",
            expect: Value(&[], r#"{"a":1,"b":2}"#),
        },
    ]
}

fn closures_and_frames_cases() -> Vec<Case> {
    vec![
        Case {
            id: "B1",
            source: "let x = 1; let f = fn() -> int { x }; x = 2; { captured = f(); now = x }",
            expect: Value(&[], r#"{"captured":1,"now":2}"#),
        },
        Case {
            id: "B2",
            source: "let cfg = { n = 1 }; let get = fn() -> int { cfg.n }; cfg.n = 5; { a = get(); b = cfg.n }",
            expect: Value(&[], r#"{"a":1,"b":5}"#),
        },
        Case {
            id: "B3",
            source: "let bump = fn(v: { n: int }) -> int { v.n += 1; v.n }; let a = { n = 1 }; { r = bump(a); orig = a.n }",
            expect: Value(&[], r#"{"orig":1,"r":2}"#),
        },
        Case {
            id: "B4",
            source: "let f = fn(a: int) -> int { a + 1 }; let g = f; g(1)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "B6",
            source: "let make = fn(x: int) { let y = x * 2; fn(z: int) -> int { x + y + z } }; let a = make(1); let b = make(10); [a(100), b(100)]",
            expect: Value(&[], "[103,130]"),
        },
        Case {
            id: "B7",
            source: "let make = fn(x: int) fn() -> int { x }; let call = fn(g: fn() -> int) -> int { g() }; call(make(7))",
            expect: Value(&[], "7"),
        },
        Case {
            id: "B8",
            source: "let p: { ... } = {}; let f = if true { p.x = 1; fn() -> int { p.x } } else { fn() -> int { 0 } }; f()",
            expect: Value(&[], "1"),
        },
        Case {
            id: "B9",
            source: "let x = 1; let f = fn() { x = 2; x }; f()",
            expect: Reject("captured"),
        },
        Case {
            id: "B10",
            source: "let f = fn(x: int) -> int { x }; [f(1), f(2), f(1)]",
            expect: Value(&[], "[1,2,1]"),
        },
    ]
}

fn laziness_cases() -> Vec<Case> {
    vec![
        Case {
            id: "C1a",
            source: "{ a = 1; b = 1 / 0 }",
            expect: Value(&["a"], "1"),
        },
        Case {
            id: "C1b",
            source: "{ a = 1; b = 1 / 0 }",
            expect: EvalError(&["b"], "division by zero"),
        },
        Case {
            id: "C2",
            source: "{ outer = { bad = 1 / 0; good = 2 } }",
            expect: Value(&["outer", "good"], "2"),
        },
        Case {
            id: "C4",
            source: r#"let bad: int = "s"; { a = 1; b = bad }"#,
            expect: Reject("expected int"),
        },
        Case {
            id: "C5",
            source: r#"let f = fn() -> int { "s" }; { a = 1; f = f }"#,
            expect: Reject("expected int"),
        },
        Case {
            id: "C6",
            source: "let base = { a = 1; b = 1 / 0 }; let out = { ...base; b = 2 }; out",
            expect: Value(&[], r#"{"a":1,"b":2}"#),
        },
        Case {
            id: "C9",
            source: "let p = { x = 1; y = 1 / 0 }; let q: { x: int } = p; q",
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "C10",
            source: "let p = { x = 1; y = 1 / 0 }; let q: { x: int; y: int } = p; q.x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "C11",
            source: "let p = { a = { x = 1; y = 1 / 0 } }; let q: { a: { x: int, ... } } = p; q.a.x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "C12",
            source: "let f = fn(v: int, w: int) -> int { v }; f(1, 1 / 0)",
            expect: Value(&[], "1"),
        },
    ]
}

fn contracts_cases() -> Vec<Case> {
    vec![
        Case {
            id: "D1",
            source: "let v = { x = 1 }; v = { x = v.x + 1 }; v.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "D2",
            source: r#"let v = { x = 1 }; v = { name = "n"; old = v.x }; v"#,
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "D3",
            source: r#"let v = { x = 1 }; let v = { name = "n"; old = v.x }; v"#,
            expect: Value(&[], r#"{"name":"n","old":1}"#),
        },
        Case {
            id: "D4",
            source: r#"let v = { x = 1 }; let v = { name = "n" }; v"#,
            expect: Reject("unused"),
        },
        Case {
            id: "D5",
            source: "let v = {}; v.y = 10; v",
            expect: Reject("closed"),
        },
        Case {
            id: "D6",
            source: "let v: { ... } = {}; v.y = 10; v.y",
            expect: Value(&[], "10"),
        },
        Case {
            id: "D7",
            source: "let v: { ... } = {}; v.y",
            expect: Reject("not known"),
        },
        Case {
            id: "D8",
            source: r#"let v = { x = 1 }; v.x = "s"; v"#,
            expect: Reject("expected int"),
        },
        Case {
            id: "D9",
            source: "let v = { x = 1 }; v.y = 2; v",
            expect: Reject("closed"),
        },
        Case {
            id: "D10a",
            source: r#"let n = 1; n = "s"; n"#,
            expect: Reject("expected int"),
        },
        Case {
            id: "D10b",
            source: "let n = 1; n = n + 1; n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "D11",
            source: r#"let mk = fn() -> { name: str } { ({ name = "a" }) }; let p = mk(); p.nmae = "b"; p"#,
            expect: Reject("closed"),
        },
        Case {
            id: "D12",
            source: "x = 1; x",
            expect: Reject("unknown assignment target"),
        },
        Case {
            id: "D13",
            source: "let p: { x?: int } = {}; if true { p.x = 1; }; match p { { x: any; ... } => { p.x }; else => 0; }",
            expect: Value(&[], "1"),
        },
    ]
}

fn projection_cases() -> Vec<Case> {
    vec![
        Case {
            id: "E1a",
            source: r#"let p = { x = 1; y = "s" }; let q: { x: int } = p; q"#,
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "E1b",
            source: r#"let p = { x = 1; y = "s" }; let q: { x: int } = p; q.y"#,
            expect: Reject("not known"),
        },
        Case {
            id: "E2",
            source: "let q: { x: int } = { x = 1; typo = 2 }; q",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E3",
            source: "let q: { x: int } = { y = 1 }; q",
            expect: Reject("field"),
        },
        Case {
            id: "E4",
            source: r#"let only_x = fn(v: { x: int }) -> { x: int } { v }; let p = { x = 1; y = "s" }; only_x(p)"#,
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "E5",
            source: r#"let only_x = fn(v: { x: int }) -> { x: int } { v }; only_x({ x = 1; y = "s" })"#,
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E6",
            source: "let f = fn() -> { x: int } { ({ x = 1; y = 2 }) }; f()",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E6b",
            source: "let f = fn() -> { x: int } { { x = 1; y = 2 } }; f()",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E7",
            source: "let f = fn() -> { x: int } { let p = { x = 1; y = 2 }; p }; f()",
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "E8a",
            source: r#"let p = { x = 1; y = "s" }; let w: { x: int, ... } = p; w.y"#,
            expect: Reject("not known"),
        },
        Case {
            id: "E8b",
            source: r#"let p = { x = 1; y = "s" }; let w: { x: int, ... } = p; w"#,
            expect: Value(&[], r#"{"x":1,"y":"s"}"#),
        },
        Case {
            id: "E9a",
            source: r#"let q: { x: int } = { x = 0 }; let p = { x = q.x + 1; y = "s" }; q = p; q"#,
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "E9b",
            source: r#"let q: { x: int } = { x = 0 }; q = { x = 1; y = "s" }; q"#,
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E10",
            source: "let p = { a = { x = 1; y = 2 }; z = 3 }; let q: { a: { x: int } } = p; q",
            expect: Value(&[], r#"{"a":{"x":1}}"#),
        },
        Case {
            id: "E12",
            source: "let u = { sub = { x = 0 } }; let p = { x = 1; y = 2 }; u.sub = p; u.sub",
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "E13",
            source: "let p = { a = { x = 1; y = 2 } }; let q: { a: { x: int, ... } } = p; q",
            expect: Value(&[], r#"{"a":{"x":1,"y":2}}"#),
        },
        Case {
            id: "E14",
            source: "let p: any = { x = 1; y = 2 }; match p { { x: int } => 1; else => 2; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "E15",
            source: r#"let p: any = { x = "s" }; match p { { x?: int; ... } => 1; else => 2; }"#,
            expect: Value(&[], "2"),
        },
        Case {
            id: "E16",
            source: "let a = { x = 1 }; let b0 = { x = 1; y = 2 }; let b: { x: int } = b0; a == b",
            expect: Value(&[], "true"),
        },
        Case {
            id: "E17",
            source: "let p: { x: int, ... } = { x = 1 }; let q: { x: int; y: int } = p; q",
            expect: Reject("evidence"),
        },
        Case {
            id: "E18",
            source: "let q: { a: { x: int } } = { a = { x = 1; typo = 2 } }; q",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E19",
            source: "let xs: [{ x: int }] = [{ x = 1; typo = 2 }]; xs",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "E20",
            source: "let p = { x = 1; y = 2 }; let xs: [{ x: int }] = [p]; xs",
            expect: Value(&[], r#"[{"x":1}]"#),
        },
        Case {
            id: "E21",
            source: "let p: { ... } = { x = 1; y = 2 }; let q = { ...p; z = 3 }; let r: { z: int } = q; r",
            expect: Value(&[], r#"{"z":3}"#),
        },
        Case {
            id: "E22",
            source: "let p: { ... } = { x = 1; y = 2 }; let q = { ...p; z = 3 }; let r: { z: int; y: int } = q; r",
            expect: Reject("evidence"),
        },
    ]
}

fn primitives_cases() -> Vec<Case> {
    vec![
        Case {
            id: "F1",
            source: "9223372036854775807 + 1",
            expect: EvalError(&[], "overflow"),
        },
        Case {
            id: "F2",
            source: "{ a = 7 / 2; b = -7 / 2; c = 7 % -2 }",
            expect: Value(&[], r#"{"a":3,"b":-3,"c":1}"#),
        },
        Case {
            id: "F2b",
            source: "1 % 0",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "F2c",
            source: "-9223372036854775808 / -1",
            expect: EvalError(&[], "overflow"),
        },
        Case {
            id: "F3",
            source: r#"1 + "s""#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "F4",
            source: "{ a = false && (1 / 0 == 1); b = true || (1 / 0 == 1) }",
            expect: Value(&[], r#"{"a":false,"b":true}"#),
        },
        Case {
            id: "F5a",
            source: "if 1 { 2 } else { 3 }",
            expect: Reject("expected bool"),
        },
        Case {
            id: "F5b",
            source: r#"1 == "1""#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "F6",
            source: "0 - (-9223372036854775808)",
            expect: EvalError(&[], "overflow"),
        },
        Case {
            id: "F6b",
            source: "9223372036854775808",
            expect: Reject("outside"),
        },
        Case {
            id: "F8",
            source: "let f = fn() -> int { 1 }; f == f",
            expect: Reject("does not accept"),
        },
        Case {
            id: "F9",
            source: "-(-5) + !false == true && 2 * 3 - 1 == 5",
            expect: Reject("does not accept"),
        },
        Case {
            id: "F10",
            source: "-(-5) == 5 && 2 * 3 - 1 == 5",
            expect: Value(&[], "true"),
        },
        Case {
            id: "F11",
            source: r#""${HOME} a\nb""#,
            expect: Value(&[], r#""${HOME} a\nb""#),
        },
    ]
}

fn usefulness_cases() -> Vec<Case> {
    vec![
        Case {
            id: "G1",
            source: "let dead = 1; 42",
            expect: Reject("unused"),
        },
        Case {
            id: "G2",
            source: "let f = fn() -> int { 1 }; f(); 42",
            expect: Reject("unused expression result"),
        },
        Case {
            id: "G3",
            source: "let f = fn() { }; 1",
            expect: Reject("must return a value"),
        },
        Case {
            id: "G4",
            source: "let a = 1; let b = a + 1; 42",
            expect: Reject("unused"),
        },
        Case {
            id: "G5",
            source: "let g = fn() -> int { 1 }; 42",
            expect: Reject("unused"),
        },
        Case {
            id: "G6",
            source: "let f = fn(a: int, b: int) -> int { a }; f(1, 2)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "G7",
            source: "{ a = 1; b = 2 }",
            expect: Value(&["a"], "1"),
        },
        Case {
            id: "G9",
            source: "let x = 1; x = 2; x",
            expect: Reject("overwritten"),
        },
        Case {
            id: "G10",
            source: "let p: { ... } = {}; if true { }; p",
            expect: Reject("does not contribute"),
        },
        Case {
            id: "G11",
            source: "let T = type { x: int }; let f = fn(v: T) -> int { v.x }; f({ x = 1 })",
            expect: Value(&[], "1"),
        },
    ]
}

fn parser_cases() -> Vec<Case> {
    vec![
        Case {
            id: "H1",
            source: "let f = fn() ({ answer = 42 }); f().answer",
            expect: Value(&[], "42"),
        },
        Case {
            id: "H2",
            source: "let f = fn() { answer = 42 }; f()",
            expect: Reject(""),
        },
        Case {
            id: "H3",
            source: "let add = fn(a: int, b: int) -> int { a + b }; add(1, 2)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "H4a",
            source: "{ a = 1; }",
            expect: Value(&[], r#"{"a":1}"#),
        },
        Case {
            id: "H4b",
            source: "[1, 2,]",
            expect: Value(&[], "[1,2]"),
        },
        Case {
            id: "H4c",
            source: "{ a = 1\n  b = 2 }",
            expect: Value(&[], r#"{"a":1,"b":2}"#),
        },
        Case {
            id: "H5",
            source: "let x = /* c /* nested */ */ 1; // d\n x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "H7a",
            source: "for x in [1] { x }",
            expect: Reject("unused"),
        },
        Case {
            id: "H7b",
            source: "let p = 1; &p",
            expect: Reject("borrowing"),
        },
        Case {
            id: "H7c",
            source: "match 1 { 1 => 2; }",
            expect: Reject(""),
        },
        Case {
            id: "H7f",
            source: "let p: any = 1; p is int",
            expect: Reject("match"),
        },
        Case {
            id: "H7d",
            source: "let f = fn<R>(v: { x: int, ...R }) -> { x: int, ...R } { v }; f({ x = 1; y = 2 })",
            expect: Value(&[], r#"{"x":1,"y":2}"#),
        },
        Case {
            id: "H7e",
            source: "let p: any = 1; p + 1",
            expect: Reject("does not accept"),
        },
        Case {
            id: "H8",
            source: r#"let p = { "quoted key" = 1 }; p["quoted key"]"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "H9",
            source: "if true { 1 } else if false { 2 } else { 3 }",
            expect: Value(&[], "1"),
        },
        Case {
            id: "H9",
            source: "if true { 1 } else if false { 2 } else { 3 }",
            expect: Value(&[], "1"),
        },
    ]
}

fn guard_cases() -> Vec<Case> {
    vec![
        Case {
            id: "J1",
            source: r#"let p: { x?: int } = { x = 1 }; match p { { x: any; ... } => { p = {}; p.x }; else => 0; }"#,
            expect: Reject("field `x`"),
        },
        Case {
            id: "J2",
            source: r#"let p: { ... } = {}; p.x = 1; ("x" in p) && p.x == 1"#,
            expect: Value(&[], "true"),
        },
        Case {
            id: "J3",
            source: r#"let p: { ... } = {}; p.x = 1; ("y" in p) && p.y == 1"#,
            expect: Reject("not known"),
        },
        Case {
            id: "J4",
            source: r#"let p: { ... } = { x = "s" }; match p { { x: any; ... } => { p.x + 1 }; else => 0; }"#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "J5",
            source: r#"let p: { ... } = { x = "s" }; match p { { x: int; ... } => { p.x + 1 }; else => 0; }"#,
            expect: Value(&[], "0"),
        },
        Case {
            id: "J6",
            source: "let p: any = { a = { x = 1 } }; match p { { a: { x: int } } => { p.a.x = 2; p.a.x }; else => 0; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "J7",
            source: r#"let p: { ... } = {}; if true { p.x = 1; } else { p.x = "s"; }; p.x + 1"#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "J8",
            source: "let xs = [{ x = 1 }, { x = 1; y = 2 }]; xs[1].y",
            expect: Reject("optional"),
        },
        Case {
            id: "J9",
            source: r#"let p = { x = 1 }; let q = { ...p; x = "s" }; q.x"#,
            expect: Value(&[], r#""s""#),
        },
        Case {
            id: "J10",
            source: r#"let p: { ... } = { x = 1 }; let q = { y = 2; ...p }; q.y + 1"#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "J11",
            source: "let p: { x?: int } = {}; let q = { x = 1; ...p }; q.x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "J12",
            source: r#"let p: { x?: int } = {}; let q = { x = "s"; ...p }; q.x + 1"#,
            expect: Reject("does not accept"),
        },
    ]
}

/// Programs whose shape is likely to trip a printer: brace-led function bodies,
/// precedence corners, keywords as keys, comments in awkward places.
fn formatting_corner_cases() -> Vec<Case> {
    vec![
        Case {
            id: "K1",
            source: "let f = fn() ({ a = 1 }).a; f()",
            expect: Value(&[], "1"),
        },
        Case {
            id: "K2",
            source: "let f = fn() ({ a = [7] }).a[0]; f()",
            expect: Value(&[], "7"),
        },
        Case {
            id: "K3",
            source: "let f = fn() ({ g = fn() -> int { 3 } }).g(); f()",
            expect: Value(&[], "3"),
        },
        Case {
            id: "K4",
            source: "let p: any = 1; (match p { int => true; else => false; }) == true",
            expect: Value(&[], "true"),
        },
        Case {
            id: "K5",
            source: "let p: any = 1; !(match p { int => true; else => false; })",
            expect: Value(&[], "false"),
        },
        Case {
            id: "K6",
            source: "-(-(-1))",
            expect: Value(&[], "-1"),
        },
        Case {
            id: "K7",
            source: "8 - (3 - 1) - 1",
            expect: Value(&[], "5"),
        },
        Case {
            id: "K8",
            source: "(2 + 3) * (4 - 1) % 4",
            expect: Value(&[], "3"),
        },
        Case {
            id: "K9",
            source: "let f = fn(x: int) -> { a: int } { ({ a = x }) }; f(2).a",
            expect: Value(&[], "2"),
        },
        Case {
            id: "K10",
            source: "let f = fn(c: bool) -> int { if c { 1 } else { 2 } }; f(false)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "K11",
            source: r#"let p = { "if" = 1; "a b" = 2; _x = 3 }; p["if"] + p["a b"] + p._x"#,
            expect: Value(&[], "6"),
        },
        Case {
            id: "K12",
            source: "let p: { x: int, ... } = { x = 1 }; let q: {} = {}; let t = type { a?: [int]; ... }; { p = p; q = q; t = t }",
            expect: EvalError(&[], "only data"),
        },
        Case {
            id: "K12b",
            source: "let p: { x: int, ... } = { x = 1 }; let q: {} = {}; let t = type { a?: [int]; ... }; { p = p; q = q; t = t }",
            expect: Value(&["p"], r#"{"x":1}"#),
        },
        Case {
            id: "K13",
            source: "let x = 1; // about x\nlet y = x + 1; /* about y */\n{ x = x; y = y } // end",
            expect: Value(&[], r#"{"x":1,"y":2}"#),
        },
        Case {
            id: "K14",
            source: "let xs = [1]; xs[0] += 1; if xs[0] == 2 { xs[0] = 5; }; xs",
            expect: Value(&[], "[5]"),
        },
        Case {
            id: "K15",
            source: "let f = fn() { let a = { b = 1 }; a.b }; f()",
            expect: Value(&[], "1"),
        },
        Case {
            id: "K16",
            source: "(fn(x: int) x * 2)(21)",
            expect: Value(&[], "42"),
        },
        Case {
            id: "K17",
            source: "let s = \"tab\\there \\\\ \\\" done\"; s",
            expect: Value(&[], r#""tab\there \\ \" done""#),
        },
        Case {
            id: "K18",
            source: "let f = fn(x: int) -> int { x }; f(4) - -f(1)",
            expect: Value(&[], "5"),
        },
    ]
}

/// Bare-pattern match: ordering, presence without forcing, arms checked before demand,
/// arm mutation joins, proofs confined to their arm.
fn match_cases() -> Vec<Case> {
    vec![
        Case {
            id: "M1a",
            source: r#"let p: any = 1; match p { int => "type"; 1 => "lit"; else => "none"; }"#,
            expect: Value(&[], r#""type""#),
        },
        Case {
            id: "M1b",
            source: r#"let p: any = 1; match p { 1 => "lit"; int => "type"; else => "none"; }"#,
            expect: Value(&[], r#""lit""#),
        },
        Case {
            id: "M2",
            source: r#"let p: any = "1"; match p { 1 => "int"; "1" => "str"; else => "none"; }"#,
            expect: Value(&[], r#""str""#),
        },
        Case {
            id: "M3a",
            source: r#"let p: { x?: int } = { x = 1 / 0 }; match p { { x: any; ... } => "present"; else => "absent"; }"#,
            expect: Value(&[], r#""present""#),
        },
        Case {
            id: "M3b",
            source: r#"let p: any = { x = 1 / 0 }; match p { { x: int; ... } => "present"; else => "absent"; }"#,
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "M3c",
            source: r#"let p: { x?: int } = {}; match p { { x: any; ... } => "present"; else => "absent"; }"#,
            expect: Value(&[], r#""absent""#),
        },
        Case {
            id: "M3d",
            source: r#"let p: { x?: int } = { x = 1 }; match p { { x: any; ... } => { p.x + 1 }; else => 0; }"#,
            expect: Value(&[], "2"),
        },
        Case {
            id: "M4",
            source: r#"let p: any = 1; match p { int => 1; str => 1 + "s"; else => 0; }"#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "M5",
            source: "let p: { ... } = {}; match p { {} => { p.x = 1; }; else => { p.x = 2; }; }; p.x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "M6",
            source: "let p: { ... } = {}; match p { {} => { p.x = 1; }; else => { }; }; p.x",
            expect: Reject("optional"),
        },
        Case {
            id: "M7",
            source: "let p: any = { x = 1 }; let r = match p { { x: int } => 0; else => 1; }; r + p.x",
            expect: Reject("not known"),
        },
        Case {
            id: "M8",
            source: r#"let p: any = 1; match p { int => ({ kind = "int" }); else => ({ kind = "other" }); }"#,
            expect: Value(&["kind"], r#""int""#),
        },
        Case {
            id: "M9",
            source: "let p: any = { x = 1; y = 2 }; match p { { x: int } => 1; { x: int; ... } => 2; else => 3; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M10a",
            source: "let a: any = { b = { c = 5 } }; match a { { b: { c: int; ... }; ... } => a.b.c; else => 0; }",
            expect: Value(&[], "5"),
        },
        Case {
            id: "M10b",
            source: "let a: any = {}; match a { { b: { c: int; ... }; ... } => a.b.c; else => 0; }",
            expect: Value(&[], "0"),
        },
        Case {
            id: "M10c",
            source: r#"let a: any = { b = { c = "s" } }; match a { { b: { c: int; ... }; ... } => a.b.c; else => 0; }"#,
            expect: Value(&[], "0"),
        },
        Case {
            id: "M11",
            source: "match 1 + 1 { int => 1; else => 0; }",
            expect: Value(&[], "1"),
        },
        Case {
            id: "M12",
            source: "let p: any = 1; match p { int => 1; }",
            expect: Reject(""),
        },
        Case {
            id: "M13",
            source: "let p: any = { x = 1 }; match p { { x: int; ... } => { let q = p; q.x }; else => 0; }",
            expect: Value(&[], "1"),
        },
        Case {
            id: "M14",
            source: "let p: any = [1, 2]; match p { [int] => p[1]; else => 0; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M15",
            source: r#"let p: any = { x = 1 }; match p { { x: int; ... } => { p = { x = "s" }; p.x }; else => 0; }"#,
            expect: Reject(""),
        },
        Case {
            id: "M16",
            source: "let p: any = 1; let f = fn() -> int { match p { int => p + 1; else => 0; } }; f()",
            expect: Value(&[], "2"),
        },
    ]
}

/// Array spread: laziness of elements versus structure, projection at typed destinations,
/// joins, nested and generic spreads, borrowed append through nested paths.
fn array_spread_cases() -> Vec<Case> {
    vec![
        Case {
            id: "L1a",
            source: r#"let a = [1]; let b = [...a, "s"]; b"#,
            expect: Value(&[], r#"[1,"s"]"#),
        },
        Case {
            id: "L1b",
            source: r#"let a = [1]; let b = [...a, "s"]; b[0] + 1"#,
            expect: Reject("does not accept"),
        },
        Case {
            id: "L2",
            source: "let bad: [int] = [1 / 0]; [...bad, 2][1]",
            expect: Value(&[], "2"),
        },
        Case {
            id: "L2b",
            source: "let bad: [int] = [1 / 0]; [...bad, 2][0]",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "L3",
            source: "let f = fn() -> [int] { [1 / 0] }; [...f(), 1][1]",
            expect: Value(&[], "1"),
        },
        Case {
            id: "L4a",
            source: "let ps = [{ x = 1; y = 2 }]; let qs: [{ x: int }] = [...ps, { x = 3 }]; qs",
            expect: Value(&[], r#"[{"x":1},{"x":3}]"#),
        },
        Case {
            id: "L4b",
            source: "let ps = [{ x = 1; y = 2 }]; let qs: [{ x: int }] = [...ps, { x = 3; z = 4 }]; qs",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "L5",
            source: "let a = [1]; [...[...a, 2], 3]",
            expect: Value(&[], "[1,2,3]"),
        },
        Case {
            id: "L6",
            source: r#"let p = { deps = ["a"] }; p.deps = [...p.deps, "b"]; p.deps"#,
            expect: Value(&[], r#"["a","b"]"#),
        },
        Case {
            id: "L7",
            source: "let a = [1]; [...a, 2]; 1",
            expect: Reject("unused expression result"),
        },
        Case {
            id: "L8a",
            source: "let push = fn<T>(xs: [T], x: T) -> [T] { [...xs, x] }; push([{ x = 1 }], { x = 2 })",
            expect: Value(&[], r#"[{"x":1},{"x":2}]"#),
        },
        Case {
            id: "L8b",
            source: r#"let push = fn<T>(xs: [T], x: T) -> [T] { [...xs, x] }; push([1], "s")"#,
            expect: Reject(""),
        },
        Case {
            id: "L9",
            source: "let dup = fn<T>(xs: [T]) -> [T] { [...xs, ...xs] }; dup([1, 2])",
            expect: Value(&[], "[1,2,1,2]"),
        },
        Case {
            id: "L10",
            source: "let a = [1, 1 / 0]; [...a, ...a][2]",
            expect: Value(&[], "1"),
        },
        Case {
            id: "L11",
            source: "let a = [1]; let b = [...a]; b[0] = 9; { a = a; b = b }",
            expect: Value(&[], r#"{"a":[1],"b":[9]}"#),
        },
        Case {
            id: "L12",
            source: "let e: [int] = []; [...e, ...e, 1]",
            expect: Value(&[], "[1]"),
        },
        Case {
            id: "L13",
            source: "[...{ a = 1 }]",
            expect: Reject(""),
        },
        Case {
            id: "L14",
            source: "let a = [1]; { ...a }",
            expect: Reject(""),
        },
    ]
}

/// Direct recursion by definition handle: explicit signatures, no mutual recursion,
/// the recursive call ignores later reassignment, divergence hits the nesting limit.
fn recursion_cases() -> Vec<Case> {
    vec![
        Case {
            id: "Q1",
            source: "let fact = fn(n: int) -> int { if n == 0 { 1 } else { n * fact(n - 1) } }; fact(5)",
            expect: Value(&[], "120"),
        },
        Case {
            id: "Q2",
            source: "let sum = fn(xs: [int], i: int) -> int { if i == 2 { 0 } else { xs[i] + sum(xs, i + 1) } }; sum([1, 2], 0)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "Q3",
            source: "let f = fn(n: int) -> int { if n == 0 { 0 } else { f(n - 1) + 1 } }; let g = f; f = fn(n: int) -> int { 100 }; { g = g(3); f = f(3) }",
            expect: Value(&[], r#"{"f":100,"g":3}"#),
        },
        Case {
            id: "Q4",
            source: "let spin = fn(n: int) -> int { spin(n + 1) }; { a = 1; b = spin(0) }",
            expect: Value(&["a"], "1"),
        },
        Case {
            id: "Q4b",
            source: "let spin = fn(n: int) -> int { spin(n + 1) }; spin(0)",
            expect: EvalError(&[], "nesting"),
        },
        Case {
            id: "Q5",
            source: "let even = fn(n: int) -> bool { if n == 0 { true } else { odd(n - 1) } }; let odd = fn(n: int) -> bool { if n == 0 { false } else { even(n - 1) } }; even(4)",
            expect: Reject("unknown name"),
        },
        Case {
            id: "Q6",
            source: "let count = fn(n: int) -> int { if n == 0 { 0 } else { 1 + count(n - 1) } }; count(100)",
            expect: Value(&[], "100"),
        },
        Case {
            id: "Q6b",
            source: "let count = fn(n: int) -> int { if n == 0 { 0 } else { 1 + count(n - 1) } }; count(10)",
            expect: Value(&[], "10"),
        },
        Case {
            id: "Q13",
            source: "let f = fn(n: int) -> int { n }; let f = fn(n: int) -> int { f(n) + 1 }; f(1)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "Q7",
            source: "let fill = fn(xs: &[int], n: int) { if n > 0 { xs = [...xs, n]; fill(&xs, n - 1); }; }; let a: [int] = []; fill(&a, 3); a",
            expect: Value(&[], "[3,2,1]"),
        },
        Case {
            id: "Q8",
            source: "let f = fn(n: int) if n == 0 { 1 } else { n * f(n - 1) }; f(3)",
            expect: Reject(""),
        },
        Case {
            id: "Q9",
            source: "let m = { f = fn(n: int) -> int { if n == 0 { 0 } else { m.f(n - 1) } } }; m.f(2)",
            expect: Reject("unknown"),
        },
        Case {
            id: "Q10",
            source: "let depth = fn(n: int) -> { n: int; inner: any } { if n == 0 { { n = 0; inner = 0 } } else { { n = n; inner = depth(n - 1) } } }; depth(2).n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "Q11",
            source: "let lazy = fn(n: int) -> { v: int; rest: any } { ({ v = n; rest = lazy(n + 1) }) }; lazy(0).v",
            expect: Value(&[], "0"),
        },
        Case {
            id: "Q12",
            source: "let f = fn<T>(x: T, n: int) -> T { if n == 0 { x } else { f(x, n - 1) } }; f(\"s\", 3)",
            expect: Value(&[], r#""s""#),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(snapshots_and_versions_cases());
    all.extend(closures_and_frames_cases());
    all.extend(laziness_cases());
    all.extend(contracts_cases());
    all.extend(projection_cases());
    all.extend(primitives_cases());
    all.extend(usefulness_cases());
    all.extend(parser_cases());
    all.extend(guard_cases());
    all.extend(formatting_corner_cases());
    all.extend(match_cases());
    all.extend(array_spread_cases());
    all.extend(recursion_cases());
    all
}

#[test]
fn snapshots_and_versions() {
    run_table(&snapshots_and_versions_cases());
}
#[test]
fn closures_and_frames() {
    run_table(&closures_and_frames_cases());
}
#[test]
fn laziness_and_preparation_boundary() {
    run_table(&laziness_cases());
}
#[test]
fn fixed_contracts_and_shadowing() {
    run_table(&contracts_cases());
}
#[test]
fn typed_copies_projection_and_fresh_literals() {
    run_table(&projection_cases());
}
#[test]
fn checked_primitives() {
    run_table(&primitives_cases());
}
#[test]
fn usefulness() {
    run_table(&usefulness_cases());
}
#[test]
fn parser_and_spans() {
    run_table(&parser_cases());
}
#[test]
fn guards_and_mutation_interact_soundly() {
    run_table(&guard_cases());
}
#[test]
fn formatting_corner_programs_evaluate() {
    run_table(&formatting_corner_cases());
}
#[test]
fn match_narrowing() {
    run_table(&match_cases());
}
#[test]
fn array_spread() {
    run_table(&array_spread_cases());
}
#[test]
fn direct_recursion() {
    run_table(&recursion_cases());
}

#[test]
fn diagnostics_point_at_the_offending_expression() {
    let source = "let outer = {\n  a = 1;\n  b = { c = 1 + \"s\" }\n};\nouter";
    let error = Program::prepare(source).expect_err("ill-typed field");
    assert!(
        error.render(source).starts_with("3:13:"),
        "span should point at the binary expression on line 3, got {}",
        error.render(source)
    );
}

/// Outcome of preparing and evaluating a program at a path, comparable across formatting.
fn outcome(source: &str, path: &[&str]) -> String {
    match Program::prepare(source) {
        Err(e) => format!("reject: {}", e.message),
        Ok(p) => match p.evaluate(path) {
            Ok(v) => format!("value: {v}"),
            Err(e) => format!("eval error: {}", e.message),
        },
    }
}

/// format is idempotent, keeps every comment, and never changes what a program means.
#[test]
fn formatter_round_trips_every_acceptance_program() {
    let mut failures = Vec::new();
    for case in all_cases() {
        let path: &[&str] = match &case.expect {
            Value(path, _) | EvalError(path, _) => path,
            Reject(_) => &[],
        };
        let Ok(syntax) = Syntax::parse(case.source) else {
            continue; // parse errors are their own cases; nothing to format
        };
        let once = syntax.format();
        let reparsed = match Syntax::parse(&once) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!(
                    "{}: formatted text does not parse: {}\n{once}",
                    case.id,
                    e.render(&once)
                ));
                continue;
            }
        };
        let twice = reparsed.format();
        if twice != once {
            failures.push(format!(
                "{}: not idempotent\n--- once\n{once}--- twice\n{twice}",
                case.id
            ));
        }
        let before: Vec<_> = syntax.comments().iter().map(|c| c.text.clone()).collect();
        let after: Vec<_> = reparsed.comments().iter().map(|c| c.text.clone()).collect();
        if before != after {
            failures.push(format!(
                "{}: comments changed {before:?} -> {after:?}",
                case.id
            ));
        }
        let (a, b) = (outcome(case.source, path), outcome(&once, path));
        if a != b {
            failures.push(format!(
                "{}: meaning changed\n  original: {a}\n  formatted: {b}\n{once}",
                case.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} formatter failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn formatter_keeps_every_comment_of_a_commented_program() {
    let source = "let x = 1; // about x\nlet y = x + 1; /* about y */\n{ x = x; y = y } // end";
    let formatted = Syntax::parse(source).unwrap().format();
    for text in ["// about x", "/* about y */", "// end"] {
        assert!(
            formatted.contains(text),
            "comment {text:?} lost:\n{formatted}"
        );
    }
    eprintln!("comment layout:\n{formatted}");
}

/// Depths that would overflow any native stack: the evaluator must drive demand from the heap.
#[test]
fn deep_evaluation_does_not_depend_on_the_native_stack() {
    let mut failures = Vec::new();
    let mut check =
        |id: &str, source: String, path: &[&str], want: &str| match Program::prepare(&source) {
            Err(e) => failures.push(format!("{id}: rejected: {}", e.render(&source))),
            Ok(p) => match p.evaluate(path) {
                Ok(v) if v.to_string() == want => (),
                Ok(v) => failures.push(format!("{id}: got {v}, wanted {want}")),
                Err(e) => failures.push(format!("{id}: evaluation failed: {}", e.render(&source))),
            },
        };
    check(
        "deep-count",
        "let count = fn(n: int) -> int { if n == 0 { 0 } else { 1 + count(n - 1) } }; count(10000)"
            .into(),
        &[],
        "10000",
    );
    check(
        "deep-build",
        "let build = fn(xs: [int], n: int) -> [int] { if n == 0 { xs } else { build([...xs, n], n - 1) } }; build([], 2000)[1999]".into(),
        &[],
        "1",
    );
    let mut writes = String::from("let v: { ... } = {};\n");
    for i in 0..2000 {
        writes.push_str(&format!("v.f{i} = {i};\n"));
    }
    writes.push_str("v.f1999");
    check("deep-writes", writes, &[], "1999");
    check(
        "deep-lazy-walk",
        "let build = fn(n: int) -> { value: int; next: any } { ({ value = n; next = if n == 0 { 0 } else { build(n - 1) } }) }; let walk = fn(node: any, n: int) -> int { match node { { value: int; next: any } => if n == 0 { node.value } else { walk(node.next, n - 1) }; else => -1; } }; walk(build(2000), 2000)".into(),
        &[],
        "0",
    );
    check(
        "deep-borrow",
        "let fill = fn(xs: &[int], n: int) { if n > 0 { xs = [...xs, n]; fill(&xs, n - 1); }; }; let a: [int] = []; fill(&a, 1500); a[1499]".into(),
        &[],
        "1",
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn divergence_and_failure_memoization_stay_diagnostics() {
    let source = "let spin = fn(n: int) -> int { spin(n + 1) }; { a = 1; b = spin(0); c = 1 / 0 }";
    let program = Program::prepare(source).unwrap();
    let mut evaluation = program.evaluator();
    let first = evaluation.select(&["b"]).unwrap_err();
    assert_eq!(
        evaluation.select(&["b"]).unwrap_err(),
        first,
        "failure must be memoized"
    );
    assert_eq!(
        evaluation.select(&["a"]).unwrap().to_string(),
        "1",
        "healthy sibling after a failure"
    );
    assert!(
        evaluation
            .select(&["c"])
            .unwrap_err()
            .message
            .contains("division by zero")
    );
    assert_eq!(evaluation.select(&["a"]).unwrap().to_string(), "1");
}

/// A 2000-deep inferred record type is rejected at preparation with a depth diagnostic, never a crash.
#[test]
fn deep_static_types_are_rejected_at_preparation() {
    let mut nest = String::from("let leaf = { v = 7 };\n");
    for i in 0..2000 {
        let inner = if i == 0 {
            "leaf".to_string()
        } else {
            format!("n{}", i - 1)
        };
        nest.push_str(&format!("let n{i} = {{ inner = {inner} }};\n"));
    }
    nest.push_str("n1999");
    let error = Program::prepare(&nest).expect_err("a 2000-deep static type must be diagnosed");
    assert!(
        error.message.contains("limit exceeded"),
        "unexpected diagnostic: {}",
        error.render(&nest)
    );
}

#[test]
fn checked_artifacts_round_trip_the_acceptance_corpus() {
    use lonk::artifact::{ArtifactError, ArtifactKey, ArtifactStore, MemoryArtifactStore};
    use lonk::{Compiler, CompilerPolicy};
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
    let mut checked = 0;
    for case in all_cases() {
        let Ok(plain) = Program::prepare(case.source) else {
            continue;
        };
        let store = Store::default();
        let mut first = Compiler::with_store(
            b"test-build",
            CompilerPolicy::default(),
            Box::new(store.clone()),
        )
        .unwrap();
        let initial = first.prepare(case.source).unwrap();
        let mut restarted =
            Compiler::with_store(b"test-build", CompilerPolicy::default(), Box::new(store))
                .unwrap();
        let loaded = restarted.prepare(case.source).unwrap();
        assert_eq!(plain.result_type(), initial.result_type(), "{}", case.id);
        assert_eq!(plain.result_type(), loaded.result_type(), "{}", case.id);
        assert_eq!(plain.evaluate(&[]), loaded.evaluate(&[]), "{}", case.id);
        assert_eq!(restarted.stats().checks, 0, "{}", case.id);
        checked += 1;
    }
    assert!(checked >= 10, "acceptance corpus unexpectedly small");
}
