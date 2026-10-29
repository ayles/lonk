//! Behavioral regression tests for receiver methods.
//! `self` is the reserved first parameter of a function stored in a record field; `fn(self)`
//! receives a snapshot, `fn(&self)` a writable borrow. A member call `o.m(args)` supplies the
//! receiver (auto-borrowing for `&self`); `o.m` alone is the unbound function. Inferred `self`
//! is the containing record's data fields plus a fresh generic remainder that carries the
//! methods, so copies and spread extensions keep calling the same code.

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

/// A counter with a writable method, a by-value method, and a poisoned sibling.
const COUNTER: &str = "let o = { x = 1; bad = 1 / 0; bump = fn(&self) { self.x += 1; }; with_x = fn(self, n: int) ({ ...self; x = n }) };";

fn member_calls() -> Vec<Case> {
    vec![
        Case {
            id: "M1",
            source: "COUNTER o.bump(); o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M2",
            source: "COUNTER o.bump(); o.bump(); o.x",
            expect: Value(&[], "3"),
        },
        Case {
            id: "M3",
            source: "COUNTER { changed = o.with_x(5).x; original = o.x }",
            expect: Value(&[], r#"{"changed":5,"original":1}"#),
        },
        Case {
            id: "M4",
            source: "let o = { x = 1; scale = fn(&self, k: int = 2) { self.x = self.x * k; } }; o.scale(); o.scale(3); o.x",
            expect: Value(&[], "6"),
        },
        Case {
            id: "M5",
            source: r#"let o = { label = "l"; bad = 1 / 0; name = fn(self) -> str { self.label } }; o.name()"#,
            expect: Value(&[], r#""l""#),
        },
        Case {
            id: "M6",
            source: "let o = { x = 1; double = fn(n: int) -> int { n * 2 }; grow = fn(&self) { self.x = self.double(self.x); } }; o.grow(); o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M7",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; }; twice = fn(&self) { self.bump(); self.bump(); } }; o.twice(); o.x",
            expect: Reject(""),
        },
        Case {
            id: "M7b",
            source: "let o = { x = 1; bump = fn(self: &{ x: int }) { self.x += 1; }; twice = fn<R>(self: &{ x: int; bump: fn(self: &{ x: int }), ...R }) { self.bump(); self.bump(); } }; o.twice(); o.x",
            expect: Reject("project"),
        },
        Case {
            id: "M7c",
            source: "let o = { x = 1; get = fn(self: { x: int; ... }) -> int { self.x }; twice = fn(self: { x: int; get: fn(self: { x: int; ... }) -> int; ... }) -> int { self.get() * 2 } }; o.twice()",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M8",
            source: "let o = { x = 1; label = \"l\"; bump = fn<R>(self: &{ x: int, ...R }) { self.x += 1; } }; o.bump(); { x = o.x; label = o.label }",
            expect: Value(&[], r#"{"label":"l","x":2}"#),
        },
        Case {
            id: "M9",
            source: "COUNTER let f = o.bump; f(&o); o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M10",
            source: "COUNTER (o.bump)(&o); o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M11a",
            source: "COUNTER o.bump(1); o.x",
            expect: Reject(""),
        },
        Case {
            id: "M11b",
            source: "COUNTER let f = o.bump; f(); o.x",
            expect: Reject(""),
        },
        Case {
            id: "M11c",
            source: "COUNTER o.with_x(); 1",
            expect: Reject(""),
        },
        Case {
            id: "M12",
            source: r#"COUNTER let p = { ...o; extra = "e" }; p.bump(); { px = p.x; extra = p.extra; ox = o.x }"#,
            expect: Value(&[], r#"{"extra":"e","ox":1,"px":2}"#),
        },
        Case {
            id: "M13",
            source: "COUNTER o.bump(); let q: { x: int } = o; q",
            expect: Value(&[], r#"{"x":2}"#),
        },
        Case {
            id: "M14",
            source: "let o = { x = 1; set = fn(&self, n: int) { self.x = n; } }; o.set(o.x + 1); o.x",
            expect: Reject("overlap"),
        },
        Case {
            id: "M15",
            source: "COUNTER let c = o; c.bump(); { o = o.x; c = c.x }",
            expect: Value(&[], r#"{"c":2,"o":1}"#),
        },
        Case {
            id: "M16",
            source: "COUNTER let f = fn() -> int { o.x }; o.bump(); { captured = f(); now = o.x }",
            expect: Value(&[], r#"{"captured":1,"now":2}"#),
        },
        Case {
            id: "M17",
            source: "let new_counter = fn(start: int) ({ n = start; inc = fn(&self) { self.n += 1; } }); let c = new_counter(5); c.inc(); c.n",
            expect: Value(&[], "6"),
        },
        Case {
            id: "M18",
            source: "let xs = [{ x = 1; bump = fn(&self) { self.x += 1; } }]; xs[0].bump(); xs[0].x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M19",
            source: "let s = { inner = { x = 1; bump = fn(&self) { self.x += 1; } }; other = 7 }; s.inner.bump(); { x = s.inner.x; other = s.other }",
            expect: Value(&[], r#"{"other":7,"x":2}"#),
        },
        Case {
            id: "M20",
            source: "let o = { x = 1; next = fn(self) -> int { self.x += 1; self.x } }; { r = o.next(); x = o.x }",
            expect: Value(&[], r#"{"r":2,"x":1}"#),
        },
        Case {
            id: "M21",
            source: "let f = fn(self) -> int { 1 }; f(1)",
            expect: Reject(""),
        },
        Case {
            id: "M22",
            source: "COUNTER let g = o.bump; o.bump = fn(&self) { self.x += 10; }; g(&o); o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M23",
            source: "let o = { x = 1; add = fn(&self, other: &int) { self.x += other; other = 0; } }; let n = 5; o.add(&n); { x = o.x; n = n }",
            expect: Value(&[], r#"{"n":0,"x":6}"#),
        },
        Case {
            id: "M24a",
            source: "let o = { x = 1; sum = fn(self, k: int = self.x) -> int { self.x + k } }; o.sum()",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M24b",
            source: "let o = { x = 1; grow = fn(&self, k: int = self.x) { self.x += k; } }; o.grow(); o.x",
            expect: Reject(""),
        },
        Case {
            id: "M25",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; o.bump(); o",
            expect: EvalError(&[], "only data"),
        },
        Case {
            id: "M26",
            source: "let xs = [{ x = 1; bump = fn(&self) { self.x += 1; } }]; for item in xs { item.bump(); }; xs[0].x",
            expect: Reject(""),
        },
        Case {
            id: "M27",
            source: "COUNTER let m = o.with_x; m(o, 9).x",
            expect: Value(&[], "9"),
        },
        Case {
            id: "M28",
            source: r#"let o = { x = 1; show = fn(self) -> str { "x" } }; let p = { ...o; x = "s" }; p.show()"#,
            expect: Reject(""),
        },
        Case {
            id: "M29",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let p: { x: int, ... } = o; p.bump(); p.x",
            expect: Reject(""),
        },
        Case {
            id: "M30",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; if true { o.bump(); } else { o.bump(); o.bump(); }; o.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M21b",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let f = fn(self) -> int { 1 }; { a = f(o); b = o.x }",
            expect: Reject(""),
        },
        Case {
            id: "M31",
            source: "let s = { inner = { x = 1; bump = fn(&self) { self.x += 1; } } }; { let a = &s.inner; a.bump(); }; s.inner.x",
            expect: Value(&[], "2"),
        },
        Case {
            id: "M32",
            source: "let xs = [{ x = 1; bump = fn(&self) { self.x += 1; } }, { x = 10; bump = fn(&self) { self.x += 1; } }]; let i = 1; xs[i].bump(); [xs[0].x, xs[1].x]",
            expect: Value(&[], "[1,11]"),
        },
        Case {
            id: "M33",
            source: "let o = { x = 1; same = fn(self) self }; o.same().x",
            expect: Value(&[], "1"),
        },
        Case {
            id: "M34",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; o.bump = fn(n: int) -> int { n }; o.x",
            expect: Reject(""),
        },
        Case {
            id: "M35",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let q: { x: int; bump: fn(self: &{ x: int }) } = o; let r: { x: int } = q; let f = q.bump; f(&r); r.x",
            expect: Value(&[], "2"),
        },
    ]
}

/// Quantified callable types inside type literals: borrowed sibling contracts, generic
/// callback fields, exact row constraints, binder scope and nesting.
fn quantified() -> Vec<Case> {
    vec![
        Case {
            id: "Q1",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; }; twice = fn<R>(self: &{ x: int; bump: fn<R2>(self: &{ x: int; ...R2 }); ...R }) { self.bump(); self.bump(); } }; let q: { x: int; bump: fn<R2>(self: &{ x: int; ...R2 }); twice: fn<R>(self: &{ x: int; bump: fn<R2>(self: &{ x: int; ...R2 }); ...R }) } = o; q.twice(); q.x",
            expect: Value(&[], "3"),
        },
        Case {
            id: "Q1b",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; }; twice = fn<R>(self: &{ x: int; bump: fn<R2>(self: &{ x: int; ...R2 }); ...R }) { self.bump(); self.bump(); } }; o.twice(); o.x",
            expect: Reject("project"),
        },
        Case {
            id: "Q1c",
            source: "let o = { x = 1; bump = fn<B>(self: &{ x: int; ...B }) { self.x += 1; }; twice = fn<R>(self: &{ x: int; bump: fn<B>(self: &{ x: int; ...B }); ...R }) { self.bump(); self.bump(); } }; o.twice(); o.x",
            expect: Value(&[], "3"),
        },
        Case {
            id: "Q2a",
            source: "let lib = { apply = fn(f: fn<T>(T) -> T, x: int) -> int { f(x) } }; lib.apply(fn<T>(v: T) -> T { v }, 5)",
            expect: Value(&[], "5"),
        },
        Case {
            id: "Q2b",
            source: "let lib = { apply = fn(f: fn<T>(T) -> T, x: int) -> int { f(x) } }; lib.apply(fn(v: int) -> int { v }, 5)",
            expect: Reject(""),
        },
        Case {
            id: "Q3",
            source: r#"let both = fn(f: fn<T>(T) -> T) -> { a: int; b: str } { ({ a = f(1); b = f("s") }) }; both(fn<T>(v: T) -> T { v })"#,
            expect: Value(&[], r#"{"a":1,"b":"s"}"#),
        },
        Case {
            id: "Q4",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let q: { x: int; bump: fn<R>(self: &{ x: int; y: int; ...R }) } = o; q.x",
            expect: Reject(""),
        },
        Case {
            id: "Q5",
            source: "let T = type { f: fn(self: &{ x: int; ...R }) }; 1",
            expect: Reject(""),
        },
        Case {
            id: "Q6",
            source: r#"let nest = fn<T>(f: fn<U>(U) -> U, x: T) -> T { f(x) }; nest(fn<U>(v: U) -> U { v }, "s")"#,
            expect: Value(&[], r#""s""#),
        },
        Case {
            id: "Q7",
            source: "let nest = fn<T>(f: fn<T>(T) -> T, x: T) -> T { f(x) }; nest(fn<T>(v: T) -> T { v }, 7)",
            expect: Value(&[], "7"),
        },
        Case {
            id: "Q8",
            source: r#"let id: fn<T>(T) -> T = fn<T>(v: T) -> T { v }; { a = id(1); b = id("s") }"#,
            expect: Value(&[], r#"{"a":1,"b":"s"}"#),
        },
        Case {
            id: "Q9",
            source: "let id: fn<T>(T) -> T = fn<T>(v: T) -> T { v }; id<int>(1)",
            expect: Value(&[], "1"),
        },
        Case {
            id: "Q10",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let q: { x: int; bump: fn<R>(self: { x: int; ...R }) } = o; q.x",
            expect: Reject(""),
        },
        Case {
            id: "Q11",
            source: "let o = { x = 1; bump = fn(&self) { self.x += 1; } }; let q: { x: int; bump: fn<R>(self: &{ x: int; ...R }) } = o; q.bump(); q.x",
            expect: Value(&[], "2"),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = member_calls();
    all.extend(quantified());
    for case in &mut all {
        if let Some(rest) = case.source.strip_prefix("COUNTER ") {
            case.source = Box::leak(format!("{COUNTER} {rest}").into_boxed_str());
        }
    }
    all
}

#[test]
fn receiver_methods_calls_copies_and_boundaries() {
    run_table(
        &all_cases()
            .into_iter()
            .filter(|c| !c.id.starts_with('Q'))
            .collect::<Vec<_>>(),
    );
}
#[test]
fn quantified_callable_contracts() {
    run_table(
        &all_cases()
            .into_iter()
            .filter(|c| c.id.starts_with('Q'))
            .collect::<Vec<_>>(),
    );
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
fn formatter_round_trips_every_method_program() {
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
