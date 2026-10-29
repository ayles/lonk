use lonk::{Program, Type, Value};

fn run(source: &str) -> Value {
    let program =
        Program::prepare(source).unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)));
    program
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
}

fn reject(source: &str, message: &str) {
    let error = Program::prepare(source).expect_err(source);
    assert!(
        error.message.contains(message),
        "expected {message:?}, got {}",
        error.render(source)
    );
}

#[test]
fn arithmetic_strings_and_bounds() {
    assert_eq!(run("1 + 2 * 3"), Value::Int(7));
    assert_eq!(run("-9223372036854775808"), Value::Int(i64::MIN));
    assert_eq!(run("-7 / 3"), Value::Int(-2));
    assert_eq!(run("-7 % 3"), Value::Int(-1));
    assert_eq!(
        run(r#""hello " + "世界 ${HOME}""#),
        Value::Str("hello 世界 ${HOME}".into())
    );
    reject("9223372036854775808", "outside");
    let program = Program::prepare("9223372036854775807 + 1").unwrap();
    assert!(
        program
            .evaluate(&[])
            .unwrap_err()
            .message
            .contains("overflow")
    );
}

#[test]
fn guards_add_nested_facts_without_weakening_guarantees() {
    reject("let p: any = 1; p.x = 2; p", "assignment path");
    assert_eq!(
        run(r#"
        let p: {...} = {inner = {x = 7}};
        match p { {inner: {x: int; ...}; ...} => {
            p.inner.x += 1;
            match p.inner { {x?: any; ...} => p.inner.x + 1; else => 0; }
        }; else => 0; }
    "#),
        Value::Int(9)
    );
    assert_eq!(
        run("let x: any = 1; match x { int => {x = x + 1; x + 1}; else => 0; }"),
        Value::Int(3)
    );
    assert_eq!(
        run("let p: {...} = {x = 1}; match p { {x: int} => {p.y = 2; p.x + p.y}; else => 0; }"),
        Value::Int(3)
    );
    reject(
        "let x: any = 1; match x { int => {x = true; x}; else => x; }",
        "expected int",
    );
}

#[test]
fn outer_write_survives_a_later_shadow_in_the_block() {
    assert_eq!(run("let x = 1; let result = if true {x += 1; let x = x + 10; x} else {0}; {result = result; outer = x}").to_string(), r#"{"outer":2,"result":12}"#);
}

#[test]
fn long_version_chains_are_iterative_and_selections_are_typed() {
    let mut source = String::from("let v = {x = 1};");
    for i in 0..1200 {
        source.push_str(&format!("v.x = {i};"));
    }
    source.push_str("v.x");
    assert_eq!(run(&source), Value::Int(1199));
    let program = Program::prepare("{xs = [1, 1 / 0]; n = 2}").unwrap();
    assert_eq!(program.type_at(&["xs", "1"]).unwrap(), &Type::Int);
    let mut evaluation = program.evaluator();
    assert!(evaluation.select(&["missing"]).is_err());
    assert_eq!(evaluation.forced_nodes(), 0);
    assert_eq!(evaluation.select(&["xs", "0"]).unwrap(), Value::Int(1));
    assert!(
        evaluation
            .select(&["xs", "1"])
            .unwrap_err()
            .message
            .contains("division")
    );
}

#[test]
fn nesting_limits_report_diagnostics_and_equality_requires_known_data() {
    let mut nested = String::from("let v: any = 0;");
    for _ in 0..80 {
        nested.push_str("v = {x = v};");
    }
    nested.push('v');
    let program = Program::prepare(&nested).unwrap();
    assert!(
        program
            .evaluate(&[])
            .unwrap_err()
            .message
            .contains("export nesting")
    );
    reject(
        &format!("{}1{}", "(".repeat(1000), ")".repeat(1000)),
        "nesting",
    );
    reject(
        &std::iter::repeat_n("1", 1000)
            .collect::<Vec<_>>()
            .join(" + "),
        "chain",
    );
    reject(
        "let p = {x = if true {1} else {fn() 1}}; p == p",
        "does not accept",
    );
    reject("let p: {...} = {x = fn() 1}; p == p", "does not accept");
    assert_eq!(
        run(r#""%%{literal} ${HOME}""#),
        Value::Str("%{literal} ${HOME}".into())
    );
}

#[test]
fn nested_copies_and_array_updates_are_independent() {
    assert_eq!(
        run(r#"
        let a = { nested = { count = 1 }; values = [2, 3] };
        let b = a;
        b.nested.count += 1;
        b.values[0] = 8;
        { original = a; changed = b }
    "#)
        .to_string(),
        r#"{"changed":{"nested":{"count":2},"values":[8,3]},"original":{"nested":{"count":1},"values":[2,3]}}"#
    );
}

#[test]
fn delayed_reads_and_closures_capture_versions() {
    assert_eq!(
        run(r#"
        let pkg: {version: str; ...} = { version = "1.0" };
        pkg.url = "download/" + pkg.version;
        let version = fn() pkg.version;
        pkg.version = "2.0";
        { old = version(); url = pkg.url; current = pkg.version }
    "#)
        .to_string(),
        r#"{"current":"2.0","old":"1.0","url":"download/1.0"}"#
    );
}

#[test]
fn shadowing_keeps_previous_capture_and_reassignment_keeps_contract() {
    assert_eq!(
        run(r#"let v = 1; let old = fn() v; let v = "next"; {old = old(); new = v}"#).to_string(),
        r#"{"new":"next","old":1}"#
    );
    reject(
        r#"let v = {x = 1}; v = {name = "no"}; v"#,
        "unexpected fields",
    );
    reject("let v = {}; v.x = 1; v", "closed binding");
}

#[test]
fn siblings_are_checked_eagerly_and_evaluated_lazily() {
    let program = Program::prepare("{ ok = 42; bad = 1 / 0 }").unwrap();
    let mut evaluation = program.evaluator();
    assert_eq!(evaluation.select(&["ok"]).unwrap(), Value::Int(42));
    let count = evaluation.forced_nodes();
    assert_eq!(evaluation.select(&["ok"]).unwrap(), Value::Int(42));
    assert_eq!(evaluation.forced_nodes(), count);
    let error = evaluation.select(&["bad"]).unwrap_err();
    let failed_count = evaluation.forced_nodes();
    assert_eq!(evaluation.select(&["bad"]).unwrap_err(), error);
    assert_eq!(evaluation.forced_nodes(), failed_count);
    reject(r#"{ ok = 42; bad = 1 + "wrong" }"#, "does not accept");
}

#[test]
fn typed_copies_calls_returns_and_nested_destinations_project() {
    assert_eq!(
        run(r#"
        let p = { x = 1; y = 1 / 0 };
        let take = fn(v: {x: int}) v;
        let result = fn() -> {x: int} { p };
        let q: {x: int} = p;
        let holder = { sub = {x = 0}; items = [{x = 0}] };
        holder.sub = p;
        holder.items[0] = p;
        {binding = q; call = take(p); result = result(); holder = holder}
    "#)
        .to_string(),
        r#"{"binding":{"x":1},"call":{"x":1},"holder":{"items":[{"x":1}],"sub":{"x":1}},"result":{"x":1}}"#
    );
    reject(
        "let take = fn(v: {x: int}) v; take({x = 1; extra = 2})",
        "unexpected fields",
    );
    reject(
        "let q: {x: int} = {x = 1; typo = 2}; q",
        "unexpected fields",
    );
}

#[test]
fn projection_does_not_reveal_removed_fields() {
    assert_eq!(
        run(r#"
        let p = {a = {x = 1; hidden = 9}; hidden = 10};
        let q: {a: {x: int}} = p;
        let boxed: any = q;
        {spread = {...q}; boxed = boxed}
    "#)
        .to_string(),
        r#"{"boxed":{"a":{"x":1}},"spread":{"a":{"x":1}}}"#
    );
}

#[test]
fn open_copies_keep_values_but_require_evidence_to_read_extras() {
    reject("let p: {x: int; ...} = {x = 1; y = 2}; p.y", "not known");
    assert_eq!(
        run(r#"
        let p: {x: int; ...} = {x = 1; y = "kept"};
        match p { {x: int; y: str; ...} => p.y; else => "missing"; }
    "#),
        Value::Str("kept".into())
    );
    assert_eq!(run("let p: {...} = {x = 1}; p").to_string(), r#"{"x":1}"#);
}

#[test]
fn optional_presence_and_dynamic_payload_checks() {
    assert_eq!(
        run(r#"
        let p: {x?: int} = {x = 8};
        match p { {x: any; ...} => p.x + 1; else => 0; }
    "#),
        Value::Int(9)
    );
    reject("let p: {x?: int} = {}; p.x", "optional");
    reject(r#"let p: {x?: int} = {}; p.x = "bad"; p"#, "expected int");
    assert_eq!(
        run(r#"let p: any = {x = "bad"}; match p { {x?: int} => 1; else => 2; }"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#"let p: any = {}; match p { {x?: int} => 1; else => 2; }"#),
        Value::Int(1)
    );
    reject("let p: any = 2; p + 1", "does not accept");
}

#[test]
fn conditional_writes_join_presence_and_choose_only_one_branch() {
    assert_eq!(
        run(r#"
        let p: {...} = {};
        if true { p.x = 8; } else { p.x = 1 / 0; };
        p.x
    "#),
        Value::Int(8)
    );
    assert_eq!(
        run(r#"
        let p: {...} = {};
        if false { p.x = 8; };
        match p { {x: any; ...} => p.x; else => 10; }
    "#),
        Value::Int(10)
    );
    reject("let p: {...} = {}; if true { p.x = 8; }; p.x", "optional");
}

#[test]
fn lexical_scopes_and_calls_have_separate_frames() {
    assert_eq!(
        run(r#"
        let make = fn(x: int) fn(y: int) x + y;
        let a = make(10);
        let b = make(20);
        [a(1), b(2), a(3)]
    "#),
        Value::Array(vec![Value::Int(11), Value::Int(22), Value::Int(13)])
    );
    assert_eq!(
        run(r#"let x = 1; let f = fn(x: int) { let x = x + 1; x }; {outer = x; inner = f(3)}"#)
            .to_string(),
        r#"{"inner":4,"outer":1}"#
    );
    reject(
        "let x = 1; let f = fn() { x = 2; x }; f()",
        "captured binding",
    );
}

#[test]
fn ignored_parameters_are_allowed_but_unused_results_and_locals_are_errors() {
    assert_eq!(
        run("let f = fn(value: int, index: int) value; f(7, 1 / 0)"),
        Value::Int(7)
    );
    reject("let dead = 1; 2", "unused computation");
    reject("let x = 1; x = 2; x", "overwritten");
    reject("let f = fn() 1; f(); 2", "unused expression result");
    reject("let f = fn() {}; f", "must return a value");
}

#[test]
fn type_aliases_are_checked_and_returned_helpers_are_live() {
    assert_eq!(
        run("let T = type {x: int}; let p: T = {x = 1}; p.x"),
        Value::Int(1)
    );
    reject("let T = type {x: int}; 1", "unused computation");
    let program = Program::prepare("{ value = 1; helper = fn(x: int) x + 1 }").unwrap();
    assert_eq!(program.evaluate(&["value"]).unwrap(), Value::Int(1));
    assert!(
        program
            .evaluate(&[])
            .unwrap_err()
            .message
            .contains("only data")
    );
}

#[test]
fn contextual_empty_arrays_and_function_parameters() {
    assert_eq!(run("let xs: [int] = []; xs"), Value::Array(vec![]));
    reject("let xs = []; xs", "annotate empty arrays");
    assert_eq!(
        run("let f: fn(int) -> int = fn(x) x + 1; f(2)"),
        Value::Int(3)
    );
    assert_eq!(
        Program::prepare("fn(x: int) x").unwrap().result_type(),
        &Type::Function {
            receiver: false,
            parameters: vec![Type::Int],
            result: Box::new(Type::Int)
        }
    );
}

#[test]
fn overridden_payloads_and_short_circuiting_stay_lazy() {
    assert_eq!(
        run("let p = {x = 1 / 0; y = 2}; {...p; x = 3}").to_string(),
        r#"{"x":3,"y":2}"#
    );
    assert_eq!(run("false && (1 / 0 == 0)"), Value::Bool(false));
    assert_eq!(run("true || (1 / 0 == 0)"), Value::Bool(true));
}

#[test]
fn unsupported_features_are_diagnostics_not_unchecked_execution() {
    reject("let f = fn<T>(v: T) v + 1; f(1)", "does not accept");
    reject("let p = 1; &p", "borrowing");
    reject("fn(v) v.x", "needs a type annotation");
    reject(r#""hello %{name}""#, "interpolation");
}
