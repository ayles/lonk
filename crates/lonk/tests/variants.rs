use lonk::{Program, Syntax};

fn value(source: &str) -> String {
    Program::prepare(source)
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .evaluate(&[])
        .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
        .to_string()
}

#[test]
fn variants_compose_with_generic_payloads_and_nested_options() {
    assert_eq!(
        value(
            r#"
        let Option=type<T> ("none" | {some:T});
        let get=fn<T>(v:Option<T>,fallback:T)->T { match v {
            "none"=>fallback; {some:any;...}=>v.some;
        } };
        let wrap=fn<T>(v:T)->Option<T> { ({some=v}) };
        let nested:Option<Option<int>>={some="none"};
        {a=get({some=42},0); b=get("none",7); c=get(wrap("x"),"y");
         nested=match nested {"none"=>0; {some:any;...}=>match nested.some {"none"=>1; {some:any;...}=>2;};}}
    "#
        ),
        r#"{"a":42,"b":7,"c":"x","nested":1}"#
    );
}

#[test]
fn literal_checks_support_scalar_operations_and_do_not_leak() {
    assert_eq!(
        value(
            r#"
        let n:any=1; let s:any="a";
        {number=match n {1=>[10,20][n]+n;else=>0;};
         text=match s {"a"=>s+"!";else=>"?";};
         compare=match n {1=>n==1;else=>false;};
         negative= {let v: (-2 | -1)=-1;match v {-2=>0;-1=>1;}}}
    "#
        ),
        r#"{"compare":true,"negative":1,"number":21,"text":"a!"}"#
    );
    for source in [
        r#"let s="a";let narrow:("a"|"b")=s;narrow"#,
        r#"let s:any="a";match s {"a"=>s;else=>"b";};s"#,
        r#"let edit=fn(n:&int){n+=1;};let n:any=1;match n {1=>{edit(&n);n};else=>0;}"#,
        r#"let n: --9223372036854775808=1;n"#,
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
    assert_eq!(
        value(
            "let n: -9223372036854775808=-9223372036854775808;match n {-9223372036854775808=>42;}"
        ),
        "42"
    );
}

#[test]
fn matches_use_static_payload_evidence_and_runtime_tags() {
    assert_eq!(
        value(
            r#"
        let V=type ({a:{value:int;bad:int}}|{b:str});
        let v:V={a={value=7;bad=1/0}};
        match v {{a:{value:7;...}}=>v.a.value;{a:any;...}=>0;{b:any;...}=>1;}
    "#
        ),
        "7"
    );
    assert_eq!(
        value(
            r#"
        let v:{x?:int}={x=1/0};match v {{x:int}=>42;else=>0;}
    "#
        ),
        "42"
    );
    assert_eq!(
        value(
            r#"
        let V=type ("none"|{some:int});let v:any={some=42};
        match v {V=>{let copy:V=v;match copy {"none"=>0;{some:any;...}=>copy.some;}};else=>1;}
    "#
        ),
        "42"
    );
    assert_eq!(
        value(
            r#"
        let V=type ({a:int}|{b:int});let v:{a?:int;b?:int}={a=42};
        match v {V=>{let copy:V=v;match copy {{a:any;...}=>copy.a;{b:any;...}=>copy.b;}};else=>0;}
    "#
        ),
        "42"
    );
}

#[test]
fn variant_payload_updates_keep_exclusivity_at_branch_and_loop_merges() {
    assert_eq!(
        value(
            r#"
        let V=type ({a:int}|{b:int});let v:V={a=1};
        for i in [2,3] {match v {{a:any;...}=>{v.a+=i;};{b:any;...}=>{v.b+=i;};};};
        match v {{a:any;...}=>v.a;{b:any;...}=>v.b;}
    "#
        ),
        "6"
    );
    assert_eq!(
        value(
            r#"
        let V=type ({a:int}|{b:int});let p={v={a=1}};let p:{v:V}=p;
        match p.v {{a:any;...}=>{p.v.a+=1;};{b:any;...}=>{p.v.b+=1;};};
        match p.v {{a:any;...}=>p.v.a;{b:any;...}=>p.v.b;}
    "#
        ),
        "2"
    );
    assert_eq!(
        value(
            r#"
        let V=type ({a:int}|{b:int});let inc=fn(n:&int){n+=1;};let v:V={b=1};
        match v {{a:any;...}=>{inc(&v.a);v.a};{b:any;...}=>{let alias=&v.b;inc(&alias);alias};}
    "#
        ),
        "2"
    );
}

#[test]
fn variant_equality_and_projection_check_the_whole_case_set() {
    assert_eq!(
        value(
            r#"
        let V=type (1|"none"|{some:int});let a:V=1;let b:V="none";let c:V={some=1};
        {same=a==a;different=a==b;record=b==c}
    "#
        ),
        r#"{"different":false,"record":false,"same":true}"#
    );
    for source in [
        "let V=type ({a:int}|{b:int});let v:{a:int;...}={a=1};let w:V=v;w",
        "let V=type ({a:int}|{b:int});let v={a=1;b=2;extra=3};let w:V=v;w",
        "let V=type ({a:int}|{b:int});let v:{a:int;b?:int}={a=1};let w:V=v;w",
        "let V=type (str|int);let v:V=1;v",
    ] {
        assert!(Program::prepare(source).is_err(), "accepted {source}");
    }
    assert_eq!(
        value("let V=type ({a:int}|{b:int});let v={a=1;b=2};let v:{a:int}=v;let w:V=v;w"),
        r#"{"a":1}"#
    );
}

#[test]
fn variant_formatting_and_nested_type_limits_are_stable() {
    assert_eq!(
        value("let Box=type<T>{x:T};let v:Box<Box<int>>={x={x=2}};v.x.x>=1"),
        "true"
    );
    assert_eq!(value("let n=1;let int=2;n<int"), "true");
    let source = r#"let V=type ("none"|/* tag */{some:(-1|2)});let v:V={some=-1};match v {"none"=>0;{some:any;...}=>1;}"#;
    let syntax = Syntax::parse(source).unwrap();
    let canonical = syntax.format();
    let again = Syntax::parse(&canonical).unwrap();
    assert_eq!(canonical, again.format());
    assert_eq!(syntax.comments().len(), again.comments().len());
    assert_eq!(value(source), value(&canonical));
    let nested = format!("let T=type {}int{};1", "(".repeat(1000), ")".repeat(1000));
    assert!(Syntax::parse(&nested).is_err());
}

#[test]
fn overlapping_payload_constraints_keep_empty_and_absent_cases() {
    assert_eq!(
        value(
            r#"
        let V=type ({a:[int]}|{b:int});let v:V={a=[]};
        match v {{a:[str]}=>42;{a:[int]}=>1;{b:int}=>2;}
    "#
        ),
        "42"
    );
    assert_eq!(
        value(
            r#"
        let V=type ({a:{x?:int}}|{b:int});let v:V={a={}};
        match v {{a:{x?:str}}=>42;{a:{x?:int}}=>1;{b:int}=>2;}
    "#
        ),
        "42"
    );
}
