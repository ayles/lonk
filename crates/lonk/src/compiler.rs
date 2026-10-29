//! Explicit preparation reuse. Inputs are immutable snapshots supplied by a host.
use crate::{
    CacheError, Diagnostic, EvaluationError, ModuleDiagnostic, ModuleSource, Program, Syntax,
    Value,
    artifact::{self, ArtifactError, ArtifactKey, ArtifactStore},
    check, codec,
    eval::{Computations, Evaluation, Scalar},
    syntax,
};
use std::{collections::BTreeMap, fmt, sync::Arc};
mod modules;
mod validation;

#[derive(Clone, Copy, Debug)]
pub struct CompilerPolicy {
    /// Budget for estimated retained syntax, graphs and outputs, excluding the
    /// fixed prelude seed and Programs/Values held by the caller. Not an RSS cap.
    pub memory_bytes: usize,
    /// Maximum encoded artifact. Larger results remain usable but are not saved.
    pub artifact_bytes: usize,
}
impl Default for CompilerPolicy {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            artifact_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompilerStats {
    pub parses: usize,
    pub checks: usize,
    pub prelude_checks: usize,
    pub module_checks: usize,
    pub module_hits: usize,
    pub syntax_hits: usize,
    pub program_hits: usize,
    pub value_hits: usize,
    /// Successful scalar demand reuse, including reuse across source edits.
    pub computation_hits: usize,
    pub computation_writes: usize,
    /// Evaluations started on misses; accumulated across selections.
    pub forced_nodes: usize,
    pub artifact_hits: usize,
    pub artifact_writes: usize,
    pub evictions: usize,
    pub retained_bytes: usize,
}

#[derive(Clone, Debug)]
pub enum CompileError {
    Language(ModuleDiagnostic),
    Infrastructure(ArtifactError),
}
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(e) => e.fmt(f),
            Self::Infrastructure(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for CompileError {}
impl From<ArtifactError> for CompileError {
    fn from(e: ArtifactError) -> Self {
        Self::Infrastructure(e)
    }
}

type Prepared = Result<Program, ModuleDiagnostic>;
type Output = Result<Value, ModuleDiagnostic>;
#[derive(Clone)]
enum Memo {
    Syntax(Result<Arc<Syntax>, Diagnostic>),
    Program(Prepared),
    Value(Output),
    Unit(Result<Arc<check::Graph>, Diagnostic>),
    Computation(Scalar),
}
struct Entry {
    memo: Memo,
    bytes: usize,
    touched: u64,
}
#[derive(Default)]
struct Memory {
    entries: BTreeMap<ArtifactKey, Entry>,
    clock: u64,
}
impl Memory {
    fn get(&mut self, key: ArtifactKey) -> Option<Memo> {
        self.clock = self.clock.wrapping_add(1);
        self.entries.get_mut(&key).map(|entry| {
            entry.touched = self.clock;
            entry.memo.clone()
        })
    }
    fn insert(
        &mut self,
        key: ArtifactKey,
        memo: Memo,
        bytes: usize,
        policy: CompilerPolicy,
        stats: &mut CompilerStats,
    ) {
        if bytes > policy.memory_bytes || policy.memory_bytes == 0 {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            stats.retained_bytes -= old.bytes;
        }
        while stats.retained_bytes > policy.memory_bytes - bytes {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.touched)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                stats.retained_bytes -= old.bytes;
            }
            stats.evictions += 1;
        }
        self.clock = self.clock.wrapping_add(1);
        self.entries.insert(
            key,
            Entry {
                memo,
                bytes,
                touched: self.clock,
            },
        );
        stats.retained_bytes += bytes;
    }
    fn parse(
        &mut self,
        source: &str,
        policy: CompilerPolicy,
        stats: &mut CompilerStats,
    ) -> Result<Arc<Syntax>, Diagnostic> {
        let key = artifact::key(0, &[source.as_bytes()]);
        if let Some(Memo::Syntax(ast)) = self.get(key) {
            stats.syntax_hits += 1;
            return ast;
        }
        stats.parses += 1;
        let ast = syntax::parse(source).map(Arc::new);
        self.insert(
            key,
            Memo::Syntax(ast.clone()),
            source.len().saturating_mul(64).saturating_add(1024),
            policy,
            stats,
        );
        ast
    }
}

