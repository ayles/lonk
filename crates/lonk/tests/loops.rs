use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn finite_loops_update_versions_and_iterate_a_source_snapshot() {
    assert_eq!(
        value(
            "let xs=[1,2,3];let before=xs;let sum=0;for x in xs {sum+=x;xs=[...xs,x+10];} {before=before;after=xs;sum=sum}"
        ),
        r#"{"after":[1,2,3,11,12,13],"before":[1,2,3],"sum":6}"#
    );
    assert_eq!(value("let xs:[int]=[];let n=7;for x in xs {n+=x;} n"), "7");
    assert_eq!(
        value("let n=0;for x in [1,2] {for y in [3,4] {n+=x*y;}} n"),
        "21"
    );
    assert_eq!(value("let x=10;let n=0;for x in [1,2] {n+=x;} x+n"), "13");
}

#[test]
fn generic_mapping_borrowed_folds_and_closures_stay_lazy() {
    assert_eq!(
        value(
            "let map=fn<T,U>(xs:[T],f:fn(T)->U)->[U] {let out:[U]=[];for x in xs {out=[...out,f(x)];} out};map([1,2,3],fn(x) x*2)"
        ),
        "[2,4,6]"
    );
    assert_eq!(
        value(
            "let total=fn<R>(v:&{sum:int;...R},xs:[int]) {for x in xs {v.sum+=x;}};let v={sum=0;label=\"kept\"};total(&v,[2,3]);v"
        ),
        r#"{"label":"kept","sum":5}"#
    );
    assert_eq!(
        value("let out:[fn()->int]=[];for x in [1,2,3] {out=[...out,fn() x];} out[0]()+out[2]()"),
        "4"
    );
    assert_eq!(
        value("let out:[int]=[];for x in [1/0,7] {out=[...out,x];} out[1]"),
        "7"
    );
    assert_eq!(
        value("let count=0;for x in [1/0,2/0] {count+=1;} count"),
        "2"
    );
}

#[test]
fn iteration_facts_include_zero_iterations_and_all_back_edges() {
    assert_eq!(
        value("let v:{...}={};for x in [1,2] {v.x=x;} match v {{x:int;...}=>v.x;else=>0}"),
        "2"
    );
    assert_eq!(
        value("let v:{x?:int;...}={};for x in [1,2] {v.x=x;} match v {{x:any;...}=>v.x;else=>0}"),
        "2"
    );
    for (source, needle) in [
        (
            "let n=0;for x in [1] {n+=x;42} n",
            "unused loop body result",
        ),
        (
            "let n=0;for x in [1] {let bad=1+\"s\";n+=x;} n",
            "does not accept",
        ),
        (
            "let n=0;let unused=99;for x in [1] {n+=x;} n",
            "unused computation",
        ),
        ("let n=0;for x in [1] {n+=x;} 42", "unused computation"),
        ("let v:{...}={};for x in [1] {v.x=x;} v.x", "optional"),
        ("let n=0;for x in 1 {n+=x;} n", "checked array"),
        ("for x in [1] {} 42", "does not update"),
        (
            "let n=0;let f=fn() {for x in [1] {n+=x;} n};f()",
            "captured",
        ),
    ] {
        let error = Program::prepare(source)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.message.contains(needle), "{source}: {error}");
    }
}

#[test]
fn loops_have_one_comment_preserving_format() {
    let source = "let n=0;for /* binder */ x in /* values */ [1,2] {/* update */n+=x;} n";
    let syntax = Syntax::parse(source).unwrap();
    let formatted = syntax.format();
    let reparsed = Syntax::parse(&formatted).unwrap();
    assert_eq!(formatted, reparsed.format());
    assert_eq!(syntax.comments().len(), reparsed.comments().len());
    assert_eq!(value(source), value(&formatted));
}

#[test]
fn else_if_chains_share_the_canonical_nested_branch_form() {
    let source = "let classify=fn(n:int)->str { if n<0 {\"negative\"} else if n==0 {\"zero\"} else {\"positive\"} };[classify(-1),classify(0),classify(1)]";
    assert_eq!(value(source), r#"["negative","zero","positive"]"#);
    let a = "let n=1;if n==0 {0} else if n==1 {1} else {2}";
    let b = "let n=1;if n==0 {0} else {if n==1 {1} else {2}}";
    assert_eq!(
        Syntax::parse(a).unwrap().format(),
        Syntax::parse(b).unwrap().format()
    );
    assert_eq!(
        value("let n=0;for x in [1,2,3] {if x==1 {n+=10;} else if x==2 {n+=20;} else {n+=30;};} n"),
        "60"
    );
}

#[test]
fn state_used_by_another_loop_output_is_useful() {
    let source = "let seen=0; let total=0; for n in [1,2,3] { total += seen; seen += n; }; total";
    assert_eq!(
        lonk::Program::prepare(source)
            .unwrap()
            .evaluate(&[])
            .unwrap()
            .to_string(),
        "4"
    );
    let dead = "let seen=0; let total=0; for n in [1,2,3] { total += n; seen += 1; }; total";
    assert!(lonk::Program::prepare(dead).is_err());
}
