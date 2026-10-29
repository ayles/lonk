//! Behavioral regression tests for borrowed parameters.
//! Whole programs; `1 / 0` is the must-not-force canary. Borrows are call-site `&place`
//! arguments to parameters spelled `name: &T`; there are no stored references, local aliases
//! or receivers in this slice. Diagnostic matching allows wording changes.

use lonk::{Program, Syntax};

enum Expect {
    Value(&'static [&'static str], &'static str),
    Reject(&'static str),
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

/// Shared helpers: a scalar incrementer and a row-preserving counter bump.
const INC: &str = "let inc = fn(x: &int) { x += 1; };";
const BUMP: &str = "let bump = fn<R>(v: &{ count: int, ...R }) { v.count += 1; };";

fn basics() -> Vec<Case> {
    vec![
        Case {
            id: "A1",
            source: "INC let n = 1; inc(&n); n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A2",
            source: "INC let n = 1; inc(&n); inc(&n); n",
            expect: Value(&[], "3"),
        },
        Case {
            id: "A3",
            source: "INC let n = 1; inc(n); n",
            expect: Reject(""),
        },
        Case {
            id: "A4",
            source: "INC let n = 1; inc(&(n + 1)); n",
            expect: Reject(""),
        },
        Case {
            id: "A5",
            source: "let f = fn(x: &int) -> int { x += 1; x }; let n = 1; { r = f(&n); n = n }",
            expect: Value(&[], r#"{"n":2,"r":2}"#),
        },
        Case {
            id: "A6",
            source: "let f = fn(x: &int) -> int { x += 1; x }; let n = 1; f(&n); n",
            expect: Reject("unused expression result"),
        },
        Case {
            id: "A7",
            source: "let f = fn(x: &int) { let s = x; x += 1; }; let n = 1; f(&n); n",
            expect: Reject("unused"),
        },
        Case {
            id: "A8",
            source: "let f = fn(x: &int) -> int { let s = x; x += 1; s }; let n = 1; { old = f(&n); now = n }",
            expect: Value(&[], r#"{"now":2,"old":1}"#),
        },
        Case {
            id: "A9",
            source: "let f = fn(x: &int, y: int) { x += y; }; let n = 1; f(&n, 1 / 0); n",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "A10",
            source: "let f = fn(x: &int, y: int) { x += 1; }; let n = 1; f(&n, 1 / 0); n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A11",
            source: "let f = fn(x: &int) { x = x * 10; }; let n = 2; f(&n); n",
            expect: Value(&[], "20"),
        },
        Case {
            id: "A12",
            source: r#"let f = fn(x: &int) { x = "s"; }; let n = 2; f(&n); n"#,
            expect: Reject(""),
        },
        Case {
            id: "A13",
            source: "let f = fn(x: &int) { }; let n = 1; f(&n); n",
            expect: Value(&[], "1"),
        }, // lenient today; see #64
        Case {
            id: "A14",
            source: "let f = fn(x: &int) -> int { x + 1 }; let n = 1; f(&n)",
            expect: Value(&[], "2"),
        }, // lenient today; see #64
        Case {
            id: "A15",
            source: "INC let n = 1; let f = fn() { inc(&n); }; f(); n",
            expect: Reject(""),
        },
        Case {
            id: "A16",
            source: "INC let n = 1; let g = fn(m: &int) { inc(&m); inc(&m); }; g(&n); n",
            expect: Value(&[], "3"),
        },
    ]
}

fn places() -> Vec<Case> {
    vec![
        Case {
            id: "P1",
            source: "INC let o = { nested = { value = 1; other = 5 }; top = \"t\" }; inc(&o.nested.value); o",
            expect: Value(&[], r#"{"nested":{"other":5,"value":2},"top":"t"}"#),
        },
        Case {
            id: "P2",
            source: "INC let o = { nested = { value = 1; bad = 1 / 0 } }; inc(&o.nested.value); o.nested.value",
            expect: Value(&[], "2"),
        },
        Case {
            id: "P3",
            source: "INC let xs = [1, 2, 3]; inc(&xs[1]); xs",
            expect: Value(&[], "[1,3,3]"),
        },
        Case {
            id: "P4",
            source: "INC let xs = [1, 2]; let i = 1; inc(&xs[i]); xs",
            expect: Value(&[], "[1,3]"),
        },
        Case {
            id: "P5",
            source: "INC let xs = [1, 2]; inc(&xs[5]); xs",
            expect: EvalError(&[], ""),
        },
        Case {
            id: "P6",
            source: "let o = { a = 1; b = 1 }; let add = fn(x: &int, y: &int) { x += 1; y += 10; }; add(&o.a, &o.b); o",
            expect: Value(&[], r#"{"a":2,"b":11}"#),
        },
        Case {
            id: "P7",
            source: "let add = fn(x: &int, y: &int) { x += 1; y += 10; }; let o = { a = 1 }; add(&o.a, &o.a); o",
            expect: Reject(""),
        },
        Case {
            id: "P8",
            source: "let f = fn(x: &{ a: int }, y: &int) { y += 1; }; let o = { a = 1 }; f(&o, &o.a); o",
            expect: Reject(""),
        },
        Case {
            id: "P9",
            source: "INC let o = { a = 1 }; let s = o; inc(&o.a); { before = s.a; after = o.a }",
            expect: Value(&[], r#"{"after":2,"before":1}"#),
        },
        Case {
            id: "P10",
            source: "INC let o = { a = 1 }; let f = fn() -> int { o.a }; inc(&o.a); { captured = f(); now = o.a }",
            expect: Value(&[], r#"{"captured":1,"now":2}"#),
        },
        Case {
            id: "P11",
            source: "INC let o = { a = { v = 1 }; b = { v = 1 } }; let deep = fn(c: &{ v: int }) { inc(&c.v); }; deep(&o.a); o",
            expect: Value(&[], r#"{"a":{"v":2},"b":{"v":1}}"#),
        },
        Case {
            id: "P12",
            source: "INC let o = { a = 1 }; if true { inc(&o.a); } else { inc(&o.a); inc(&o.a); }; o.a",
            expect: Value(&[], "2"),
        },
        Case {
            id: "P13",
            source: "INC let o: { a?: int } = {}; inc(&o.a); o",
            expect: Reject(""),
        },
        Case {
            id: "P14",
            source: "INC let o: { ... } = {}; inc(&o.a); o",
            expect: Reject(""),
        },
        Case {
            id: "P15",
            source: "INC let r = &1; 1",
            expect: Reject(""),
        },
    ]
}

fn storage_rejections() -> Vec<Case> {
    vec![
        Case {
            id: "S1",
            source: "let n = 1; { let r = &n; r += 1; }; n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "S2",
            source: "let n = 1; { r = &n }",
            expect: Reject(""),
        },
        Case {
            id: "S3",
            source: "let n = 1; [&n]",
            expect: Reject(""),
        },
        Case {
            id: "S4",
            source: "let f = fn(x: &int) -> fn() -> int { x += 1; fn() -> int { x } }; let n = 1; let g = f(&n); n += 10; { g = g(); n = n }",
            expect: Value(&[], r#"{"g":2,"n":12}"#),
        },
        Case {
            id: "S5",
            source: "let f = fn(x: &int) { let g = fn() { x += 1; }; }; let n = 1; f(&n); n",
            expect: Reject(""),
        },
        Case {
            id: "S6",
            source: "let f = fn(x: &int) -> &int { x }; let n = 1; f(&n)",
            expect: Reject(""),
        },
        Case {
            id: "S7",
            source: "let f = fn(x: &int) { let y = x; y += 1; }; let n = 1; f(&n); n",
            expect: Reject(""),
        },
    ]
}

fn contracts() -> Vec<Case> {
    vec![
        Case {
            id: "C1",
            source: "let edit = fn(v: &{ x: int, ... }) { v = { x = 20 }; }; let p = { x = 1; y = \"hello\" }; edit(&p); p",
            expect: Reject(""),
        },
        Case {
            id: "C2",
            source: "let edit = fn(v: &{ x: int, ... }) { v = { x = 20 }; }; let p: { x: int, ... } = { x = 1; y = \"hello\" }; edit(&p); p",
            expect: Value(&[], r#"{"x":20}"#),
        },
        Case {
            id: "C3",
            source: "let edit = fn(v: &{ x: int, ... }) { v.x = 20; }; let p: { x: int, ... } = { x = 1; y = \"hello\" }; edit(&p); p",
            expect: Value(&[], r#"{"x":20,"y":"hello"}"#),
        },
        Case {
            id: "C4",
            source: "let edit = fn(v: &{ x: int, ... }) { v.x = 20; }; let p: { x: int, ... } = { x = 1; y = \"hello\" }; edit(&p); p.y",
            expect: Reject("not known"),
        },
        Case {
            id: "C5",
            source: "let bump = fn(v: &{ x: int }) { v.x += 1; }; let p = { x = 1; y = \"s\" }; bump(&p); p",
            expect: Reject(""),
        },
        Case {
            id: "C6",
            source: "let bump = fn(v: &{ x: int }) { v.x += 1; }; let p = { x = 1 }; bump(&p); p",
            expect: Value(&[], r#"{"x":2}"#),
        },
        Case {
            id: "C7",
            source: "let bump = fn(v: &{ x: int }) { v.x += 1; }; let p: { ... } = { x = 1 }; bump(&p); p",
            expect: Reject(""),
        },
        Case {
            id: "C8",
            source: "let bump = fn(v: &{ x: int }) { v.x += 1; }; let p: { ... } = {}; p.x = 1; match p { { x: int } => { bump(&p); p.x }; else => 0; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "C8b",
            source: "let bump = fn(v: &{ x: int }) { v.x += 1; }; let p: { ... } = {}; p.x = 1; bump(&p); p.x",
            expect: Reject(""),
        },
        Case {
            id: "C9",
            source: "let f = fn(v: &{ x: int }) { v = { y = 1 }; }; 1",
            expect: Reject(""),
        },
        Case {
            id: "C10",
            source: "let f = fn(v: &{ x: int }) { v = { x = 1; y = 1 }; }; 1",
            expect: Reject(""),
        },
        Case {
            id: "C11",
            source: "let f = fn(v: &{ x: int, ... }) { v.y = 1; }; let p: { x: int, ... } = { x = 1 }; f(&p); p",
            expect: Value(&[], r#"{"x":1,"y":1}"#),
        },
        Case {
            id: "C12",
            source: "let f = fn(v: &{ x?: int }) { v = {}; }; let p: { x?: int } = { x = 1 }; f(&p); p.x",
            expect: Reject(""),
        },
        Case {
            id: "C13",
            source: "let f = fn(v: &{ x: int }) { v.x += 1; }; let p: { x?: int } = {}; p.x = 1; f(&p); p.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "C14",
            source: "let f = fn(v: &{ x?: int }) { v.x = 5; }; let p: { x?: int } = {}; f(&p); match p { { x: any; ... } => { p.x }; else => 0; }",
            expect: Value(&[], "5"),
        },
        Case {
            id: "C15",
            source: "let f = fn(v: &{ x?: int }) { v.x = 5; }; let p: { x?: int } = {}; f(&p); p.x",
            expect: Reject("optional"),
        },
        Case {
            id: "C16",
            source: "let f = fn(v: &[int]) { v[0] = 9; }; let xs = [1, 2]; f(&xs); xs",
            expect: Value(&[], "[9,2]"),
        },
        Case {
            id: "C17",
            source: "let f = fn(v: &int) { v += 1; }; let p = { a = 1; b = 1 / 0 }; f(&p.a); p.a",
            expect: Value(&[], "2"),
        },
        Case {
            id: "C18",
            source: "let f = fn(v: &{ x: int }) { v.x += 1; }; let q: any = { x = 1 }; match q { { x: int } => { f(&q); q.x }; else => 0; }",
            expect: Value(&[], "2"),
        },
    ]
}

fn rows() -> Vec<Case> {
    vec![
        Case {
            id: "R1",
            source: "BUMP let p = { count = 1; label = \"a\" }; bump(&p); p",
            expect: Value(&[], r#"{"count":2,"label":"a"}"#),
        },
        Case {
            id: "R2",
            source: "BUMP let p = { count = 1; label = \"a\" }; bump(&p); p.label",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "R3",
            source: "BUMP let p = { count = 1; label = \"a\" }; bump(&p); p == { count = 2; label = \"a\" }",
            expect: Value(&[], "true"),
        },
        Case {
            id: "R4",
            source: "BUMP let p: { count: int, ... } = { count = 1; hidden = \"h\" }; bump(&p); p",
            expect: Value(&[], r#"{"count":2,"hidden":"h"}"#),
        },
        Case {
            id: "R5",
            source: "let reset = fn<R>(v: &{ count: int, ...R }) { v = { ...v; count = 0 }; }; let p = { count = 1; label = \"a\" }; reset(&p); p",
            expect: Value(&[], r#"{"count":0,"label":"a"}"#),
        },
        Case {
            id: "R6",
            source: "let reset = fn<R>(v: &{ count: int, ...R }) { let rest: { ...R } = v; v = { ...rest; count = 0 }; }; let p = { count = 1; label = \"a\" }; reset(&p); p",
            expect: Value(&[], r#"{"count":0,"label":"a"}"#),
        },
        Case {
            id: "R7",
            source: "let reset = fn<R>(v: &{ count: int, ...R }) { v = { count = 0 }; }; 1",
            expect: Reject(""),
        },
        Case {
            id: "R8",
            source: "let f = fn<R>(v: &{ count: int, ...R }) { v.extra = 1; }; 1",
            expect: Reject(""),
        },
        Case {
            id: "R9",
            source: "BUMP let twice = fn<R>(v: &{ count: int, ...R }) { bump(&v); bump(&v); }; let p = { count = 1; label = \"a\" }; twice(&p); p",
            expect: Value(&[], r#"{"count":3,"label":"a"}"#),
        },
        Case {
            id: "R10",
            source: "BUMP let p = { count = 1; bad = 1 / 0 }; bump(&p); p.count",
            expect: Value(&[], "2"),
        },
        Case {
            id: "R11",
            source: "BUMP let p = { count = 1; f = fn(n: int) -> int { n * 2 } }; bump(&p); p.f(21)",
            expect: Value(&[], "42"),
        },
        Case {
            id: "R12",
            source: "BUMP let o = { inner = { count = 1; tag = \"t\" }; other = 7 }; bump(&o.inner); o",
            expect: Value(&[], r#"{"inner":{"count":2,"tag":"t"},"other":7}"#),
        },
        Case {
            id: "R13",
            source: "let f = fn<R>(v: &{ a: { x: int }, ...R }) { v.a.x += 1; }; let p = { a = { x = 1; y = 2 }; z = 3 }; f(&p); p",
            expect: Reject(""),
        },
        Case {
            id: "R14",
            source: "let f = fn<R>(v: &{ a: { x: int, ...R } }) { v.a.x += 1; }; let p = { a = { x = 1; y = 2 } }; f(&p); p",
            expect: Value(&[], r#"{"a":{"x":2,"y":2}}"#),
        },
        Case {
            id: "R15",
            source: "BUMP let p: { count: int; label?: str } = { count = 1; label = \"a\" }; bump(&p); match p { { label: any; ... } => { p.label }; else => \"none\"; }",
            expect: Value(&[], r#""a""#),
        },
        Case {
            id: "R16",
            source: "BUMP let p = { count = 1 }; let s = p; bump(&p); { before = s; after = p }",
            expect: Value(&[], r#"{"after":{"count":2},"before":{"count":1}}"#),
        },
    ]
}

/// Reads beside a live borrow, writeback liveness, and reborrows through parameters.
fn access() -> Vec<Case> {
    vec![
        Case {
            id: "X1",
            source: "let f = fn(x: &int, y: int) { x += y; }; let n = 1; f(&n, n); n",
            expect: Reject("overlaps"), // conservative one-active-writable-access rule (#C71)
        },
        Case {
            id: "X2",
            source: "let f = fn(x: &int, y: int) { x += y; }; let p = { x = 1; y = 10 }; f(&p.x, p.y); p",
            expect: Value(&[], r#"{"x":11,"y":10}"#),
        },
        Case {
            id: "X3",
            source: "let f = fn(x: &{ a: int }, y: int) { x.a += y; }; let p = { a = 1 }; f(&p, p.a); p",
            expect: Reject("overlaps"),
        },
        Case {
            id: "X4",
            source: "let f = fn(x: &int) -> int { x += 1; x }; let n = 1; let r = f(&n); r",
            expect: Value(&[], "2"), // a borrowed writeback is a returned field, not required to be consumed (#C71)
        },
        Case {
            id: "X5",
            source: "INC let g = fn(p: { a: int }) -> int { inc(&p.a); p.a }; g({ a = 1 })",
            expect: Value(&[], "2"),
        },
        Case {
            id: "X6",
            source: "INC let o = { a = { b = 1 } }; inc(&o.a.b); inc(&o.a.b); o.a.b",
            expect: Value(&[], "3"),
        },
        Case {
            id: "X7",
            source: "let f = fn(x: &int) -> int { x += 1; x }; let n = 1; if f(&n) > 1 { n } else { 0 }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "X8",
            source: "let f = fn(x: &int) -> int { x += 1; x }; let n = 1; 1 + f(&n) + n",
            expect: Reject(""),
        },
        Case {
            id: "X9",
            source: "INC let g = fn(xs: &[int]) { inc(&xs[0]); }; let a = [1]; g(&a); a",
            expect: Value(&[], "[2]"),
        },
        Case {
            id: "X10",
            source: "INC let o = { a = 1 }; let s = { ...o }; inc(&o.a); { s = s; o = o }",
            expect: Value(&[], r#"{"o":{"a":2},"s":{"a":1}}"#),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(basics());
    all.extend(places());
    all.extend(storage_rejections());
    all.extend(contracts());
    all.extend(rows());
    all.extend(access());
    for case in &mut all {
        if let Some(rest) = case.source.strip_prefix("INC ") {
            case.source = Box::leak(format!("{INC} {rest}").into_boxed_str());
        } else if let Some(rest) = case.source.strip_prefix("BUMP ") {
            case.source = Box::leak(format!("{BUMP} {rest}").into_boxed_str());
        }
    }
    all
}

fn table(prefix: char) -> Vec<Case> {
    all_cases()
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .collect()
}

#[test]
fn borrowed_scalars_and_eligibility() {
    run_table(&table('A'));
}
#[test]
fn places_nesting_disjointness_and_overlap() {
    run_table(&table('P'));
}
#[test]
fn borrows_cannot_be_stored_returned_or_captured() {
    run_table(&table('S'));
}
#[test]
fn admission_and_preservation_follow_the_signature() {
    run_table(&table('C'));
}
#[test]
fn generic_rows_preserve_and_rebuild_through_borrows() {
    run_table(&table('R'));
}
#[test]
fn reads_beside_borrows_and_writeback_liveness() {
    run_table(&table('X'));
}

fn outcome(source: &str, path: &[&str]) -> String {
    match Program::prepare(source) {
        Err(e) => format!("reject: {}", e.message),
        Ok(p) => match p.evaluate(path) {
            Ok(v) => format!("value: {v}"),
            Err(e) => format!("eval error: {}", e.message),
        },
    }
}

#[test]
fn formatter_round_trips_every_borrow_program() {
    let mut failures = Vec::new();
    for case in all_cases() {
        let path: &[&str] = match &case.expect {
            Value(path, _) | EvalError(path, _) => path,
            Reject(_) => &[],
        };
        let Ok(syntax) = Syntax::parse(case.source) else {
            continue;
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
        if reparsed.format() != once {
            failures.push(format!("{}: not idempotent\n{once}", case.id));
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
