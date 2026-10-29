//! Behavioral regression tests for scoped local aliases.
//! `let alias = &place;` names a writable location until the end of the enclosing block. Reading
//! the alias yields a snapshot, assigning writes the referent, `&alias` reborrows the same place,
//! and overlapping access through any other path is suspended while the alias is live.

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

fn lifetimes_and_writes() -> Vec<Case> {
    vec![
        Case {
            id: "A1",
            source: "let o = { nested = { value = 1 } }; if true { let v = &o.nested.value; v += 1; }; o.nested.value",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A2",
            source: "let o = { a = 1 }; let v = &o.a; v += 1; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A3",
            source: "let o = { a = 1; b = 1 }; if true { let v = &o.a; o.b = o.b + 1; v += 1; }; o",
            expect: Value(&[], r#"{"a":2,"b":2}"#),
        },
        Case {
            id: "A4",
            source: "let o = { a = 1 }; if true { let v = &o.a; o.a = 5; v += 1; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A5",
            source: "let o = { a = 1 }; if true { let v = &o.a; let s = o; v += 1; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A6",
            source: "let o = { a = 1 }; let out = 0; if true { let v = &o.a; let s = v; v += 1; out = s; }; { out = out; a = o.a }",
            expect: Value(&[], r#"{"a":2,"out":1}"#),
        },
        Case {
            id: "A7",
            source: "INC let o = { a = 1 }; if true { let v = &o.a; inc(&v); inc(&v); }; o.a",
            expect: Value(&[], "3"),
        },
        Case {
            id: "A8",
            source: "let o = { nested = { value = 1; other = 5 } }; if true { let v = &o.nested; let w = &v.value; w += 1; }; o.nested",
            expect: Value(&[], r#"{"other":5,"value":2}"#),
        },
        Case {
            id: "A9",
            source: "let o = { a = 1 }; if true { let v = &o.a; v = 5; }; o.a",
            expect: Value(&[], "5"),
        },
        Case {
            id: "A10",
            source: "let o = { a = 1; b = 2 }; if true { let v = &o.a; v = &o.b; v += 1; }; o",
            expect: Reject(""),
        },
        Case {
            id: "A11",
            source: "let o = { a = 1 }; if true { let v = &o.a; v += 1; let s = v; o.a = s; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A12",
            source: "let o = { a = 1 }; if true { let v = &o.a; v += 1; let s = v; v = s + 1; }; o.a",
            expect: Value(&[], "3"),
        },
        Case {
            id: "A13",
            source: "let o = { a = 1; b = 1 }; if true { let a = &o.a; let b = &o.b; a += 1; b += 10; }; o",
            expect: Value(&[], r#"{"a":2,"b":11}"#),
        },
        Case {
            id: "A14",
            source: "let o = { a = 1 }; if true { let a = &o.a; let b = &o.a; a += 1; b += 10; }; o",
            expect: Reject(""),
        },
        Case {
            id: "A15",
            source: "let o = { a = 1 }; if true { let v = &o.a; o = { a = 9 }; v += 1; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A16",
            source: "let o = { a = 1 }; for x in [1, 2] { let v = &o.a; v += x; }; o.a",
            expect: Value(&[], "4"),
        },
        Case {
            id: "A17",
            source: "let v = 1; let o = { a = 1 }; if true { let v = &o.a; v += 1; }; { v = v; a = o.a }",
            expect: Value(&[], r#"{"a":2,"v":1}"#),
        },
        Case {
            id: "A18",
            source: "let o = { a = 1; bad = 1 / 0 }; if true { let v = &o.a; v += 1; }; o.a",
            expect: Value(&[], "2"),
        },
        Case {
            id: "A19",
            source: "let o = { a = 1 }; if true { let v = &o.a; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "A20",
            source: "let o = { a = 1 }; let out = 0; if true { let v = &o.a; out = v + 1; }; { out = out; a = o.a }",
            expect: Value(&[], r#"{"a":1,"out":2}"#),
        },
    ]
}

fn places_and_indices() -> Vec<Case> {
    vec![
        Case {
            id: "P1",
            source: "let xs = [1, 2]; let i = 0; if true { let a = &xs[i]; i = 1; a += 10; }; { xs = xs; i = i }",
            expect: Value(&[], r#"{"i":1,"xs":[11,2]}"#),
        },
        Case {
            id: "P2",
            source: "INC let xs = [1, 2]; if true { let a = &xs[0]; inc(&xs[1]); a += 10; }; xs",
            expect: Value(&[], "[11,3]"),
        },
        Case {
            id: "P3",
            source: "INC let xs = [1, 2]; let j = 1; if true { let a = &xs[0]; inc(&xs[j]); a += 10; }; xs",
            expect: Value(&[], "[11,3]"),
        },
        Case {
            id: "P4",
            source: "INC let xs = [1, 2]; let j = 1; if true { let a = &xs[j]; j = 0; inc(&xs[j]); a += 10; }; xs",
            expect: Value(&[], "[2,12]"),
        },
        Case {
            id: "P4b",
            source: "INC let f = fn(xs: [int], i: int, j: int) -> [int] { { let a = &xs[i]; inc(&xs[j]); a += 10; }; xs }; f([1, 2], 0, 1)",
            expect: Reject("overlap"),
        },
        Case {
            id: "P5",
            source: "let xs = [1]; if true { let a = &xs[3]; a += 1; }; xs",
            expect: EvalError(&[], ""),
        },
        Case {
            id: "P6",
            source: "let o: { a?: int } = {}; if true { let v = &o.a; v += 1; }; o",
            expect: Reject(""),
        },
        Case {
            id: "P7",
            source: "let o: { ... } = {}; if true { let v = &o.a; v += 1; }; o",
            expect: Reject(""),
        },
        Case {
            id: "P8",
            source: "if true { let v = &1; v += 1; }; 1",
            expect: Reject(""),
        },
        Case {
            id: "P9",
            source: "let o = { a = { b = 1 } }; if true { let v = &o.a; v.b += 1; v.b += 1; }; o.a.b",
            expect: Value(&[], "3"),
        },
        Case {
            id: "P10",
            source: "let o = { a = { b = 1 } }; if true { let v = &o.a; v = { b = 7 }; }; o.a.b",
            expect: Value(&[], "7"),
        },
        Case {
            id: "P11",
            source: "let o = { a = { b = 1 } }; if true { let v = &o.a; v = { b = 7; c = 1 }; }; o.a",
            expect: Reject(""),
        },
    ]
}

fn escape_and_capture() -> Vec<Case> {
    vec![
        Case {
            id: "E1",
            source: "let o = { a = 1 }; if true { let v = &o.a; let r = { p = &v }; v += 1; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "E2",
            source: "let o = { a = 1 }; if true { let v = &o.a; let r = [&v]; v += 1; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "E3",
            source: "let f = fn(o: &{ a: int }) -> int { let v = &o.a; v += 1; v }; let o = { a = 1 }; { r = f(&o); a = o.a }",
            expect: Value(&[], r#"{"a":2,"r":2}"#),
        },
        Case {
            id: "E4",
            source: "let f = fn(o: &{ a: int }) -> &int { let v = &o.a; &v }; 1",
            expect: Reject(""),
        },
        Case {
            id: "E5",
            source: "let o = { a = 1 }; let out = 0; if true { let v = &o.a; let g = fn() -> int { v }; v += 1; out = g(); }; { out = out; a = o.a }",
            expect: Value(&[], r#"{"a":2,"out":1}"#),
        },
        Case {
            id: "E6",
            source: "let o = { a = 1 }; if true { let v = &o.a; let g = fn() { v += 1; }; }; o.a",
            expect: Reject(""),
        },
        Case {
            id: "E7",
            source: "let f = fn(p: { a: int }) -> int { { let v = &p.a; v += 1; }; p.a }; f({ a = 1 })",
            expect: Value(&[], "2"),
        },
        Case {
            id: "E8",
            source: "let f = fn<R>(v: &{ count: int, ...R }) { let c = &v.count; c += 1; }; let p = { count = 1; label = \"a\" }; f(&p); p",
            expect: Value(&[], r#"{"count":2,"label":"a"}"#),
        },
        Case {
            id: "E9",
            source: "let o = { a = 1 }; let v = if true { let w = &o.a; w += 1; w } else { 0 }; { v = v; a = o.a }",
            expect: Value(&[], r#"{"a":2,"v":2}"#),
        },
        Case {
            id: "E10",
            source: "let o = { a = 1 }; let out = 0; if true { let v = &o.a; { let v2 = &v; v2 += 1; }; out = v; }; { out = out; a = o.a }",
            expect: Value(&[], r#"{"a":2,"out":2}"#),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(lifetimes_and_writes());
    all.extend(places_and_indices());
    all.extend(escape_and_capture());
    for case in &mut all {
        if let Some(rest) = case.source.strip_prefix("INC ") {
            case.source = Box::leak(format!("{INC} {rest}").into_boxed_str());
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
fn alias_lifetimes_writes_and_suspension() {
    run_table(&table('A'));
}
#[test]
fn alias_places_and_stable_indices() {
    run_table(&table('P'));
}
#[test]
fn aliases_never_escape_but_reads_are_snapshots() {
    run_table(&table('E'));
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
fn formatter_round_trips_every_alias_program() {
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
