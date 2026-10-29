use lonk::Program;

fn check(source: String, expected: &str) {
    let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
    let mut evaluation = program.evaluator();
    assert_eq!(evaluation.select(&[]).unwrap().to_string(), expected);
    let forced = evaluation.forced_nodes();
    assert_eq!(evaluation.select(&[]).unwrap().to_string(), expected);
    assert_eq!(evaluation.forced_nodes(), forced);
}

fn numbers(n: usize) -> String {
    (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

#[test]
fn retained_registry_versions_survive_later_inserts_and_deletions() {
    check(
        format!(
            r#"
        let registry: {{...}} = {{}};
        let versions: [{{...}}] = [];
        for n in [{}] {{
            registry = object.with(registry, n.to_str(), {{n=n; poison=1/0}});
            versions.push(registry);
        }};
        let first = versions[0];
        let last = versions[999];
        let trimmed = object.remove(registry, "999");
        {{
            first_count=object.len(first);
            first_has_last="999" in first;
            middle_count=object.len(versions[499]);
            last_count=object.len(last);
            now_count=object.len(registry);
            trimmed_count=object.len(trimmed);
            trimmed_has_last="999" in trimmed;
            original_has_last="999" in registry;
        }}
    "#,
            numbers(1000)
        ),
        r#"{"first_count":1,"first_has_last":false,"last_count":1000,"middle_count":500,"now_count":1000,"original_has_last":true,"trimmed_count":999,"trimmed_has_last":false}"#,
    );
}

#[test]
fn array_versions_slices_and_concatenations_keep_independent_paths() {
    check(
        format!(
            r#"
        let xs: [int] = [];
        let history: [[int]] = [];
        for n in [{}] {{ xs.push(n); history.push(xs); }};
        let original=xs;
        let middle=xs[400:600];
        xs[500]=1/0;
        let tail=xs[501:];
        middle[100]=9999;
        let combined=[...xs[:500],...middle, ...tail];
        {{
            first=history[0];
            earlier_len=history[499].len();
            last=history[999][500];
            original=original[500];
            middle=middle[100];
            combined_len=combined.len();
            combined_edit=combined[600];
            tail_first=tail[0];
            now_len=xs.len();
        }}
    "#,
            numbers(1000)
        ),
        r#"{"combined_edit":9999,"combined_len":1199,"earlier_len":500,"first":[0],"last":500,"middle":9999,"now_len":1000,"original":500,"tail_first":501}"#,
    );
}
