use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn spreading_arrays_preserves_lazy_elements_and_snapshot_versions() {
    assert_eq!(
        value("let a=[1,2];let old=a;a=[...a,3];{old=old;new=a;combined=[0,...a,...old,4]}"),
        r#"{"combined":[0,1,2,3,1,2,4],"new":[1,2,3],"old":[1,2]}"#
    );
    assert_eq!(value("let a=[1/0,7];let b=[0,...a];b[2]"), "7");
    assert_eq!(
        value("let a=[1];let b=[...a];a[0]=2;{a=a;b=b}"),
        r#"{"a":[2],"b":[1]}"#
    );
    assert_eq!(value("let a:[int]=[...[],...[]];a"), "[]");
}

#[test]
fn generic_borrowed_append_preserves_element_types_and_rows() {
    assert_eq!(
        value(
            "let append=fn<T>(xs:&[T],x:T){xs=[...xs,x];};let xs=[1];let before=xs;append(&xs,2);{before=before;after=xs}"
        ),
        r#"{"after":[1,2],"before":[1]}"#
    );
    assert_eq!(
        value(
            "let add=fn<R>(v:&{flags:[str];...R},flag:str){v.flags=[...v.flags,flag];};let p={name=\"web\";flags=[\"-Wall\"]};add(&p,\"-O2\");p"
        ),
        r#"{"flags":["-Wall","-O2"],"name":"web"}"#
    );
    for source in [
        "[...1]",
        "let x:any=[1];[...x]",
        "let x:[int]=[...[\"s\"]];x",
        "let append=fn<T>(xs:&[T],x:T){xs=[...xs,x];};let xs=[1];append(&xs,\"s\");xs",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
}

#[test]
fn formatting_preserves_spreads_and_their_comments() {
    let source = "let xs=[1,2];[0,/* before spread */.../* after spread */xs,3]";
    let parsed = Syntax::parse(source).unwrap();
    let formatted = parsed.format();
    assert_eq!(value(source), value(&formatted));
    assert_eq!(formatted, Syntax::parse(&formatted).unwrap().format());
    assert_eq!(
        parsed.comments().len(),
        Syntax::parse(&formatted).unwrap().comments().len()
    );
}