/// Owned, bounded memoization; dropping it releases all retained preparations.
/// Every miss still checks all bodies. No global state, filesystem or clocks.
pub struct Compiler {
    policy: CompilerPolicy,
    stats: CompilerStats,
    memory: Memory,
    library: Option<Arc<check::Graph>>,
    store: Option<Box<dyn ArtifactStore>>,
    namespace: Vec<u8>,
}
impl Default for Compiler {
    fn default() -> Self {
        Self::new(CompilerPolicy::default())
    }
}
impl Compiler {
    pub fn new(policy: CompilerPolicy) -> Self {
        Self {
            policy,
            stats: CompilerStats::default(),
            memory: Memory::default(),
            library: None,
            store: None,
            namespace: Vec::new(),
        }
    }
    /// Enable persistent reuse in an opaque host-supplied compatibility namespace.
    ///
    /// The namespace must change whenever this Lonk implementation, its
    /// dependencies, or relevant host behavior/configuration changes. A statically
    /// linked host can use its native executable build ID (for example, from
    /// the `buildid` crate). A host using
    /// dynamic libraries must also account for their identities. Do not use a
    /// package version or a constant shared across different builds.
    ///
    /// If no identity is available, use `new` for memory-only caching. This
    /// method rejects empty namespaces; it does not discover the host's identity or
    /// access the filesystem. Artifacts still require a trusted producer.
    pub fn with_store(
        namespace: &[u8],
        policy: CompilerPolicy,
        store: Box<dyn ArtifactStore>,
    ) -> Result<Self, ArtifactError> {
        if namespace.is_empty() {
            return Err(ArtifactError(
                "persistent caching requires a compatibility namespace".into(),
            ));
        }
        Ok(Self {
            store: Some(store),
            namespace: namespace.to_vec(),
            ..Self::new(policy)
        })
    }
    pub fn stats(&self) -> &CompilerStats {
        &self.stats
    }
    /// Release retained state, including the prelude. Historical counters remain.
    pub fn clear_memory(&mut self) {
        self.memory = Memory::default();
        self.library = None;
        self.stats.retained_bytes = 0;
    }
    pub fn prepare(&mut self, source: &str) -> Result<Program, CompileError> {
        self.prepare_input(Input::Source(source))
    }
    pub fn prepare_modules(
        &mut self,
        entry: &str,
        modules: &BTreeMap<String, ModuleSource>,
    ) -> Result<Program, CompileError> {
        self.prepare_input(Input::Modules(entry, modules))
    }
    pub fn evaluate(&mut self, source: &str, path: &[&str]) -> Result<Value, CompileError> {
        self.evaluate_input(Input::Source(source), path)
    }
    pub fn evaluate_modules(
        &mut self,
        entry: &str,
        modules: &BTreeMap<String, ModuleSource>,
        path: &[&str],
    ) -> Result<Value, CompileError> {
        self.evaluate_input(Input::Modules(entry, modules), path)
    }

