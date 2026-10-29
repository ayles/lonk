use super::*;
use crate::ModuleSource;

pub(crate) fn compile_modules(
    entry: &str,
    modules: &BTreeMap<String, ModuleSource>,
) -> Result<Graph, Diagnostic> {
    compile_modules_cached(entry, modules, &Prelude::new()?, &mut |source| {
        syntax::parse(source).map(Arc::new)
    })
}

pub(crate) fn compile_modules_cached(
    entry: &str,
    modules: &BTreeMap<String, ModuleSource>,
    seed: &Prelude,
    parse: &mut dyn FnMut(&str) -> Result<Arc<syntax::Syntax>, Diagnostic>,
) -> Result<Graph, Diagnostic> {
    let mut checker = seed.checker.clone();
    let prelude = &seed.env;
    let library = seed.library;
    let origins: BTreeMap<_, _> = modules
        .keys()
        .enumerate()
        .map(|(i, n)| (n.clone(), i + 1))
        .collect();
    let mut prepared = BTreeMap::new();
    let mut inputs = Inputs {
        modules,
        prelude,
        origins: &origins,
        parse,
    };
    let result = checker.module(entry, &mut inputs, &mut prepared, &mut Vec::new())?;
    let mut outputs = vec![(Some("$library".into()), library)];
    outputs.extend(
        prepared
            .into_iter()
            .map(|(n, v): (String, Checked)| (Some(n), v.id)),
    );
    let usefulness = checker.node(Kind::Record(outputs), Type::Any, Span::default());
    checker.roots.insert(0, usefulness.id);
    checker.check_usefulness()?;
    Ok(Graph {
        nodes: checker.nodes,
        root: result.id,
    }
    .compact())
}

struct Inputs<'a> {
    modules: &'a BTreeMap<String, ModuleSource>,
    prelude: &'a Env,
    origins: &'a BTreeMap<String, usize>,
    parse: &'a mut dyn FnMut(&str) -> Result<Arc<syntax::Syntax>, Diagnostic>,
}

impl Checker {
    fn module(
        &mut self,
        name: &str,
        inputs: &mut Inputs<'_>,
        prepared: &mut BTreeMap<String, Checked>,
        visiting: &mut Vec<String>,
    ) -> Result<Checked, Diagnostic> {
        let modules = inputs.modules;
        let origins = inputs.origins;
        let prelude = inputs.prelude;
        if let Some(value) = prepared.get(name) {
            return Ok(value.clone());
        }
        if visiting.iter().any(|n| n == name) {
            let cycle = visiting
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(name))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(Diagnostic::new(
                Span::default(),
                format!(
                    "module dependency cycle: {cycle}; cyclic module interfaces are not implemented"
                ),
            )
            .with_origin(origins[name]));
        }
        if visiting.len() >= 128 {
            return Err(Diagnostic::new(
                Span::default(),
                "module dependency depth limit exceeded (128)",
            ));
        }
        let module = modules.get(name).ok_or_else(|| {
            Diagnostic::new(
                Span::default(),
                format!("unknown host-supplied module `{name}`"),
            )
        })?;
        visiting.push(name.into());
        let mut env = prelude.clone();
        for (binding, dependency) in &module.dependencies {
            if !crate::modules::valid_binding(binding) {
                return Err(Diagnostic::new(
                    Span::default(),
                    format!("invalid module binding `{binding}`"),
                )
                .with_origin(origins[name]));
            }
            let value = self
                .module(dependency, inputs, prepared, visiting)
                .map_err(|e| e.with_origin(origins[name]))?;
            env.insert(
                binding.clone(),
                Binding {
                    identity: value.id,
                    value: value.id,
                    known: value.ty.clone(),
                    contract: value.ty,
                    owner: usize::MAX,
                },
            );
        }
        self.origin = origins[name];
        let ast = (inputs.parse)(&module.source).map_err(|e| e.with_origin(self.origin))?;
        let result = self
            .block(&ast.body, &mut env, None)
            .map_err(|e| e.with_origin(self.origin))?;
        if result.ty == Type::Unit || result.ty.contains_unresolved() {
            return Err(Diagnostic::new(
                ast.body.span,
                "a module must produce a value with a resolved type",
            )
            .with_origin(self.origin));
        }
        visiting.pop();
        prepared.insert(name.into(), result.clone());
        Ok(result)
    }
}
