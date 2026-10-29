//! Statically checked, lazy snapshot evaluation. The library has no host I/O.

pub mod artifact;
pub mod cache;
mod check;
mod codec;
mod compiler;
mod eval;
mod format;
mod modules;
mod syntax;
mod types;

pub use cache::{
    CacheError, CacheKey, CachePolicy, CacheStats, CacheStore, EvaluationError, MemoryStore,
};
pub use compiler::{CompileError, Compiler, CompilerPolicy, CompilerStats};
pub use eval::{CachedEvaluation, Evaluation, Value};
pub use format::format;
pub use modules::{ModuleDiagnostic, ModuleSource};
pub use syntax::{Comment, Diagnostic, Span, Syntax};
pub use types::{Field, LiteralType, Type};

/// A whole source unit that has passed parsing, name resolution and type checking.
#[derive(Clone, Debug)]
pub struct Program {
    source: std::sync::Arc<str>,
    graph: std::sync::Arc<check::Graph>,
    sources: std::sync::Arc<Vec<(String, std::sync::Arc<str>)>>,
}

impl Program {
    pub fn prepare(source: &str) -> Result<Self, Diagnostic> {
        let ast = syntax::parse(source)?;
        let graph = check::compile(&ast.body)?;
        Ok(Self::from_graph(source, graph))
    }

    fn from_graph(source: &str, graph: check::Graph) -> Self {
        let source: std::sync::Arc<str> = source.into();
        Self {
            source: source.clone(),
            graph: graph.into(),
            sources: vec![
                (
                    "<stdlib>".into(),
                    include_str!("../std/collections.lonk").into(),
                ),
                ("<source>".into(), source),
            ]
            .into(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn result_type(&self) -> &Type {
        &self.graph.nodes[self.graph.root].ty
    }

    /// Retained evaluator nodes after reachability compaction. A diagnostic
    /// metric, not a stable language observation or a byte-size measurement.
    pub fn node_count(&self) -> usize {
        self.graph.nodes.len()
    }

    /// Inspect a selection without evaluating it. Array path segments are
    /// decimal indices; optional/unknown fields need source-level evidence.
    pub fn type_at(&self, path: &[&str]) -> Result<&Type, Diagnostic> {
        self.result_type()
            .select(path, self.graph.nodes[self.graph.root].span)
    }

    /// An evaluation session memoizes both successful results and failures.
    pub fn evaluator(&self) -> Evaluation<'_> {
        Evaluation::new(&self.graph)
    }

    /// Spill ready data through an embedder-owned, fresh session store. This
    /// does not make computations or cache keys reusable across process runs.
    pub fn evaluator_with_cache(
        &self,
        store: Box<dyn CacheStore>,
        policy: CachePolicy,
    ) -> CachedEvaluation<'_> {
        CachedEvaluation::new(&self.graph, store, policy)
    }

    pub fn evaluate(&self, path: &[&str]) -> Result<Value, Diagnostic> {
        self.evaluator().select(path)
    }
}