    fn load<T: serde::de::DeserializeOwned>(
        &mut self,
        key: ArtifactKey,
    ) -> Result<Option<T>, ArtifactError> {
        let Some(store) = &mut self.store else {
            return Ok(None);
        };
        let key = artifact::key(15, &[&self.namespace, &key.0]);
        let Some(bytes) = store.get(key, self.policy.artifact_bytes)? else {
            return Ok(None);
        };
        let value = codec::decode(key, &bytes, self.policy.artifact_bytes)?;
        self.stats.artifact_hits += 1;
        Ok(Some(value))
    }
    fn save<T: serde::Serialize>(
        &mut self,
        key: ArtifactKey,
        value: &T,
    ) -> Result<(), ArtifactError> {
        let Some(store) = &mut self.store else {
            return Ok(());
        };
        let key = artifact::key(15, &[&self.namespace, &key.0]);
        if let Some(bytes) = codec::encode(key, value, self.policy.artifact_bytes)? {
            store.put(key, &bytes)?;
            self.stats.artifact_writes += 1;
        }
        Ok(())
    }
    fn prepare_input(&mut self, input: Input<'_>) -> Result<Program, CompileError> {
        let key = input.key();
        if let Some(Memo::Program(result)) = self.memory.get(key) {
            self.stats.program_hits += 1;
            return result.map_err(CompileError::Language);
        }
        let sources = input.sources();
        let before = self.stats.module_checks;
        let checked = match input {
            Input::Modules(entry, modules) => self.module_program(entry, modules, sources.clone()),
            Input::Source(source) => {
                let modules = BTreeMap::from([("<source>".into(), ModuleSource::new(source))]);
                self.module_program("<source>", &modules, sources.clone())
            }
        };
        if self.stats.module_checks > before {
            self.stats.checks += 1;
        }
        let result = match checked {
            Ok(program) => Ok(program),
            Err(CompileError::Language(error)) => Err(error),
            Err(error) => return Err(error),
        };
        let bytes = match &result {
            Ok(p) => p
                .graph
                .nodes
                .len()
                .saturating_mul(std::mem::size_of::<check::Node>() * 2)
                .saturating_add(
                    sources
                        .iter()
                        .map(|(_, s)| s.len().saturating_mul(32))
                        .sum::<usize>(),
                ),
            Err(e) => e
                .source
                .len()
                .saturating_add(e.diagnostic.message.len())
                .saturating_add(512),
        };
        self.memory.insert(
            key,
            Memo::Program(result.clone()),
            bytes,
            self.policy,
            &mut self.stats,
        );
        result.map_err(CompileError::Language)
    }

    fn evaluate_input(&mut self, input: Input<'_>, path: &[&str]) -> Result<Value, CompileError> {
        let program_key = input.key();
        let mut parts = vec![program_key.0.as_slice()];
        parts.extend(path.iter().map(|s| s.as_bytes()));
        let key = artifact::key(3, &parts);
        if let Some(Memo::Value(result)) = self.memory.get(key) {
            self.stats.value_hits += 1;
            return result.map_err(CompileError::Language);
        }
        let result = if let Some(saved) = self.load::<Output>(key)? {
            if let Err(error) = &saved {
                validation::diagnostic(error)?;
            }
            saved
        } else {
            let result = match self.prepare_input(input) {
                Ok(p) => {
                    let mut evaluation = Evaluation::with_computations(&p.graph, self)?;
                    let result = evaluation.select_computed(path);
                    let forced = evaluation.forced_nodes();
                    drop(evaluation);
                    self.stats.forced_nodes += forced;
                    match result {
                        Ok(value) => Ok(value),
                        Err(EvaluationError::Language(e)) => Err(input.error(e, &p.sources)),
                        Err(EvaluationError::Infrastructure(e)) => {
                            return Err(ArtifactError(e.to_string()).into());
                        }
                    }
                }
                Err(CompileError::Language(e)) => Err(e),
                Err(error) => return Err(error),
            };
            self.save(key, &result)?;
            result
        };
        let bytes = match &result {
            Ok(v) => validation::value_bytes(v),
            Err(e) => e
                .source
                .len()
                .saturating_add(e.diagnostic.message.len())
                .saturating_add(512),
        };
        // Avoid copying a large exported value when it will not be retained.
        if self.policy.memory_bytes != 0 && bytes <= self.policy.memory_bytes {
            self.memory.insert(
                key,
                Memo::Value(result.clone()),
                bytes,
                self.policy,
                &mut self.stats,
            );
        }
        result.map_err(CompileError::Language)
    }
}

