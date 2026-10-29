use lonk::{CachePolicy, EvaluationError, MemoryStore, Program, Value};

fn check_failure(source: &str, message: &str) {
    let program = Program::prepare(source).unwrap_or_else(|e| panic!("{}", e.render(source)));
    let mut plain = program.evaluator();
    let expected = plain.select(&[]).unwrap_err();
    assert!(expected.message.contains(message), "{source}: {expected:?}");
    let forced = plain.forced_nodes();
    assert_eq!(plain.select(&[]).unwrap_err(), expected);
    assert_eq!(plain.forced_nodes(), forced);
    let mut cached =
        program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
    for _ in 0..2 {
        match cached.select(&[]) {
            Err(EvaluationError::Language(actual)) => assert_eq!(actual, expected),
            other => panic!("expected language failure, got {other:?}"),
        }
    }
    cached.close().unwrap();
}

#[test]
fn reads_slices_and_updates_demand_the_root_before_index_expressions() {
    for tail in [
        "p[-1]",
        "p[5]",
        "p[9223372036854775807 + 1]",
        "p[9223372036854775807 + 1:]",
        "p[-1] = 42; p",
        "p[5] = 42; p",
        "p[9223372036854775807 + 1] = 42; p",
    ] {
        check_failure(
            &format!("let p = [1][(1 / 0):]; {tail}"),
            "division by zero",
        );
    }
}

#[test]
fn update_bounds_are_checked_when_the_corresponding_array_is_demanded() {
    for index in ["-1", "5"] {
        check_failure(
            &format!("let p = [1]; p[{index}] = 42; p"),
            "array update index out of bounds",
        );
        check_failure(
            &format!("let p = {{xs = [1][(1 / 0):]}}; p.xs[{index}] = 42; p.xs"),
            "division by zero",
        );
    }
}

#[test]
fn nested_update_indices_are_captured_left_to_right_before_child_descent() {
    check_failure(
        "let p = [[1]]; p[1 / 0][9223372036854775807 + 1] = 42; p",
        "division by zero",
    );
    check_failure(
        "let p = [[1][(1 / 0):]]; p[0][9223372036854775807 + 1] = 42; p",
        "integer overflow",
    );
}

#[test]
fn unselected_nested_updates_keep_the_child_and_bounds_lazy() {
    for index in ["-1", "5"] {
        let source =
            format!("let p = {{xs = [1][(1 / 0):]; keep = 42}}; p.xs[{index}] = 0; p.keep");
        let program = Program::prepare(&source).unwrap_or_else(|e| panic!("{}", e.render(&source)));
        assert_eq!(program.evaluate(&[]).unwrap(), Value::Int(42));
        let mut cached =
            program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
        assert_eq!(cached.select(&[]).unwrap(), Value::Int(42));
        let forced = cached.forced_nodes();
        assert_eq!(cached.select(&[]).unwrap(), Value::Int(42));
        assert_eq!(cached.forced_nodes(), forced);
        cached.close().unwrap();
    }
}
