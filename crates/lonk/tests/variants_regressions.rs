//! Behavioral regression tests for the bounded variant slice.
//! Literal alternatives (`"a" | "b"`, `1 | 2`) and distinct required single-field record
//! alternatives, normalized as a set, admitted only when exactly one alternative applies,
//! matched exhaustively without `else` when the union is finite. No bindings, no null, no `?`.

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

const MODE: &str = r#"let Mode = type ("debug" | "release");"#;
const SOURCE: &str = "let Source = type ({ local: str } | { git: { url: str; revision: str } });";

fn literal_unions() -> Vec<Case> {
    vec![
        Case {
            id: "L1",
            source: r#"MODE let m: Mode = "debug"; match m { "debug" => 1; "release" => 2; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "L2",
            source: r#"MODE let m: Mode = "debug"; match m { "debug" => 1; }"#,
            expect: Reject(""),
        },
        Case {
            id: "L3",
            source: r#"MODE let m: Mode = "release"; match m { "debug" => 1; else => 2; }"#,
            expect: Value(&[], "2"),
        },
        Case {
            id: "L4",
            source: r#"MODE let m: Mode = "test"; m"#,
            expect: Reject(""),
        },
        Case {
            id: "L5",
            source: r#"MODE let s = "debug"; let m: Mode = s; m"#,
            expect: Reject(""),
        },
        Case {
            id: "L6",
            source: r#"MODE let name = fn(m: Mode) -> str { match m { "debug" => "d"; "release" => "r"; } }; name("release")"#,
            expect: Value(&[], r#""r""#),
        },
        Case {
            id: "L7",
            source: r#"MODE let Cfg = type { mode: Mode }; let c: Cfg = { mode = "release" }; c.mode"#,
            expect: Value(&[], r#""release""#),
        },
        Case {
            id: "L8",
            source: r#"let Small = type (1 | 2); let n: Small = 2; match n { 1 => "one"; 2 => "two"; }"#,
            expect: Value(&[], r#""two""#),
        },
        Case {
            id: "L9",
            source: r#"let Mixed = type ("a" | 1); let x: Mixed = 1; match x { "a" => "s"; 1 => "i"; }"#,
            expect: Value(&[], r#""i""#),
        },
        Case {
            id: "L10",
            source: r#"let D = type ("a" | "a"); let m: D = "a"; match m { "a" => 1; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "L11",
            source: r#"MODE let m: Mode = "debug"; let s: str = m; s + "!""#,
            expect: Value(&[], r#""debug!""#),
        },
        Case {
            id: "L12",
            source: r#"MODE let m: Mode = "debug"; let before = m; m = "release"; [match before { "debug" => 1; "release" => 2; }, match m { "debug" => 1; "release" => 2; }]"#,
            expect: Value(&[], "[1,2]"),
        },
        Case {
            id: "L13",
            source: r#"MODE let m: Mode = "debug"; m = "test"; m"#,
            expect: Reject(""),
        },
        Case {
            id: "L14",
            source: r#"MODE let xs: [Mode] = ["debug", "release"]; let out: [int] = []; for m in xs { out = [...out, match m { "debug" => 1; "release" => 2; }]; }; out"#,
            expect: Value(&[], "[1,2]"),
        },
        Case {
            id: "L15",
            source: r#"MODE let m: Mode = "debug"; match m { "debug" => 1; "release" => 2; "test" => 3; }"#,
            expect: Reject(""),
        },
        Case {
            id: "L16",
            source: r#"let b = true; match b { true => 1; false => 0; }"#,
            expect: Value(&[], "1"),
        },
    ]
}

fn record_alternatives() -> Vec<Case> {
    vec![
        Case {
            id: "R1",
            source: r#"SOURCE let s: Source = { git = { url = "u"; revision = "r" } }; match s { { local: str } => s.local; { git: { url: str; revision: str } } => s.git.url; }"#,
            expect: Value(&[], r#""u""#),
        },
        Case {
            id: "R2",
            source: r#"SOURCE let s: Source = { local = "p"; git = { url = "u"; revision = "r" } }; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R2b",
            source: r#"SOURCE let v = { local = "p"; git = { url = "u"; revision = "r" } }; let s: Source = v; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R3",
            source: r#"SOURCE let s: Source = { other = 1 }; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R4",
            source: r#"SOURCE let v = { local = "p"; extra = 1 }; let s: Source = v; s"#,
            expect: Value(&[], r#"{"local":"p"}"#),
        },
        Case {
            id: "R4b",
            source: r#"SOURCE let v = { local = "p"; git = { url = "u"; revision = "r" } }; let w: { local: str } = v; let s: Source = w; s"#,
            expect: Value(&[], r#"{"local":"p"}"#),
        },
        Case {
            id: "R5",
            source: r#"let Bad = type ({ git?: str; url: str } | { svn?: str; url: str }); 1"#,
            expect: Reject(""),
        },
        Case {
            id: "R6",
            source: r#"let Three = type ({ a: int } | { b: int } | { c: int }); let t: Three = { a = 1; c = 1 }; t"#,
            expect: Reject(""),
        },
        Case {
            id: "R6b",
            source: r#"let Three = type ({ a: int } | { b: int } | { c: int }); let t: Three = { b = 1 }; match t { { a: int } => 1; { b: int } => 2; { c: int } => 3; }"#,
            expect: Value(&[], "2"),
        },
        Case {
            id: "R7",
            source: r#"let Three = type ({ a: int } | { b: int } | { c: int }); let t: Three = { b = 1 }; match t { { a: int } => 1; { b: int } => 2; }"#,
            expect: Reject(""),
        },
        Case {
            id: "R8",
            source: r#"SOURCE let s: Source = { git = { url = "u"; revision = if 1 / 0 == 0 { "x" } else { "y" } } }; match s { { local: str } => s.local; { git: { url: str; revision: str } } => s.git.url; }"#,
            expect: Value(&[], r#""u""#),
        },
        Case {
            id: "R8b",
            source: r#"SOURCE let s: Source = { git = { url = "u"; revision = if 1 / 0 == 0 { "x" } else { "y" } } }; match s { { local: str } => s.local; { git: { url: str; revision: str } } => s.git.revision; }"#,
            expect: EvalError(&[], "division by zero"),
        },
        Case {
            id: "R9",
            source: r#"SOURCE let describe = fn(s: Source) -> str { match s { { local: str } => "local"; { git: { url: str; revision: str } } => "git"; } }; [describe({ local = "p" }), describe({ git = { url = "u"; revision = "r" } })]"#,
            expect: Value(&[], r#"["local","git"]"#),
        },
        Case {
            id: "R10",
            source: r#"SOURCE let Pkg = type { source: Source }; let p: Pkg = { source = { local = "p" } }; match p.source { { local: str } => p.source.local; { git: { url: str; revision: str } } => p.source.git.url; }"#,
            expect: Value(&[], r#""p""#),
        },
        Case {
            id: "R11",
            source: r#"SOURCE let a: any = { local = "p" }; let s: Source = a; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R11b",
            source: r#"SOURCE let a: any = { local = "p" }; match a { { local: str } => { let s: Source = a; match s { { local: str } => s.local; { git: { url: str; revision: str } } => s.git.url; } }; else => "none"; }"#,
            expect: Value(&[], r#""p""#),
        },
        Case {
            id: "R12",
            source: r#"let Opt = type ("none" | { some: int }); let o: Opt = { some = 1 }; match o { "none" => 0; { some: int } => o.some; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "R12b",
            source: r#"let Opt = type ("none" | { some: int }); let o: Opt = "none"; match o { "none" => 0; { some: int } => o.some; }"#,
            expect: Value(&[], "0"),
        },
        Case {
            id: "R13",
            source: r#"SOURCE let s: Source = { local = "p" }; let before = s; s = { local = "x" }; [match before { { local: str } => before.local; { git: { url: str; revision: str } } => before.git.url; }, match s { { local: str } => s.local; { git: { url: str; revision: str } } => s.git.url; }]"#,
            expect: Value(&[], r#"["p","x"]"#),
        },
        Case {
            id: "R13b",
            source: r#"SOURCE let s: Source = { local = "p" }; s = { other = 1 }; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R14",
            source: r#"let A = type ({ a: int } | { b: int }); let B = type ({ b: int } | { a: int }); let x: A = { a = 1 }; let y: B = x; match y { { a: int } => y.a; { b: int } => y.b; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "R15",
            source: r#"let Open = type ({ a: int, ... } | { b: int }); 1"#,
            expect: Reject(""),
        },
        Case {
            id: "R16",
            source: r#"let Same = type ({ a: int } | { a: str }); 1"#,
            expect: Reject(""),
        },
        Case {
            id: "R17",
            source: r#"let a: any = "x"; [match a { "x" => 1; str => 2; else => 0; }, match 5 { "x" => 1; str => 2; else => 0; }]"#,
            expect: Value(&[], "[1,0]"),
        },
        Case {
            id: "R18",
            source: r#"SOURCE let s: Source = { local = "p" }; s"#,
            expect: Value(&[], r#"{"local":"p"}"#),
        },
        Case {
            id: "R19",
            source: r#"SOURCE let s: Source = { local = 1 }; s"#,
            expect: Reject(""),
        },
        Case {
            id: "R20",
            source: r#"SOURCE let s: Source = { git = { url = "u" } }; s"#,
            expect: Reject(""),
        },
    ]
}

fn exhaustiveness_and_dynamic() -> Vec<Case> {
    vec![
        Case {
            id: "E1",
            source: r#"let a: any = 1; match a { int => 1; str => 2; }"#,
            expect: Reject(""),
        },
        Case {
            id: "E2",
            source: r#"let s = "x"; match s { "x" => 1; "y" => 2; }"#,
            expect: Reject(""),
        },
        Case {
            id: "E3",
            source: r#"let s = "x"; match s { "x" => 1; else => 2; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "E4",
            source: r#"MODE let m: Mode = "debug"; match m { "debug" => 1; "release" => 2; else => 3; }"#,
            expect: Value(&[], "1"),
        },
        Case {
            id: "E5",
            source: r#"MODE let classify = fn(m: Mode) -> int { match m { "debug" => 1; "release" => 2; } }; let a: any = "release"; match a { "debug" => classify("debug"); "release" => classify("release"); else => 0; }"#,
            expect: Value(&[], "2"),
        },
    ]
}

fn all_cases() -> Vec<Case> {
    let mut all = Vec::new();
    all.extend(literal_unions());
    all.extend(record_alternatives());
    all.extend(exhaustiveness_and_dynamic());
    for case in &mut all {
        if let Some(rest) = case.source.strip_prefix("MODE ") {
            case.source = Box::leak(format!("{MODE} {rest}").into_boxed_str());
        } else if let Some(rest) = case.source.strip_prefix("SOURCE ") {
            case.source = Box::leak(format!("{SOURCE} {rest}").into_boxed_str());
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
fn literal_unions_admit_and_match_exhaustively() {
    run_table(&table('L'));
}
#[test]
fn record_alternatives_admit_unambiguously() {
    run_table(&table('R'));
}
#[test]
fn exhaustiveness_and_dynamic_fallbacks() {
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
fn formatter_round_trips_every_variant_program() {
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
