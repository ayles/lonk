//! Behavioral regression tests for structural sharing.
//! Sharing must be invisible: every snapshot keeps its value no matter how many later versions
//! share its nodes, payloads stay lazy through copies, spreads, slices and projections, and the
//! cost of building and reading large values no longer grows quadratically.

use lonk::Program;
use std::time::Instant;

#[allow(dead_code)]
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
                "evaluated to {v} but expected an error containing {needle:?}"
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

/// S: snapshots stay independent through shared structure.
const SNAPSHOTS: &[Case] = &[
    Case {
        id: "S1 a copy diverges from its source",
        source: r#"let a = { x = 1; y = 2 }; let b = a; b.x = 5; [a.x, b.x, a.y, b.y]"#,
        expect: Value(&[], "[1,5,2,2]"),
    },
    Case {
        id: "S2 nested path copies keep sibling subtrees and every version",
        source: r#"let a = { i = { z = 1 }; j = { z = 2 } }; let b = a; b.i.z = 9; let c = b; c.j.z = 8; [a.i.z, a.j.z, b.i.z, b.j.z, c.i.z, c.j.z]"#,
        expect: Value(&[], "[1,2,9,2,9,8]"),
    },
    Case {
        id: "S3 arrays copied, appended and written",
        source: r#"let xs = [1, 2, 3]; let ys = xs; ys[1] = 9; let zs = [...xs, 4]; xs[0] = 7; [xs, ys, zs]"#,
        expect: Value(&[], "[[7,2,3],[1,9,3],[1,2,3,4]]"),
    },
    Case {
        id: "S4 a slice shares then diverges both ways",
        source: r#"let xs = [1, 2, 3, 4]; let m = xs[1:3]; xs[1] = 0; m[0] = 5; [xs, m]"#,
        expect: Value(&[], "[[1,0,3,4],[5,3]]"),
    },
    Case {
        id: "S5 spread chains keep every layer",
        source: r#"let base = { a = 1; b = 2 }; let d1 = { ...base; c = 3 }; let d2 = { ...d1; a = 0 }; base.a = 9; [base.a, d1.a, d2.a, d2.c, d1.b]"#,
        expect: Value(&[], "[9,1,0,3,2]"),
    },
    Case {
        id: "S6 removal and insertion copies leave the source alone",
        source: r#"let v = { a = 1; b = 2; c = 3 }; let w = object.remove(v, "b"); let u = object.with(w, "d", 4); [object.keys(v), object.keys(w), object.keys(u)]"#,
        expect: Value(&[], r#"[["a","b","c"],["a","c"],["a","c","d"]]"#),
    },
    Case {
        id: "S7 borrowed writes after a snapshot",
        source: r#"let f = fn(v: &{ n: int; items: [int] }) { v.items.push(v.n); v.n += 1; }; let s: { n: int; items: [int] } = { n = 1; items = [] }; let before = s; f(&s); f(&s); [before, s]"#,
        expect: Value(&[], r#"[{"items":[],"n":1},{"items":[1,2],"n":3}]"#),
    },
    Case {
        id: "S8 per-iteration snapshots of a growing accumulator",
        source: r#"let acc: [int] = []; let seen: [[int]] = []; for x in [1, 2, 3] { acc = [...acc, x]; seen = [...seen, acc]; }; seen"#,
        expect: Value(&[], "[[1],[1,2],[1,2,3]]"),
    },
    Case {
        id: "S9 writing an element of a shared nested array",
        source: r#"let a = { xs = [1, 2] }; let b = a; b.xs[1] = 9; [a.xs, b.xs]"#,
        expect: Value(&[], "[[1,2],[1,9]]"),
    },
    Case {
        id: "S10 loop over a slice while writing the source binding",
        source: r#"let xs = [1, 2, 3]; for x in xs[1:] { xs[0] += x; }; xs"#,
        expect: Value(&[], "[6,2,3]"),
    },
    Case {
        id: "S11 equality is structural whether shared or rebuilt",
        source: r#"let a = { x = [1, 2] }; let b = a; let c = { x = [1, 2] }; let d = { ...a; x = [1, 3] }; [a == b, a == c, d == a, a.x[1:] == c.x[1:]]"#,
        expect: Value(&[], "[true,true,false,true]"),
    },
    Case {
        id: "S12 concatenation shares both operands and keeps them",
        source: r#"let xs = [1, 2]; let ys = [3]; let zs = [...xs, ...ys, ...xs]; xs[0] = 0; ys[0] = 0; [xs, ys, zs]"#,
        expect: Value(&[], "[[0,2],[0],[1,2,3,1,2]]"),
    },
    Case {
        id: "S13 a projected copy keeps its lazy payloads and the source keeps its fields",
        source: r#"let wide = { a = 1; b = 1 / 0; c = 3 }; let narrow: { a: int; c: int } = wide; let again = { ...narrow; a = 2 }; [narrow.c, again.a, object.len(wide)]"#,
        expect: Value(&[], "[3,2,3]"),
    },
    Case {
        id: "S14 prelude edits share and preserve",
        source: r#"let xs = [1, 2, 3, 4]; let a = xs.insert(2, 9); let b = xs.remove_at(0); let c = xs.splice(1, 3, [7]); [xs, a, b, c]"#,
        expect: Value(&[], "[[1,2,3,4],[1,2,9,3,4],[2,3,4],[1,7,4]]"),
    },
];

/// L: payloads stay lazy through every sharing operation.
const LAZINESS: &[Case] = &[
    Case {
        id: "L1 spread then removal never touches the poison",
        source: r#"let a = { x = 1; poison = 1 / 0 }; let b = { ...a; y = 2 }; let c = object.remove(b, "x"); [object.len(c), match object.lookup(c, "y") { { value: any; } => 1; else => 0; }]"#,
        expect: Value(&[], "[2,1]"),
    },
    Case {
        id: "L2 array spread, slice and index around a poison",
        source: r#"let xs = [1 / 0, 2, 3]; let ys = [...xs, 4]; [ys[1:][0], ys[3], ys.len(), ys[2:4]]"#,
        expect: Value(&[], "[2,4,4,[3,4]]"),
    },
    Case {
        id: "L3 overwriting a poisoned element never forces it",
        source: r#"let xs = [1 / 0, 2]; let ys = xs; ys[0] = 5; [ys[0], xs[1]]"#,
        expect: Value(&[], "[5,2]"),
    },
    Case {
        id: "L3b but the old snapshot still carries it",
        source: r#"let xs = [1 / 0, 2]; let ys = xs; ys[0] = 5; [ys[0], xs[0]]"#,
        expect: EvalError(&[], "zero"),
    },
    Case {
        id: "L4 nested update beside a poison",
        source: r#"let a = { i = { z = 1; p = 1 / 0 }; q = 1 / 0 }; let b = a; b.i.z = 2; [a.i.z, b.i.z]"#,
        expect: Value(&[], "[1,2]"),
    },
    Case {
        id: "L5 keys and len of a shared record with poisoned payloads",
        source: r#"let a = { z = 1 / 0; a = 1 / 0 }; let b = object.with(a, "m", 1 / 0); [object.keys(a), object.keys(b), object.len(b)]"#,
        expect: Value(&[], r#"[["a","z"],["a","m","z"],3]"#),
    },
    Case {
        id: "L6 the forced-node count proves the copies demanded nothing extra",
        source: r#"let a = { x = 1; poison = 1 / 0 }; let b = { ...a; y = 2 }; let c = { ...b; z = 3 }; let d = object.remove(c, "poison"); match object.lookup(d, "x") { { value: any; } => 1; else => 0; }"#,
        expect: Value(&[], "1"),
    },
];

/// D: depth and size.
const SCALE: &[Case] = &[
    Case {
        id: "D1 two thousand appends then slices and reads",
        source: r#"let build = fn(n: int) -> [int] { if n == 0 { return []; }; [...build(n - 1), n - 1] }; let b = build(2000); [b.len(), b[1999], b[1000:1004], b[:0].len()]"#,
        expect: Value(&[], "[2000,1999,[1000,1001,1002,1003],0]"),
    },
    Case {
        id: "D2 two thousand dynamic keys then lookup",
        source: r#"let make = fn(n: int) -> { ... } { let r: { ... } = {}; for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] { for j in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] { for k in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] { for l in [1, 2] { r = object.with(r, "k" + (((i * 10 + j) * 10 + k) * 2 + l).to_str(), n); }; }; }; }; r }; let r = make(7); [object.len(r), match object.lookup(r, "k2222") { { value: any; } => 1; else => 0; }]"#,
        expect: Value(&[], "[2000,1]"),
    },
];

#[test]
fn snapshots_stay_independent() {
    run_table(SNAPSHOTS);
}

#[test]
fn payloads_stay_lazy() {
    run_table(LAZINESS);
}

#[test]
fn depth_and_size() {
    run_table(SCALE);
}

#[test]
fn forced_nodes_do_not_depend_on_record_width() {
    // Spreads, removal and lookup demand structure, never payloads: the number of forced
    // nodes for the same chain must be identical whether the record has 2 fields or 200.
    let count = |width: usize| {
        let fields = (0..width)
            .map(|i| format!("f{i} = {i};"))
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!(
            "let a = {{ {fields} poison = 1 / 0 }}; let b = {{ ...a; y = 2 }}; let c = {{ ...b; z = 3 }}; let d = object.remove(c, \"poison\"); match object.lookup(d, \"f1\") {{ {{ value: any; }} => 1; else => 0; }}"
        );
        let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
        let mut evaluation = program.evaluator();
        let value = evaluation
            .select(&[])
            .unwrap_or_else(|e| panic!("{}", e.render(&source)));
        assert_eq!(value.to_string(), "1");
        evaluation.forced_nodes()
    };
    let (narrow, wide) = (count(2), count(200));
    assert_eq!(
        narrow, wide,
        "2 fields forced {narrow} nodes, 200 fields forced {wide}"
    );
}

fn build_time(entries: usize) -> f64 {
    let items = (0..entries)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        "let r: {{ ... }} = {{}}; let acc: [int] = []; for n in [{items}] {{ r = object.with(r, \"k\" + n.to_str(), n); acc = [...acc, n]; }}; [object.len(r), acc.len(), acc[{last}]]",
        last = entries - 1
    );
    let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
    let start = Instant::now();
    let value = program
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}", e.render(&source)));
    let elapsed = start.elapsed().as_secs_f64();
    assert_eq!(
        value.to_string(),
        format!("[{entries},{entries},{}]", entries - 1)
    );
    elapsed
}

#[test]
fn construction_scales_better_than_quadratically() {
    // Ten times the entries: linear or n log n growth stays near 10x, quadratic reaches 100x.
    let small = build_time(300).max(1e-4);
    let large = build_time(3000);
    let ratio = large / small;
    println!("sharing scale: 300 entries {small:.4}s, 3000 entries {large:.4}s, ratio {ratio:.1}");
    assert!(
        ratio < 40.0,
        "300 entries {small:.4}s, 3000 entries {large:.4}s, ratio {ratio:.1}"
    );
}
