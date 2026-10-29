use lonk::artifact::MemoryArtifactStore;
use lonk::{CachePolicy, Compiler, CompilerPolicy, MemoryStore, Program, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"{ answer = 6 * 7; payload = ["hello", "world"]; unused = 1 / 0 }"#;
    let program = Program::prepare(source)?;
    assert_eq!(program.evaluate(&["answer"])?, Value::Int(42));

    // The host obtains the native identity. Without it, keep caching in memory.
    let policy = CompilerPolicy::default();
    let mut compiler = match buildid::build_id() {
        Some(id) => Compiler::with_store(id, policy, Box::new(MemoryArtifactStore::default()))?,
        None => Compiler::new(policy),
    };
    println!("{}", compiler.evaluate(source, &["answer"])?);

    // MemoryStore demonstrates spill/reload; it does not save process memory.
    // A real host implements CacheStore using its own storage facilities.
    let mut session =
        program.evaluator_with_cache(Box::new(MemoryStore::new()), CachePolicy::new(0));
    let first = session.select(&["payload"])?;
    assert_eq!(session.select(&["payload"])?, first);
    println!(
        "reloaded records: {}",
        session.cache_stats().records_reloaded
    );
    session.close()?;
    Ok(())
}
