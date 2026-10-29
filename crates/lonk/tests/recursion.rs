use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn recursive_calls_use_the_definition_and_captured_snapshots() {
    assert_eq!(
        value("let fact=fn(n:int)->int {if n==0 {1} else {n*fact(n-1)}};fact(6)"),
        "720"
    );
    assert_eq!(
        value(
            "let f=fn(n:int)->int {if n==0 {1} else {n*f(n-1)}};let saved=f;f=fn(n:int)->int { n+100 };{saved=saved(5);new=f(5)}"
        ),
        r#"{"new":105,"saved":120}"#
    );
    assert_eq!(
        value(
            "let make=fn(base:int) {let count=fn(n:int)->int {if n==0 {base} else {1+count(n-1)}};count};let a=make(10);let b=make(20);{a=a(3);b=b(4)}"
        ),
        r#"{"a":13,"b":24}"#
    );
    assert_eq!(
        value("let f=fn(n:int)->int { n+1 };let f=fn(n:int)->int { f(n)*2 };f(3)"),
        "8"
    );
    assert_eq!(
        value("let f:fn(int)->int=fn(n) {if n==0 {0} else {1+f(n-1)}};f(5)"),
        "5"
    );
}

#[test]
fn recursive_generics_rows_and_borrowed_outputs() {
    assert_eq!(
        value(
            "let repeat=fn<T>(n:int,v:T)->[T] {if n==0 {[]} else {[v,...repeat(n-1,v)]}};{a=repeat(3,7);b=repeat(2,\"x\")}"
        ),
        r#"{"a":[7,7,7],"b":["x","x"]}"#
    );
    assert_eq!(
        value(
            "let down=fn<R>(v:&{x:int;...R}) {if v.x>0 {v.x-=1;down(&v);};};let v={x=5;label=\"kept\"};down(&v);v"
        ),
        r#"{"label":"kept","x":0}"#
    );
    assert_eq!(
        value(
            "let repeat=fn<T>(n:int,v:T)->T {if n==0 {v} else {repeat<T>(n-1,v)}};let a={x=7;bad=1/0};repeat(5,a).x"
        ),
        "7"
    );
    assert_eq!(
        value("let f=fn<T>(n:int,v:T)->int {if n==0 {1} else {f<bool>(n-1,true)}};f(4,\"s\")"),
        "1"
    );
}

#[test]
fn unselected_recursion_stays_lazy_and_failures_are_diagnostics() {
    let source = "let forever=fn(n:int)->int { forever(n) };{good=42;bad=forever(0)}";
    let program = Program::prepare(source).unwrap();
    assert_eq!(program.evaluate(&["good"]).unwrap().to_string(), "42");
    let error = program.evaluate(&["bad"]).unwrap_err();
    assert!(error.message.contains("nesting limit"), "{error}");
    for (source, needle) in [
        ("let f=fn(n:int) f(n);f(0)", "result type annotation"),
        ("let f=fn(n:int)->int { f(true) };f(0)", "expected int"),
        (
            "let f=fn(n:int)->int {if n==0 {\"x\"} else {f(n-1)}};f(0)",
            "expected int",
        ),
        (
            "let f=fn(n:int)->int { g(n) };let g=fn(n:int)->int { f(n) };g(0)",
            "unknown name",
        ),
        (
            "let f=fn(n:int)->int {f=fn(x:int)->int { x };n};f(0)",
            "captured",
        ),
        ("let f=fn(n:int)->int { f(n) };42", "unused computation"),
    ] {
        let error = Program::prepare(source)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.message.contains(needle), "{source}: {error}");
    }
}

#[test]
fn recursion_preserves_canonical_source_and_comments() {
    let source = "let f=fn(n:int)->int {/* stop */if n==0 {1} else {/* recur */n*f(n-1)}};f(4)";
    let formatted = Syntax::parse(source).unwrap().format();
    assert_eq!(formatted, Syntax::parse(&formatted).unwrap().format());
    assert_eq!(value(source), value(&formatted));
}

#[test]
fn large_demands_use_the_heap_work_stack_and_memoize_failures() {
    assert_eq!(
        value("let count=fn(n:int)->int {if n==0 {0} else {1+count(n-1)}};count(10000)"),
        "10000"
    );
    assert_eq!(
        value(
            "let build=fn(n:int)->[int] {if n==0 {[]} else {[n,...build(n-1)]}};build(2000)[1999]"
        ),
        "1"
    );
    assert_eq!(
        value(
            "let build=fn(n:int)->{value:int;next:any} { ({value=n;next=build(n+1)}) };let read=fn(v:any,n:int)->int {match v {{value:int;next:any;...}=>if n==0 {v.value} else {read(v.next,n-1)};else=>0}};read(build(0),2000)"
        ),
        "2000"
    );
    let source = "let spin=fn(n:int)->int { spin(n) };{bad=spin(0);good=42}";
    let program = Program::prepare(source).unwrap();
    let mut evaluation = program.evaluator();
    let error = evaluation.select(&["bad"]).unwrap_err();
    let before = evaluation.forced_nodes();
    assert_eq!(evaluation.select(&["bad"]).unwrap_err(), error);
    assert_eq!(evaluation.forced_nodes(), before);
    assert_eq!(evaluation.select(&["good"]).unwrap().to_string(), "42");
    let source = "let grow=fn<T>(x:T)->int { grow<[T]>([x]) };grow(0)";
    let error = Program::prepare(source).unwrap().evaluate(&[]).unwrap_err();
    assert!(error.message.contains("type nesting limit"), "{error}");
}
