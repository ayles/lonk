use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn borrowed_outputs_and_ordinary_results_are_independent_lazy_demands() {
    assert_eq!(
        value("let f = fn(x: &int) -> int { x + 1 }; let n = 1; f(&n)"),
        "2"
    );
    assert_eq!(
        value("let f = fn(x: &int) -> int { x = 1 / 0; 42 }; let n = 1; f(&n)"),
        "42"
    );
    assert_eq!(
        value("let f = fn(x: &int) { x = 42; }; let n = 1 / 0; f(&n); n"),
        "42"
    );
    assert_eq!(
        value("let f = fn(x: &int) { x = 42; }; let a = [1 / 0]; f(&a[0]); a"),
        "[42]"
    );
    let program =
        Program::prepare("let f = fn(x: &int) -> int { 42 }; let a = [1]; f(&a[9])").unwrap();
    assert!(
        program
            .evaluate(&[])
            .unwrap_err()
            .message
            .contains("out of bounds")
    );
}

#[test]
fn active_borrows_exclude_overlapping_access_but_allow_disjoint_access() {
    assert_eq!(
        value("let f = fn(x: &int, y: int) { x += y; }; let p = {x=1;y=2}; f(&p.x,p.y); p.x"),
        "3"
    );
    assert_eq!(
        value(
            "let f = fn(x: &int, y: &int) { x += 1; y += 10; }; let a = [1,2]; f(&a[0],&a[1]); a"
        ),
        "[2,12]"
    );
    for source in [
        "let f = fn(x: &int, y: int) { x += y; }; let p = {x=1;y=2}; f(&p.x,p.x); p",
        "let f = fn(x: &{x:int}, y: int) { x.x += y; }; let p = {x=1}; f(&p,p.x); p",
        "let f = fn(x: &int, y: fn()->int) { x += y(); }; let n = 1; f(&n,fn()->int { n }); n",
        "let f = fn(x: &int, y: int) { x += y; }; let mutate = fn(x:&int)->int {x+=1;x}; let n=1; f(&n,mutate(&n)); n",
    ] {
        let error = Program::prepare(source).expect_err(source);
        assert!(error.message.contains("overlap"), "{error}");
    }
}

#[test]
fn borrow_index_is_a_snapshot_even_when_a_later_argument_changes_the_index() {
    assert_eq!(
        value(
            "let bump=fn(x:&int,extra:int){x+=extra;}; let next=fn(i:&int)->int{i=1;1}; let xs=[1,2]; let i=0; bump(&xs[i],next(&i)); {xs=xs;i=i}"
        ),
        r#"{"i":1,"xs":[2,2]}"#
    );
}

#[test]
fn callbacks_with_borrow_modes_and_returned_snapshot_closures_work() {
    assert_eq!(
        value("let apply=fn(f:fn(&int), x:&int){f(&x);}; let n=1; apply(fn(v:&int){v+=1;},&n); n"),
        "2"
    );
    assert_eq!(
        value(
            "let keep=fn(x:&int)->fn()->int { fn() x }; let n=1; let saved=keep(&n); n=2; {old=saved();now=n}"
        ),
        r#"{"now":2,"old":1}"#
    );
    assert_eq!(
        value("let apply=fn<T>(f:fn(&T),x:&T){f(&x);}; let n=1; apply(fn(v:&int){v+=2;},&n); n"),
        "3"
    );
}

#[test]
fn borrows_do_not_admit_weakened_generic_caller_contracts_or_stored_references() {
    for source in [
        "let set=fn(v:&int){v=2;}; let f=fn<T>(v:T)->T {match v {int=>{set(&v);};else=>{};};v}; f(1)",
        "let refs: [&int] = []; refs",
        "let T = type &int; T",
        "let f=fn(v: &&int){ }; f",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}

#[test]
fn canonical_borrow_syntax_preserves_behavior() {
    let source = "let bump=fn<R>(p:&{x:int,...R}){let rest:{...R}=p;p={...rest;x=p.x+1};};let p={x=1;y=2};bump(&p);p";
    let formatted = Syntax::parse(source).unwrap().format();
    assert_eq!(value(source), value(&formatted));
    assert_eq!(formatted, Syntax::parse(&formatted).unwrap().format());
}
