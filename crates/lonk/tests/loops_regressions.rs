//! Behavioral regression tests for finite array loops.
//! `for item in array { ... }` is a statement: the source structure is a snapshot, payloads stay
//! lazy, each iteration is its own frame, and the loop's outputs are versioned accumulators.

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

const INC: &str = "let inc = fn(x: &int) { x += 1; };";
const BUMP: &str = "let bump = fn<R>(v: &{ count: int, ...R }) { v.count += 1; };";

fn accumulation() -> Vec<Case> {
    vec![
        Case {
            id: "A1",
            source: "let total = 0; for x in [1, 2, 3] { total = total + x; }; total",
            expect: Value(&[], "6"),
        },
        Case {
            id: "A2",
            source: "let out: [int] = []; for x in [1, 2] { out = [...out, x * 10]; }; out",
            expect: Value(&[], "[10,20]"),
        },
        Case {
            id: "A3",
            source: "let count = 0; for x in [1 / 0, 2] { count = count + 1; }; count",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A4",
            source: "let total = 0; for x in [1 / 0, 2] { total = total + x; }; total",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "A5",
            source: "let e: [int] = []; let total = 0; for x in e { total = total + 1; }; total",
            expect: Value(&[], "0"),
        },
        Case {
            id: "A6",
            source: "let total = 0; for x in [1, 2] { for y in [10, 20] { total = total + x * y; }; }; total",
            expect: Value(&[], "90"),
        },
        Case {
            id: "A7",
            source: "let total = 0; for x in [1, 2, 3] { if x > 1 { total = total + x; }; }; total",
            expect: Value(&[], "5"),
        },
        Case {
            id: "A8",
            source: "let log: [int] = []; for x in [3, 1, 2] { log = [...log, x]; }; log",
            expect: Value(&[], "[3,1,2]"),
        },
        Case {
            id: "A9",
            source: "let xs = [1, 2]; let out: [int] = []; for x in xs { xs = [...xs, 3]; out = [...out, x]; }; { xs = xs; out = out }",
            expect: Value(&[], r#"{"out":[1,2],"xs":[1,2,3,3]}"#),
        },
        Case {
            id: "A10",
            source: "let total = 0; for x in [1, 2] { total = total + x; }; for x in [10] { total = total + x; }; total",
            expect: Value(&[], "13"),
        },
        Case {
            id: "A11",
            source: "let total = 0; let xs = [1, 2]; for x in xs { total = total + x; }; { total = total; xs = xs }",
            expect: Value(&[], r#"{"total":3,"xs":[1,2]}"#),
        },
    ]
}

fn facts_and_contracts() -> Vec<Case> {
    vec![
        Case {
            id: "F1",
            source: "let p: { ... } = {}; for x in [1] { p.x = x; }; p.x",
            expect: Reject("optional"),
        },
        Case {
            id: "F2",
            source: "let p: { ... } = {}; for x in [1] { p.x = x; }; match p { { x: any; ... } => { p.x }; else => 0; }",
            expect: Value(&[], "1"),
        },
        Case {
            id: "F3",
            source: r#"let v = 1; for x in [1] { v = "s"; }; v"#,
            expect: Reject(""),
        },
        Case {
            id: "F4",
            source: r#"let v: { ... } = {}; v.x = 1; for x in [1] { v.x = "s"; }; v"#,
            expect: Reject(""),
        },
        Case {
            id: "F5",
            source: "let p: { ... } = {}; for x in [1, 2, 3] { match p { { n: int; ... } => { p.n = p.n + 1; }; else => { p.n = 1; }; }; }; match p { { n: int; ... } => { p.n }; else => 0; }",
            expect: Value(&[], "3"),
        },
        Case {
            id: "F6",
            source: "let p: { n?: int } = {}; for x in [1, 2] { p.n = x; }; match p { { n: any; ... } => { p.n }; else => 0; }",
            expect: Value(&[], "2"),
        },
        Case {
            id: "F7",
            source: "let p = { n = 0 }; for x in [1, 2] { p.n = p.n + x; }; p.n",
            expect: Value(&[], "3"),
        },
        Case {
            id: "F8",
            source: "let a: any = 1; let total = 0; for x in [1] { match a { int => { total = total + a; }; else => { }; }; }; total",
            expect: Value(&[], "1"),
        },
        Case {
            id: "F9",
            source: "let total = 0; for x in [1, 2] { total = total + x; total = total * 2; }; total",
            expect: Value(&[], "8"),
        },
    ]
}

fn binders_and_scope() -> Vec<Case> {
    vec![
        Case {
            id: "S1",
            source: "let x = 10; let total = 0; for x in [1, 2] { total = total + x; }; total + x",
            expect: Value(&[], "13"),
        },
        Case {
            id: "S2",
            source: "let total = 0; for x in [1, 2] { total = total + x; }; x",
            expect: Reject("unknown name"),
        },
        Case {
            id: "S3",
            source: "let xs = [{ a = 1 }]; for x in xs { x.a = 5; }; xs",
            expect: Reject(""),
        },
        Case {
            id: "S4",
            source: "let fs: [fn() -> int] = []; for x in [1, 2] { fs = [...fs, fn() -> int { x }]; }; [fs[0](), fs[1]()]",
            expect: Value(&[], "[1,2]"),
        },
        Case {
            id: "S5",
            source: "let total = 0; for x in [1, 2] { let y = x * 2; total = total + y; }; total",
            expect: Value(&[], "6"),
        },
        Case {
            id: "S6",
            source: "let total = 0; for x in [1] { let total = 5; }; total",
            expect: Reject(""),
        },
        Case {
            id: "S7",
            source: "let total = 0; let f = fn() -> int { for x in [1] { total = total + x; }; total }; f()",
            expect: Reject("captured"),
        },
        Case {
            id: "S8",
            source: "let total = 0; for x in [1, 2] { total = total + x; }; let total = total * 10; total",
            expect: Value(&[], "30"),
        },
    ]
}

fn borrows_in_loops() -> Vec<Case> {
    vec![
        Case {
            id: "B1",
            source: "INC let n = 0; for x in [1, 2] { inc(&n); }; n",
            expect: Value(&[], "2"),
        },
        Case {
            id: "B2",
            source: "BUMP let p = { count = 0; label = \"a\" }; for x in [1, 2, 3] { bump(&p); }; p",
            expect: Value(&[], r#"{"count":3,"label":"a"}"#),
        },
        Case {
            id: "B3",
            source: "let push = fn<T>(xs: &[T], x: T) { xs = [...xs, x]; }; let out: [int] = []; for x in [1, 2] { push(&out, x); }; out",
            expect: Value(&[], "[1,2]"),
        },
        Case {
            id: "B4",
            source: "INC let o = { a = 0; b = 0 }; for x in [1, 2] { inc(&o.a); }; o",
            expect: Value(&[], r#"{"a":2,"b":0}"#),
        },
        Case {
            id: "B5",
            source: "let add = fn(acc: &int, xs: [int]) { for x in xs { acc += x; }; }; let n = 0; add(&n, [1, 2, 3]); n",
            expect: Value(&[], "6"),
        },
    ]
}

fn liveness_and_misuse() -> Vec<Case> {
    vec![
        Case {
            id: "U1",
            source: "let total = 0; for x in [1, 2] { total = total + x; }; 42",
            expect: Reject("unused"),
        },
        Case {
            id: "U2",
            source: "for x in [1] { x }",
            expect: Reject(""),
        },
        Case {
            id: "U3",
            source: "for x in [1] { }; 1",
            expect: Reject(""),
        },
        Case {
            id: "U4",
            source: "let a: any = [1]; let total = 0; for x in a { total = total + 1; }; total",
            expect: Reject(""),
        },
        Case {
            id: "U5",
            source: "let total = 0; for x in { a = 1 } { total = total + 1; }; total",
            expect: Reject(""),
        },
        Case {
            id: "U6",
            source: "let total = 0; for x in [1, 2] { if x == 1 { break; }; total = total + x; }; total",
            expect: Reject(""),
        },
        Case {
            id: "U7",
            source: "let total = 0; for x in [1] { let dead = x; total = total + 1; }; total",
            expect: Reject("unused"),
        },
        Case {
            id: "U8",
            source: "let total = 0; for x in [1, 2] { total = total + x; }; total",
            expect: Value(&[], "3"),
        },
    ]
}

fn generic_algorithms() -> Vec<Case> {
    vec![
        Case {
            id: "G1",
            source: "let fold = fn<T, A>(xs: [T], init: A, f: fn(A, T) -> A) -> A { let acc = init; for x in xs { acc = f(acc, x); }; acc }; fold([1, 2, 3], 0, fn(a: int, x: int) -> int { a + x })",
            expect: Value(&[], "6"),
        },
        Case {
            id: "G2",
            source: "let map = fn<T, U>(xs: [T], f: fn(T) -> U) -> [U] { let out: [U] = []; for x in xs { out = [...out, f(x)]; }; out }; map([1, 2], fn(x: int) -> int { x * 2 })",
            expect: Value(&[], "[2,4]"),
        },
        Case {
            id: "G3",
            source: "let map = fn<T, U>(xs: [T], f: fn(T) -> U) -> [U] { let out: [U] = []; for x in xs { out = [...out, f(x)]; }; out }; map([1, 1 / 0], fn(x: int) -> int { x * 2 })[0]",
            expect: Value(&[], "2"),
        },
        Case {
            id: "G4",
            source: "let filter = fn<T>(xs: [T], keep: fn(T) -> bool) -> [T] { let out: [T] = []; for x in xs { if keep(x) { out = [...out, x]; }; }; out }; filter([1, 2, 3, 4], fn(x: int) -> bool { x % 2 == 0 })",
            expect: Value(&[], "[2,4]"),
        },
        Case {
            id: "G5",
            source: "let names = fn<R>(ps: [{ name: str, ...R }]) -> [str] { let out: [str] = []; for p in ps { out = [...out, p.name]; }; out }; names([{ name = \"a\"; v = 1 }, { name = \"b\"; v = 2 }])",
            expect: Value(&[], r#"["a","b"]"#),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(accumulation());
    all.extend(facts_and_contracts());
    all.extend(binders_and_scope());
    all.extend(borrows_in_loops());
    all.extend(liveness_and_misuse());
    all.extend(generic_algorithms());
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
fn accumulation_order_and_laziness() {
    run_table(&table('A'));
}
#[test]
fn facts_across_iterations_and_contracts() {
    run_table(&table('F'));
}
#[test]
fn binders_shadowing_and_frames() {
    run_table(&table('S'));
}
#[test]
fn borrows_inside_loops() {
    run_table(&table('B'));
}
#[test]
fn liveness_and_misuse_are_rejected() {
    run_table(&table('U'));
}
#[test]
fn generic_algorithms_over_arrays() {
    run_table(&table('G'));
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
fn formatter_round_trips_every_loop_program() {
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
