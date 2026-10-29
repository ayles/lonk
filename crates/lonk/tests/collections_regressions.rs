//! Behavioral regression tests for the collection surface.
//! Core is `xs[i]`, `xs[a:b]`, spread and a few intrinsics; everything else is prelude code in the
//! language internally. Primitive member lookup uses the fixed standard library, independently
//! of lexical namespace bindings. Objects use their own receiver fields.

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

/// S: slices.
const SLICES: &[Case] = &[
    Case {
        id: "S1 open-ended and empty slices",
        source: r#"let xs = [1, 2, 3]; [xs[1:], xs[:0], xs[3:], xs[1:1], xs[:]]"#,
        expect: Value(&[], "[[2,3],[],[],[],[1,2,3]]"),
    },
    Case {
        id: "S2 a slice demands structure and bounds, not payloads",
        source: r#"let xs = [1 / 0, 2, 1 / 0]; xs[1:2][0]"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "S3 start above end",
        source: r#"[1, 2][1:0]"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "S3b end above length",
        source: r#"[1, 2][0:3]"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "S3c negative bound",
        source: r#"let i = 0 - 1; [1, 2][i:]"#,
        expect: EvalError(&[], "nonnegative"),
    },
    Case {
        id: "S4 bounds must be int",
        source: r#"[1, 2]["a":]"#,
        expect: Reject(""),
    },
    Case {
        id: "S5 slicing an object",
        source: r#"let v = { a = 1 }; v[0:1]"#,
        expect: Reject("slicing requires"),
    },
    Case {
        id: "S6 a bound may be any int expression",
        source: r#"let xs = [4, 5, 6]; xs[xs.len() - 1:]"#,
        expect: Value(&[], "[6]"),
    },
    Case {
        id: "S7 a slice is a value copy",
        source: r#"let xs = [1, 2, 3]; let ys = xs[1:]; xs[1] = 9; [xs, ys]"#,
        expect: Value(&[], "[[1,9,3],[2,3]]"),
    },
    Case {
        id: "S8 the element type is preserved",
        source: r#"let xs: [int] = [1, 2]; let ys: [int] = xs[:1]; ys"#,
        expect: Value(&[], "[1]"),
    },
    Case {
        id: "S8b and checked",
        source: r#"let xs: [int] = [1, 2]; let ys: [str] = xs[:1]; ys"#,
        expect: Reject(""),
    },
    Case {
        id: "S9 slices in loops and generics",
        source: r#"let tail = fn<T>(xs: [T]) -> [T] { xs[1:] }; let total = 0; for x in tail([9, 1, 2]) { total += x; }; total"#,
        expect: Value(&[], "3"),
    },
    Case {
        id: "S10 string slices count scalar values",
        source: r#"["héllo"[1:3], "abc"[3:], "abc"[:0]]"#,
        expect: Value(&[], r#"["él","",""]"#),
    },
    Case {
        id: "S10b string slice bounds are strict too",
        source: r#""abc"[2:1]"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "S11 slicing the result of a call",
        source: r#"[3, 1, 2].reverse()[1:]"#,
        expect: Value(&[], "[1,3]"),
    },
    Case {
        id: "S12 string slices on literal and union string types",
        source: r#"let m: ("ab" | "cd") = "cd"; let n: [str] = [m[1:], "xy"[:1]]; n"#,
        expect: Value(&[], r#"["d","x"]"#),
    },
];

/// M: the member mechanism.
const MEMBERS: &[Case] = &[
    Case {
        id: "M1 member and namespace spellings are one call",
        source: r#"[[1, 2].len(), array.len([1, 2])]"#,
        expect: Value(&[], "[2,2]"),
    },
    Case {
        id: "M2 len does not force elements",
        source: r#"[1 / 0, 2].len()"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "M3 unknown member",
        source: r#"[1].nope()"#,
        expect: Reject("nope"),
    },
    Case {
        id: "M4 an object's own field wins; objects never dispatch to a namespace",
        source: r#"let v = { len = fn(self) -> int { 7 } }; v.len()"#,
        expect: Value(&[], "7"),
    },
    Case {
        id: "M4b an object without such a field gets no array member",
        source: r#"let v = { a = 1 }; v.len()"#,
        expect: Reject("len"),
    },
    Case {
        id: "M5 ordinary helper functions may use builtin members",
        source: r#"let sum = fn(xs: [int]) -> int { xs.fold(0, fn(a: int, b: int) -> int { a + b }) }; sum([1, 2, 3])"#,
        expect: Value(&[], "6"),
    },
    Case {
        id: "M6 shadowing the namespace cannot affect builtin members",
        source: r#"let array = 1; [array, [1].len()]"#,
        expect: Value(&[], "[1,1]"),
    },
    Case {
        id: "M7 lexical namespace calls do not override builtin members",
        source: r#"let f = fn(xs: [int]) { let array = { len = fn(xs: [int]) -> int { 99 } }; [array.len(xs), xs.len()] }; [f([1]), [1].len()]"#,
        expect: Value(&[], "[[99,1],1]"),
    },
    Case {
        id: "M8 a writing member needs a place",
        source: r#"[1, 2].push(3)"#,
        expect: Reject("place"),
    },
    Case {
        id: "M9 a writing member cannot borrow a captured binding",
        source: r#"let xs = [1]; let f = fn() -> int { xs.push(2); 1 }; f()"#,
        expect: Reject("captured"),
    },
    Case {
        id: "M10 a writing member respects the contract",
        source: r#"let xs: [int] = []; xs.push("s"); xs"#,
        expect: Reject(""),
    },
    Case {
        id: "M11 writing member on a field place",
        source: r#"let v = { items = [1] }; v.items.push(2); v.items"#,
        expect: Value(&[], "[1,2]"),
    },
    Case {
        id: "M12 writing member on an element place",
        source: r#"let g = [[1], [2]]; g[0].push(5); g"#,
        expect: Value(&[], "[[1,5],[2]]"),
    },
    Case {
        id: "M13 chains",
        source: r#"[3, 1, 2].reverse().take(2).map(fn(x: int) -> int { x * 10 })"#,
        expect: Value(&[], "[20,10]"),
    },
    Case {
        id: "M14 callback parameters are inferred from the member signature",
        source: r#"[1, 2].map(fn(x) x * 2)"#,
        expect: Value(&[], "[2,4]"),
    },
    Case {
        id: "M15 find stops at the first match",
        source: r#"[2, 1 / 0].find(fn(x: int) -> bool { x == 2 })"#,
        expect: Value(&[], r#"{"value":2}"#),
    },
    Case {
        id: "M16 first on empty is absence, recovered by match",
        source: r#"let r = [1].first(); let e: [int] = []; let s = e.first(); [match r { { value: any; } => r.value; else => 0; }, match s { { value: any; } => s.value; else => 0; }]"#,
        expect: Value(&[], "[1,0]"),
    },
    Case {
        id: "M17 string, int and bool members",
        source: r#"["abc".len(), "".len(), 42.to_str().len(), true.to_str().len(), (0 - 5).to_str().len()]"#,
        expect: Value(&[], "[3,0,2,4,2]"),
    },
    Case {
        id: "M18 a namespace function is a first-class value",
        source: r#"let n = array.len; let m = [1, 2].map(fn(x: int) -> int { x }); n(m)"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "M19 members on any are rejected",
        source: r#"let x: any = [1]; x.len()"#,
        expect: Reject(""),
    },
    Case {
        id: "M20 receiver indices must not mutate",
        source: r#"let i = 0; let xs = [[1], [2]]; xs[{ let j = 1; i = j; i }].push(3); xs"#,
        expect: Reject("must not mutate"),
    },
    Case {
        id: "M21 member on a literal-typed string binding",
        source: r#"let m: ("a" | "bb") = "bb"; m.len()"#,
        expect: Value(&[], "2"),
    },
    Case {
        id: "M22 value semantics of library results",
        source: r#"let xs = [1]; let ys = xs.map(fn(x: int) -> int { x }); xs[0] = 5; [xs, ys]"#,
        expect: Value(&[], "[[5],[1]]"),
    },
    Case {
        id: "M23 join on arrays",
        source: r#"[["a", "b"].join("/"), array.join([], "-")]"#,
        expect: Value(&[], r#"["a/b",""]"#),
    },
];

/// O: objects.
const OBJECTS: &[Case] = &[
    Case {
        id: "O1 remove drops the field and never forces payloads",
        source: r#"let v = { a = 1; z = 1 / 0 }; object.remove(v, "z")"#,
        expect: Value(&[], r#"{"a":1}"#),
    },
    Case {
        id: "O1b the result is open, so facts need a match",
        source: r#"let v = { a = 1; z = 2 }; object.remove(v, "z").a"#,
        expect: Reject(""),
    },
    Case {
        id: "O1c and a match recovers them",
        source: r#"let w = object.remove({ a = 1; z = 2 }, "z"); match w { { a: any; ... } => match w.a { int => w.a; else => 0; }; else => 0; }"#,
        expect: Value(&[], "1"),
    },
    Case {
        id: "O2 removing an absent key is idempotent",
        source: r#"object.remove({ a = 1 }, "q")"#,
        expect: Value(&[], r#"{"a":1}"#),
    },
    Case {
        id: "O3 dynamic key",
        source: r#"let k = "a"; object.remove({ a = 1; b = 2 }, k)"#,
        expect: Value(&[], r#"{"b":2}"#),
    },
    Case {
        id: "O4 the source binding is unchanged",
        source: r#"let v = { a = 1 }; let w = object.remove(v, "a"); [v, w]"#,
        expect: Value(&[], r#"[{"a":1},{}]"#),
    },
    Case {
        id: "O5 documented simplification: even an unrelated removal loses every field fact",
        source: r#"let x: { a: int; ... } = { a = 1; b = 2 }; x = object.remove(x, "b"); x"#,
        expect: Reject(""),
    },
    Case {
        id: "O6 lookup by dynamic key",
        source: r#"let v = { a = 1 }; let k = "a"; let r = object.lookup(v, k); let m = object.lookup(v, "zz"); [match r { { value: any; } => match r.value { int => r.value; else => 0; }; else => 0; }, match m { { value: any; } => 1; else => 0; }]"#,
        expect: Value(&[], "[1,0]"),
    },
    Case {
        id: "O7 with adds a lazy field",
        source: r#"object.lookup(object.with({ a = 1 / 0 }, "b", 2), "b")"#,
        expect: Value(&[], r#"{"value":2}"#),
    },
    Case {
        id: "O8 keys are lexical and never force payloads",
        source: r#"[object.keys({ b = 1 / 0; a = 2 }), object.keys({})]"#,
        expect: Value(&[], r#"[["a","b"],[]]"#),
    },
    Case {
        id: "O9 object operations need an object",
        source: r#"object.remove([1], "a")"#,
        expect: Reject(""),
    },
    Case {
        id: "O10 iterate keys",
        source: r#"let v = { a = 1; bb = 2 }; let n = 0; for k in object.keys(v) { n += k.len(); }; n"#,
        expect: Value(&[], "3"),
    },
    Case {
        id: "O11 remove cannot claim a captured row",
        source: r#"let f = fn<R>(v: { a: int; ...R }) -> { ...R } { object.remove(v, "a") }; f({ a = 1; b = 2 })"#,
        expect: Reject(""),
    },
    Case {
        id: "O12 len",
        source: r#"[object.len({ a = 1 / 0; b = 2 }), object.len({})]"#,
        expect: Value(&[], "[2,0]"),
    },
];

/// T: strings.
const STRINGS: &[Case] = &[
    Case {
        id: "T1 split keeps empty pieces",
        source: r#"["a,,b".split(","), "".split(","), "abc".split("x")]"#,
        expect: Value(&[], r#"[["a","","b"],[""],["abc"]]"#),
    },
    Case {
        id: "T2 splitting by the empty separator",
        source: r#""abc".split("")"#,
        expect: Value(&[], r#"["a","b","c"]"#),
    },
    Case {
        id: "T3 join forces every element",
        source: r#"[string.join([], "/"), string.join(["a", "b"], "")]"#,
        expect: Value(&[], r#"["","ab"]"#),
    },
    Case {
        id: "T3b join with a poisoned element",
        source: r#"string.join(["a", "abc"[5:]], "/")"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "T4 replace replaces every occurrence",
        source: r#"["aaa".replace("a", "b"), "abc".replace("zz", "y")]"#,
        expect: Value(&[], r#"["bbb","abc"]"#),
    },
    Case {
        id: "T5 replacing the empty needle matches Go and Python",
        source: r#""abc".replace("", "-")"#,
        expect: Value(&[], r#""-a-b-c-""#),
    },
    Case {
        id: "T6 contains, prefix, suffix with the empty string",
        source: r#"["abc".contains(""), "abc".starts_with(""), "".ends_with("a")]"#,
        expect: Value(&[], "[true,true,false]"),
    },
    Case {
        id: "T7 trim only strips ASCII whitespace",
        source: r#"[" \t x \t ".trim(), "x".trim()]"#,
        expect: Value(&[], r#"["x","x"]"#),
    },
    Case {
        id: "T8 string members are typed",
        source: r#""abc".contains(1)"#,
        expect: Reject(""),
    },
    Case {
        id: "T9 empty separator on empty and non-ASCII sources",
        source: r#"["".split(""), "é😀".split("")]"#,
        expect: Value(&[], r#"[[],["é","😀"]]"#),
    },
];

/// P: prelude algorithms.
const PRELUDE: &[Case] = &[
    Case {
        id: "P1 insert at both ends and out of range",
        source: r#"[[1, 2].insert(2, 3), [1, 2].insert(0, 0)]"#,
        expect: Value(&[], "[[1,2,3],[0,1,2]]"),
    },
    Case {
        id: "P1b insert out of range",
        source: r#"[1].insert(5, 0)"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "P2 remove_at last and out of range",
        source: r#"[1, 2].remove_at(1)"#,
        expect: Value(&[], "[1]"),
    },
    Case {
        id: "P2b remove_at out of range",
        source: r#"[1].remove_at(1)"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "P3 take and drop are strict",
        source: r#"[1].take(2)"#,
        expect: EvalError(&[], "slice requires"),
    },
    Case {
        id: "P4 splice takes a half-open range like slices",
        source: r#"[[1, 2, 3].splice(1, 2, [8, 9]), [1, 2, 3].splice(0, 0, [0]), [1, 2, 3].splice(3, 3, [4])]"#,
        expect: Value(&[], "[[1,8,9,3],[0,1,2,3],[1,2,3,4]]"),
    },
    Case {
        id: "P5 contains with a structural comparison",
        source: r#"[{ a = 1 }].contains({ a = 1 }, fn(x: { a: int }, y: { a: int }) -> bool { x.a == y.a })"#,
        expect: Value(&[], "true"),
    },
    Case {
        id: "P6 fold into a record",
        source: r#"[1, 2].fold({ s = 0 }, fn(acc: { s: int }, x: int) -> { s: int } { ({ s = acc.s + x }) })"#,
        expect: Value(&[], r#"{"s":3}"#),
    },
    Case {
        id: "P7 reverse and concat keep payloads lazy",
        source: r#"[[1 / 0, 2].reverse()[0], [1 / 0].concat([2])[1], [1 / 0].append(3)[1]]"#,
        expect: Value(&[], "[2,2,3]"),
    },
    Case {
        id: "P8 filter must force every predicate",
        source: r#"[1 / 0, 2].filter(fn(x: int) -> bool { x > 1 })"#,
        expect: EvalError(&[], ""),
    },
    Case {
        id: "P9 index_of on a late match",
        source: r#"let r = ["a", "b"].index_of("b", fn(x: str, y: str) -> bool { x == y }); match r { { index: any; } => r.index; else => 0 - 1; }"#,
        expect: Value(&[], "1"),
    },
    Case {
        id: "P10 push through a slice-built loop",
        source: r#"let out: [int] = []; for x in [5, 6, 7][1:] { out.push(x * 2); }; out"#,
        expect: Value(&[], "[12,14]"),
    },
    Case {
        id: "P11 an ordinary user-defined writing helper",
        source: r#"let clear = fn<T>(xs: &[T]) { xs = []; }; let xs = [1, 2]; let before = xs; clear(&xs); [before, xs]"#,
        expect: Value(&[], "[[1,2],[]]"),
    },
    Case {
        id: "P12 extend keeps the old snapshot",
        source: r#"let xs = [1]; let old = xs; xs.extend([2, 3]); [old, xs]"#,
        expect: Value(&[], "[[1],[1,2,3]]"),
    },
];

fn all_cases() -> Vec<&'static Case> {
    [SLICES, MEMBERS, OBJECTS, STRINGS, PRELUDE]
        .into_iter()
        .flatten()
        .collect()
}

#[test]
fn slices() {
    run_table(SLICES);
}

#[test]
fn member_mechanism() {
    run_table(MEMBERS);
}

#[test]
fn objects() {
    run_table(OBJECTS);
}

#[test]
fn strings() {
    run_table(STRINGS);
}

#[test]
fn prelude_algorithms() {
    run_table(PRELUDE);
}

#[test]
fn canonical_slice_and_member_forms() {
    let formatted = Syntax::parse("let xs = [1, 2];\n[xs[ 1 : 2 ], xs[ : ], xs[1 :], xs[: 1], xs . len(), \"a\".len(), 42.to_str()]")
        .unwrap()
        .format();
    for needle in [
        "xs[1:2]",
        "xs[:]",
        "xs[1:]",
        "xs[:1]",
        "xs.len()",
        "\"a\".len()",
        "42.to_str()",
    ] {
        assert!(formatted.contains(needle), "{needle}\n{formatted}");
    }
}

#[test]
fn formatter_round_trips_every_collections_program() {
    for case in all_cases() {
        let Ok(syntax) = Syntax::parse(case.source) else {
            continue;
        };
        let formatted = syntax.format();
        let again = Syntax::parse(&formatted)
            .unwrap_or_else(|e| panic!("{}: {}\n{formatted}", case.id, e.render(&formatted)))
            .format();
        assert_eq!(again, formatted, "{}", case.id);
        if let (Ok(a), Ok(b)) = (Program::prepare(case.source), Program::prepare(&formatted)) {
            assert_eq!(
                a.evaluate(&[]).map(|v| v.to_string()).ok(),
                b.evaluate(&[]).map(|v| v.to_string()).ok(),
                "{}",
                case.id
            );
        }
    }
}
