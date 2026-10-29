use lonk::{CachePolicy, EvaluationError, MemoryStore, Program, Value};

fn doubling(count: u32) -> String {
    let steps = (0..count)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("let xs = [42]; for i in [{steps}] {{ xs = [...xs, ...xs]; }}; xs[0]")
}

#[test]
fn a_compact_huge_array_can_be_selected_and_spilled_without_expanding_it() {
    let source = doubling(usize::BITS - 2);
    let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
    assert_eq!(program.evaluate(&[]).unwrap(), Value::Int(42));
    let mut evaluation =
        program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
    assert_eq!(evaluation.select(&[]).unwrap(), Value::Int(42));
    let forced = evaluation.forced_nodes();
    assert_eq!(evaluation.select(&[]).unwrap(), Value::Int(42));
    assert_eq!(evaluation.forced_nodes(), forced);
    assert_eq!(evaluation.cache_stats().resident_payload_bytes, 0);
    assert!(evaluation.cache_stats().records_written < 10_000);
    evaluation.close().unwrap();
}

#[test]
fn logical_array_length_overflow_is_a_memoized_diagnostic() {
    let source = doubling(usize::BITS);
    let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
    let mut plain = program.evaluator();
    let error = plain.select(&[]).unwrap_err();
    assert_eq!(error.message, "array length exceeds the supported size");
    let forced = plain.forced_nodes();
    assert_eq!(plain.select(&[]).unwrap_err(), error);
    assert_eq!(plain.forced_nodes(), forced);
    let mut cached =
        program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
    for _ in 0..2 {
        match cached.select(&[]) {
            Err(EvaluationError::Language(actual)) => assert_eq!(actual, error),
            other => panic!("expected checked length failure, got {other:?}"),
        }
    }
}
