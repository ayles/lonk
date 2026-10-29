//! Behavioral regression tests for trailing positional defaults.
//! A default is a fresh lazy expression per call on a trailing value parameter. It may read
//! captured snapshots and the entry snapshots of earlier value parameters, never later or
//! borrowed parameters or itself. Availability is part of the callable signature.

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

fn evaluation() -> Vec<Case> {
    vec![
        Case {
            id: "D1",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; { x = f(1); y = f(1, 2) }",
            expect: Value(&[], r#"{"x":11,"y":3}"#),
        },
        Case {
            id: "D2",
            source: "let f = fn(a: int, b: int = a * 2) -> int { a + b }; f(3)",
            expect: Value(&[], "9"),
        },
        Case {
            id: "D3a",
            source: "let f = fn(a: int, b: int = 1 / 0) -> int { a + b }; f(1, 2)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "D3b",
            source: "let f = fn(a: int, b: int = 1 / 0) -> int { a + b }; f(1)",
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "D4",
            source: "let f = fn(a: int, b: int = 1 / 0) -> int { a }; f(1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "D5",
            source: "let k = 1; let f = fn(a: int = k) -> int { a }; k = 5; { call = f(); k = k }",
            expect: Value(&[], r#"{"call":1,"k":5}"#),
        },
        Case {
            id: "D6",
            source: "let f = fn(a: int, b: int = 1, c: int = b + a) -> int { a + b + c }; { x = f(1); y = f(1, 2); z = f(1, 2, 3) }",
            expect: Value(&[], r#"{"x":4,"y":6,"z":6}"#),
        },
        Case {
            id: "D7",
            source: "let f = fn(a: int = 1) -> int { a }; { x = f(); y = f(5) }",
            expect: Value(&[], r#"{"x":1,"y":5}"#),
        },
        Case {
            id: "D8",
            source: "let f = fn(p: { n: int } = { n = 0 }) -> { n: int } { p.n += 1; p }; [f().n, f().n]",
            expect: Value(&[], "[1,1]"),
        },
        Case {
            id: "D9",
            source: "let f = fn(a: int, b: int = a) -> int { a = a + 10; a + b }; f(1)",
            expect: Value(&[], "12"),
        },
        Case {
            id: "D10",
            source: "let q = { x = 1; y = 2 }; let f = fn(p: { x: int } = q) -> { x: int } { p }; f()",
            expect: Value(&[], r#"{"x":1}"#),
        },
        Case {
            id: "D11",
            source: "let rep = fn<T>(x: T, times: int = 2) -> [T] { if times == 1 { [x] } else { [x, x] } }; { a = rep(1); b = rep(\"s\", 1) }",
            expect: Value(&[], r#"{"a":[1,1],"b":["s"]}"#),
        },
        Case {
            id: "D12",
            source: "let pair = fn<T>(x: T, y: T = x) -> [T] { [x, y] }; pair(1)",
            expect: Value(&[], "[1,1]"),
        },
        Case {
            id: "D13",
            source: "let lib = { f = fn(a: int, b: int = 2) -> int { a * b } }; lib.f(3)",
            expect: Value(&[], "6"),
        },
        Case {
            id: "D14",
            source: "let g = fn(x: int, f: fn(int) -> int = fn(n: int) -> int { n + 1 }) -> int { f(x) }; g(1)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "D15",
            source: "let count = fn(n: int, acc: int = 0) -> int { if n == 0 { acc } else { count(n - 1, acc + 1) } }; count(5)",
            expect: Value(&[], "5"),
        },
        Case {
            id: "D16",
            source: "let add = fn(a: int, b: int = 1) -> int { a + b }; let xs = [1, 2]; let out: [int] = []; for x in xs { out = [...out, add(x)]; }; out",
            expect: Value(&[], "[2,3]"),
        },
        Case {
            id: "D17",
            source: "let f = fn(a: int, b: int = a + 1, c: int = b + 1) -> int { c }; f(1)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "D18",
            source: "let f = fn(a: int, b: int = 1 / 0) -> int { a }; let g = f; g(1)",
            expect: Value(&[], "1"),
        },
    ]
}

fn rejections() -> Vec<Case> {
    vec![
        Case {
            id: "R1",
            source: "let f = fn(a: int = 1, b: int) -> int { a + b }; f(1, 2)",
            expect: Reject(""),
        },
        Case {
            id: "R2",
            source: "let f = fn(a: int, b: int = c, c: int = 1) -> int { a + b + c }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "R3",
            source: "let f = fn(a: int, b: int = b) -> int { a + b }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "R4",
            source: "let f = fn(a: &int = 1) { a += 1; }; let n = 1; f(); n",
            expect: Reject(""),
        },
        Case {
            id: "R5",
            source: "let f = fn(a: int, b: int = 1) -> int { a + b }; f()",
            expect: Reject(""),
        },
        Case {
            id: "R6",
            source: "let f = fn(a: int, b: int = 1) -> int { a + b }; f(1, 2, 3)",
            expect: Reject(""),
        },
        Case {
            id: "R7",
            source: r#"let f = fn(a: int, b: int = "s") -> int { a + b }; f(1)"#,
            expect: Reject(""),
        },
        Case {
            id: "R8",
            source: "let f = fn(a: int, p: { x: int } = { x = 1; typo = 2 }) -> int { a + p.x }; f(1)",
            expect: Reject("unexpected fields"),
        },
        Case {
            id: "R9",
            source: "let inc = fn(x: &int) { x += 1; }; let f = fn(a: int, b: int = { inc(&a); a }) -> int { a + b }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "R10",
            source: "let f = fn(a: int, b = 2) -> int { a + b }; f(1)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "R11",
            source: "let f = fn(a: int, b: int = 1) -> int { a + b }; f(1, 2); 1",
            expect: Reject("unused"),
        },
        Case {
            id: "R12",
            source: "let f = fn(a: int, b: int = a) { }; 1",
            expect: Reject(""),
        },
        Case {
            id: "R13",
            source: "let f = fn(a: int, b: int = f(0)) -> int { a + b }; f(1)",
            expect: Reject(""),
        },
    ]
}

fn signatures() -> Vec<Case> {
    vec![
        Case {
            id: "S1",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; let g: fn(int, int) -> int = f; g(1, 2)",
            expect: Value(&[], "3"),
        },
        Case {
            id: "S2",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; let g: fn(int, int) -> int = f; g(1)",
            expect: Reject(""),
        },
        Case {
            id: "S3",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; let h: fn(int, int = ...) -> int = f; h(1)",
            expect: Value(&[], "11"),
        },
        Case {
            id: "S4",
            source: "let h: fn(int, int = ...) -> int = fn(a: int, b: int) -> int { a + b }; h(1)",
            expect: Reject(""),
        },
        Case {
            id: "S5",
            source: "let apply = fn(f: fn(int) -> int, x: int) -> int { f(x) }; apply(fn(a: int, b: int = 1) -> int { a + b }, 1)",
            expect: Reject(""),
        },
        Case {
            id: "S6",
            source: "let apply = fn(f: fn(int, int = ...) -> int, x: int) -> int { f(x) }; apply(fn(a: int, b: int = 5) -> int { a + b }, 1)",
            expect: Value(&[], "6"),
        },
        Case {
            id: "S7",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; let T = type fn(int, int = ...) -> int; let g: T = f; g(1, 1)",
            expect: Value(&[], "2"),
        },
        Case {
            id: "S8",
            source: "let f = fn(a: int, b: int = 10) -> int { a + b }; let p: any = f; match p { fn(int, int = ...) -> int => 1; else => 0; }",
            expect: Reject(""),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(evaluation());
    all.extend(rejections());
    all.extend(signatures());
    all
}

fn table(prefix: char) -> Vec<Case> {
    all_cases()
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .collect()
}

#[test]
fn defaults_evaluate_lazily_per_call() {
    run_table(&table('D'));
}
#[test]
fn invalid_defaults_are_rejected() {
    run_table(&table('R'));
}
#[test]
fn availability_lives_in_the_signature() {
    run_table(&table('S'));
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
fn formatter_round_trips_every_default_program() {
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
