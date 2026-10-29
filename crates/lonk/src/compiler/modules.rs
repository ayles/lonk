//! Module bodies are cached against imported interfaces, then linked separately.
use super::*;

struct Unit {
    graph: Arc<check::Graph>,
    imports: Vec<usize>,
    origin: usize,
    interface: [u8; 32],
}
struct Load<'a> {
    modules: &'a BTreeMap<String, ModuleSource>,
    sources: Sources,
    indices: BTreeMap<String, usize>,
    visiting: Vec<String>,
    units: Vec<Unit>,
}

impl Compiler {
    fn library(&mut self) -> Result<Arc<check::Graph>, CompileError> {
        if let Some(library) = &self.library {
            return Ok(library.clone());
        }
        let key = artifact::key(5, &[]);
        let source: Arc<str> = include_str!("../../std/collections.lonk").into();
        let sources = Arc::new(vec![("<stdlib>".into(), source.clone())]);
        let mut graph = if let Some(graph) = self.load::<check::Graph>(key)? {
            graph
        } else {
            self.stats.prelude_checks += 1;
            let graph = check::prelude_graph().map_err(|diagnostic| {
                CompileError::Language(ModuleDiagnostic {
                    module: "<stdlib>".into(),
                    source: source.to_string(),
                    diagnostic,
                })
            })?;
            self.save(key, &graph)?;
            graph
        };
        validation::graph(&mut graph, &sources)?;
        let graph = Arc::new(graph);
        if self.policy.memory_bytes != 0 {
            self.library = Some(graph.clone());
        }
        Ok(graph)
    }

    pub(super) fn module_program(
        &mut self,
        entry: &str,
        modules: &BTreeMap<String, ModuleSource>,
        sources: Sources,
    ) -> Result<Program, CompileError> {
        let library = self.library()?;
        let interface = codec::fingerprint(&library.nodes[library.root].ty.canonical_ids())?;
        let mut load = Load {
            modules,
            sources: sources.clone(),
            indices: BTreeMap::new(),
            visiting: Vec::new(),
            units: vec![Unit {
                graph: library,
                imports: Vec::new(),
                origin: 0,
                interface,
            }],
        };
        let root = load.module(self, entry)?;
        let units: Vec<_> = load
            .units
            .iter()
            .map(|unit| (unit.graph.as_ref(), unit.imports.clone(), unit.origin))
            .collect();
        let mut graph = check::link_units(&units, root);
        validation::graph(&mut graph, &sources)?;
        Ok(Input::Modules(entry, modules).program(graph, sources))
    }
}

impl Load<'_> {
    fn error(&self, name: &str, diagnostic: Diagnostic) -> CompileError {
        let origin = self.sources.iter().position(|(n, _)| n == name);
        let diagnostic = Diagnostic {
            origin,
            ..diagnostic
        };
        CompileError::Language(ModuleDiagnostic {
            module: name.into(),
            source: self
                .modules
                .get(name)
                .map_or_else(String::new, |m| m.source.clone()),
            diagnostic,
        })
    }

    fn module(&mut self, compiler: &mut Compiler, name: &str) -> Result<usize, CompileError> {
        if let Some(index) = self.indices.get(name) {
            return Ok(*index);
        }
        if self.visiting.iter().any(|n| n == name) {
            return Err(self.error(
                name,
                Diagnostic::new(
                    crate::Span::default(),
                    format!(
                        "module dependency cycle: {}; cyclic module interfaces are not implemented",
                        self.visiting
                            .iter()
                            .map(String::as_str)
                            .chain(std::iter::once(name))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    ),
                ),
            ));
        }
        if self.visiting.len() >= 128 {
            return Err(self.error(
                name,
                Diagnostic::new(
                    crate::Span::default(),
                    "module dependency depth limit exceeded (128)",
                ),
            ));
        }
        let modules = self.modules;
        let module = modules.get(name).ok_or_else(|| {
            self.error(
                name,
                Diagnostic::new(
                    crate::Span::default(),
                    format!("unknown host-supplied module `{name}`"),
                ),
            )
        })?;
        self.visiting.push(name.into());
        let mut imports = vec![0];
        for (binding, target) in &module.dependencies {
            if !crate::modules::valid_binding(binding) {
                return Err(self.error(
                    name,
                    Diagnostic::new(
                        crate::Span::default(),
                        format!("invalid module binding `{binding}`"),
                    ),
                ));
            }
            imports.push(self.module(compiler, target)?);
        }
        let mut parts = vec![
            name.as_bytes(),
            module.source.as_bytes(),
            self.units[0].interface.as_slice(),
        ];
        for ((binding, target), index) in module.dependencies.iter().zip(imports.iter().skip(1)) {
            parts.extend([
                binding.as_bytes(),
                target.as_bytes(),
                self.units[*index].interface.as_slice(),
            ]);
        }
        let key = artifact::key(6, &parts);
        let origin = self
            .sources
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| ArtifactError("missing module origin".into()))?;
        let result = if let Some(Memo::Unit(result)) = compiler.memory.get(key) {
            compiler.stats.module_hits += 1;
            result
        } else {
            let local_sources =
                Arc::new(vec![self.sources[0].clone(), self.sources[origin].clone()]);
            let saved = compiler.load::<Result<check::Graph, Diagnostic>>(key)?;
            let result = match saved {
                Some(mut result) => {
                    compiler.stats.module_hits += 1;
                    match &mut result {
                        Ok(graph) => validation::unit(graph, &local_sources, imports.len())?,
                        Err(diagnostic) => validation::diagnostic(&ModuleDiagnostic {
                            module: name.into(),
                            source: module.source.clone(),
                            diagnostic: diagnostic.clone(),
                        })?,
                    }
                    result.map(Arc::new)
                }
                None => {
                    compiler.stats.module_checks += 1;
                    let interfaces: Vec<_> = std::iter::once("$stdlib".to_string())
                        .chain(module.dependencies.keys().cloned())
                        .zip(imports.iter())
                        .map(|(binding, index)| {
                            let graph = &self.units[*index].graph;
                            (binding, graph.nodes[graph.root].ty.clone())
                        })
                        .collect();
                    let mut result = compiler
                        .memory
                        .parse(&module.source, compiler.policy, &mut compiler.stats)
                        .and_then(|ast| check::compile_unit(&ast.body, &interfaces));
                    if let Ok(graph) = &mut result {
                        validation::unit(graph, &local_sources, imports.len())?;
                    }
                    compiler.save(key, &result)?;
                    result.map(Arc::new)
                }
            };
            let bytes = module
                .source
                .len()
                .saturating_mul(32)
                .saturating_add(match &result {
                    Ok(graph) => graph
                        .nodes
                        .len()
                        .saturating_mul(std::mem::size_of::<check::Node>() * 2),
                    Err(diagnostic) => diagnostic.message.len() + 512,
                });
            compiler.memory.insert(
                key,
                Memo::Unit(result.clone()),
                bytes,
                compiler.policy,
                &mut compiler.stats,
            );
            result
        };
        let graph = result.map_err(|diagnostic| self.error(name, diagnostic))?;
        let interface = codec::fingerprint(&graph.nodes[graph.root].ty.canonical_ids())?;
        let index = self.units.len();
        self.units.push(Unit {
            graph,
            imports,
            origin,
            interface,
        });
        self.indices.insert(name.into(), index);
        self.visiting.pop();
        Ok(index)
    }
}