impl Computations for Compiler {
    fn get(&mut self, key: ArtifactKey) -> Result<Option<Scalar>, CacheError> {
        let value = if let Some(Memo::Computation(value)) = self.memory.get(key) {
            Some(value)
        } else {
            self.load::<Scalar>(key)
                .map_err(|e| CacheError::new("artifact get", None, e.0))?
        };
        if value.is_some() {
            self.stats.computation_hits += 1;
        }
        Ok(value)
    }
    fn put(&mut self, key: ArtifactKey, value: &Scalar) -> Result<(), CacheError> {
        self.save(key, value)
            .map_err(|e| CacheError::new("artifact put", None, e.0))?;
        if value.bytes() <= self.policy.memory_bytes {
            self.memory.insert(
                key,
                Memo::Computation(value.clone()),
                value.bytes(),
                self.policy,
                &mut self.stats,
            );
        }
        self.stats.computation_writes += 1;
        Ok(())
    }
}

type Sources = Arc<Vec<(String, Arc<str>)>>;
#[derive(Clone, Copy)]
enum Input<'a> {
    Source(&'a str),
    Modules(&'a str, &'a BTreeMap<String, ModuleSource>),
}
impl Input<'_> {
    fn key(self) -> ArtifactKey {
        match self {
            Self::Source(source) => artifact::key(1, &[source.as_bytes()]),
            Self::Modules(entry, modules) => {
                // Each module is hashed independently to delimit its binding map.
                // All supplied modules are included, matching graph origin numbering.
                let hashes: Vec<_> = modules
                    .iter()
                    .map(|(name, module)| {
                        let mut parts = vec![name.as_bytes(), module.source.as_bytes()];
                        for (binding, target) in &module.dependencies {
                            parts.extend([binding.as_bytes(), target.as_bytes()]);
                        }
                        artifact::key(4, &parts)
                    })
                    .collect();
                let mut parts = vec![entry.as_bytes()];
                parts.extend(hashes.iter().map(|h| h.0.as_slice()));
                artifact::key(2, &parts)
            }
        }
    }
    fn sources(self) -> Sources {
        let mut sources = vec![(
            "<stdlib>".into(),
            Arc::from(include_str!("../std/collections.lonk")),
        )];
        match self {
            Self::Source(s) => sources.push(("<source>".into(), s.into())),
            Self::Modules(_, modules) => sources.extend(
                modules
                    .iter()
                    .map(|(n, m)| (n.clone(), Arc::from(m.source.as_str()))),
            ),
        }
        Arc::new(sources)
    }
    fn program(self, graph: check::Graph, sources: Sources) -> Program {
        let entry = match self {
            Self::Source(_) => "<source>",
            Self::Modules(entry, _) => entry,
        };
        let source = sources
            .iter()
            .find(|(n, _)| n == entry)
            .map_or_else(|| Arc::from(""), |(_, s)| s.clone());
        Program {
            source,
            graph: Arc::new(graph),
            sources,
        }
    }
    fn error(self, diagnostic: Diagnostic, sources: &Sources) -> ModuleDiagnostic {
        let fallback = match self {
            Self::Source(_) => 1,
            Self::Modules(entry, _) => sources
                .iter()
                .position(|(n, _)| n == entry)
                .unwrap_or(usize::MAX),
        };
        let (module, source) = sources
            .get(diagnostic.origin.unwrap_or(fallback))
            .cloned()
            .unwrap_or_else(|| {
                (
                    match self {
                        Self::Source(_) => "<source>",
                        Self::Modules(entry, _) => entry,
                    }
                    .into(),
                    Arc::from(""),
                )
            });
        ModuleDiagnostic {
            module,
            source: source.to_string(),
            diagnostic,
        }
    }
}
