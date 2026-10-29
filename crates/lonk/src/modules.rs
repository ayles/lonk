//! Host-owned immutable module sources and explicit dependency bindings.
use crate::{Diagnostic, Program, check, syntax};
use std::collections::BTreeMap;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ModuleSource {
    pub source: String,
    /// Local binding name -> logical module identity. No path interpretation.
    pub dependencies: BTreeMap<String, String>,
}
impl ModuleSource {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            dependencies: BTreeMap::new(),
        }
    }
    pub fn with_dependency(
        mut self,
        binding: impl Into<String>,
        module: impl Into<String>,
    ) -> Self {
        self.dependencies.insert(binding.into(), module.into());
        self
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ModuleDiagnostic {
    pub module: String,
    pub source: String,
    pub diagnostic: Diagnostic,
}
impl std::fmt::Display for ModuleDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}",
            self.module,
            self.diagnostic.render(&self.source)
        )
    }
}
impl std::error::Error for ModuleDiagnostic {}

impl Program {
    /// Prepare a host-supplied module graph. Sources and bindings are immutable;
    /// this API never reads files or interprets logical identities as paths.
    /// Cyclic interfaces are diagnosed until explicit interface linking exists.
    pub fn prepare_modules(
        entry: &str,
        modules: &BTreeMap<String, ModuleSource>,
    ) -> Result<Self, ModuleDiagnostic> {
        let mut sources = vec![(
            "<stdlib>".into(),
            include_str!("../std/collections.lonk").into(),
        )];
        sources.extend(modules.iter().map(|(n, m)| (n.clone(), m.source.clone())));
        let graph = check::compile_modules(entry, modules).map_err(|diagnostic| {
            let (module, source) = sources
                .get(diagnostic.origin.unwrap_or(usize::MAX))
                .cloned()
                .unwrap_or_else(|| (entry.into(), String::new()));
            ModuleDiagnostic {
                module,
                source,
                diagnostic,
            }
        })?;
        let source = modules
            .get(entry)
            .map_or_else(String::new, |m| m.source.clone());
        Ok(Self {
            source: source.into(),
            graph: graph.into(),
            sources: sources
                .into_iter()
                .map(|(n, s)| (n, s.into()))
                .collect::<Vec<_>>()
                .into(),
        })
    }

    /// Render a preparation/evaluation diagnostic against its actual source.
    pub fn render_diagnostic(&self, diagnostic: &Diagnostic) -> String {
        let origin = diagnostic
            .origin
            .unwrap_or(self.graph.nodes[self.graph.root].origin);
        match self.sources.get(origin) {
            Some((name, source)) if name != "<source>" => {
                format!("{name}:{}", diagnostic.render(source))
            }
            Some((_, source)) => diagnostic.render(source),
            None => diagnostic.render(&self.source),
        }
    }
}

pub(crate) fn valid_binding(name: &str) -> bool {
    // Reuse the parser's exact identifier/keyword rules for host bindings.
    let source = format!("let {name} = 0; {name}");
    syntax::parse(&source).is_ok_and(
        |s| matches!(&s.body.statements[..], [syntax::Statement::Let(n, _, _, _)] if n == name),
    )
}
