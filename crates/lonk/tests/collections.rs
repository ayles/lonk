use lonk::{Program, Syntax};

fn run(source: &str, expected: &str) {
    let p = Program::prepare(source).unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)));
    assert_eq!(
        p.evaluate(&[])
            .unwrap_or_else(|e| panic!("{}\n{source}", e.render(source)))
            .to_string(),
        expected
    );
    let canonical = Syntax::parse(source).unwrap().format();
    assert_eq!(canonical, Syntax::parse(&canonical).unwrap().format());
    assert_eq!(
        Program::prepare(&canonical)
            .unwrap()
            .evaluate(&[])
            .unwrap()
            .to_string(),
        expected
    );
}

#[test]
fn slices_and_array_algorithms() {
    run(
        "let xs = [1, 2, 3]; {len=xs.len(); middle=xs[1:2]; prefix=xs[:2]; suffix=xs[2:]; copy=xs[:]; appended=xs.append(4); joined=xs.concat([4,5]); sliced=xs.slice(1); replaced=xs.splice(1,2,[7,8]); reversed=xs.reverse()}",
        r#"{"appended":[1,2,3,4],"copy":[1,2,3],"joined":[1,2,3,4,5],"len":3,"middle":[2],"prefix":[1,2],"replaced":[1,7,8,3],"reversed":[3,2,1],"sliced":[2,3],"suffix":[3]}"#,
    );
    run(
        "let xs = [1,2,3]; {map=xs.map(fn(x:int)->int { x*2 }); filter=xs.filter(fn(x:int)->bool { x>1 }); fold=xs.fold(0,fn(sum:int,x:int)->int { sum+x }); first=xs.first(); last=xs.last(); found=xs.find(fn(x:int)->bool { x==2 }); none=xs.find(fn(x:int)->bool { x==7 }); any=xs.any(fn(x:int)->bool { x==2 }); all=xs.all(fn(x:int)->bool { x>0 })}",
        r#"{"all":true,"any":true,"filter":[2,3],"first":{"value":1},"fold":6,"found":{"value":2},"last":{"value":3},"map":[2,4,6],"none":{}}"#,
    );
    run(
        "let xs:[int]=[]; {first=xs.first(); all=xs.all(fn(x:int)->bool { x>0 }); any=xs.any(fn(x:int)->bool { x>0 }); slice=xs[:]; len=array.len(xs)}",
        r#"{"all":true,"any":false,"first":{},"len":0,"slice":[]}"#,
    );
}

#[test]
fn lazy_shapes_short_circuit_and_snapshots() {
    run("let xs=[1/0, 42, 1/0]; [xs.len(), xs[1:2][0]]", "[3,42]");
    run("let xs=[1,1/0]; xs.any(fn(x:int)->bool { x==1 })", "true");
    run(
        "let xs=[1,2]; let ys=xs[:]; xs[0]=3; [xs,ys]",
        "[[3,2],[1,2]]",
    );
    run("let xs=[1/0,42]; xs.map(fn(x:int)->int { x+1 })[1]", "43");
    run(
        "let second=fn<T>(xs:[T])->T { xs[1] }; second([1,42])",
        "42",
    );
    run("let xs=[1,2]; let length=xs.len; length(xs)", "2");
}

#[test]
fn objects_and_strings() {
    run(
        "let v={z=1/0; a=2}; {len=object.len(v); keys=object.keys(v); removed=object.remove(v,\"z\"); lookup=object.lookup(v,\"a\"); missing=object.lookup(v,\"none\"); with=object.with(object.remove(v,\"z\"),\"other\",3)}",
        r#"{"keys":["a","z"],"len":2,"lookup":{"value":2},"missing":{},"removed":{"a":2},"with":{"a":2,"other":3}}"#,
    );
    run(
        "{len=\"é😀a\".len(); slice=\"é😀a\"[1:2]; parts=\"a,b,c\".split(\",\"); joined=string.join([\"a\",\"b\"],\"/\"); trimmed=\"  hi\n\".trim(); replaced=\"abab\".replace(\"a\",\"X\"); has=\"abc\".contains(\"b\"); start=\"abc\".starts_with(\"ab\"); end=\"abc\".ends_with(\"bc\"); n=42.to_str(); b=true.to_str()}",
        r#"{"b":"true","end":true,"has":true,"joined":"a/b","len":3,"n":"42","parts":["a","b","c"],"replaced":"XbXb","slice":"😀","start":true,"trimmed":"hi"}"#,
    );
}

#[test]
fn invalid_bounds_and_contracts() {
    for source in [
        "[1,2][-1:]",
        "[1,2][2:1]",
        "[1,2][:3]",
        "[1,2].splice(2,1,[])",
    ] {
        let p = Program::prepare(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert!(p.evaluate(&[]).is_err(), "{source}");
    }
    for source in [
        "[1][false:]",
        "[1].append(false)",
        "let x={a=1}; x=object.remove(x,\"a\"); x",
        "object.remove({a=1},\"a\").a",
    ] {
        assert!(Program::prepare(source).is_err(), "{source}");
    }
}

#[test]
fn edits_and_user_defined_borrowed_helpers() {
    run(
        "let xs=[1,2]; let old=xs; xs.push(3); xs.extend([4]); {old=old; now=xs; inserted=xs.insert(2,7); removed=xs.remove_at(1); take=xs.take(2); drop=xs.drop(2); contains=xs.contains(3,fn(a:int,b:int)->bool { a==b }); index=xs.index_of(3,fn(a:int,b:int)->bool { a==b }); missing=xs.index_of(9,fn(a:int,b:int)->bool { a==b })}",
        r#"{"contains":true,"drop":[3,4],"index":{"index":2},"inserted":[1,2,7,3,4],"missing":{},"now":[1,2,3,4],"old":[1,2],"removed":[1,3,4],"take":[1,2]}"#,
    );
    run(
        "let clear=fn<T>(xs:&[T]) {xs=[];}; let xs=[1,2]; clear(&xs); xs",
        "[]",
    );
}

#[test]
fn builtin_members_are_independent_of_lexical_names_and_namespace_copies() {
    run(
        "let array = 99; let string = 98; let integer = 97; let boolean = 96; {names = [array, string, integer, boolean]; length = [1 / 0, 2].len(); text = \"hello\".len(); number = 42.to_str(); flag = true.to_str()}",
        r#"{"flag":"true","length":2,"names":[99,98,97,96],"number":"42","text":5}"#,
    );
    run(
        "let array = {len = fn(xs: [int]) 99}; let f = fn(array: int) {let length = [1, 2].len; length([1, 2]) + array}; [array.len([1]), [1].len(), f(40)]",
        "[99,1,42]",
    );
    run(
        "let array = {...array; push = fn(xs: &[int], n: int) {xs = [n];}}; let a = [1]; let b = a; a.push(2); array.push(&b, 2); [a, b]",
        "[[1,2],[2]]",
    );
    for source in [
        "let array = {...array; second = fn<T>(xs: [T]) -> T {xs[1]}}; {namespace = array; result = [1, 2].second()}",
        "let array = {...array; clear = fn<T>(xs: &[T]) {xs = [];}}; let xs = [1]; xs.clear(); {namespace = array; value = xs}",
    ] {
        assert!(Program::prepare(source).is_err(), "{source}");
    }
}
