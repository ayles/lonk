//! Interface-only checking and a linker. The evaluator still sees one graph.
use super::*;
use crate::types::TypeIds;

pub(crate) fn prelude_graph() -> Result<Graph, Diagnostic> {
    let mut checker = Checker::new();
    let (_, library) = checker.standard_library()?;
    checker.roots.insert(0, library);
    checker.check_usefulness()?;
    Ok(Graph {
        nodes: checker.nodes,
        root: library,
    }
    .compact())
}

/// Import zero is the fixed primitive library. Remaining slots are sorted host
/// dependency bindings. No dependency implementation is consulted by checking.
pub(crate) fn compile_unit(
    ast: &syntax::Block,
    imports: &[(String, Type)],
) -> Result<Graph, Diagnostic> {
    let mut checker = Checker::new();
    let mut env = Env::new();
    let mut library = None;
    for (slot, (name, interface)) in imports.iter().enumerate() {
        let mut next = checker.generics.len();
        let ty = TypeIds::default().ty(interface, &mut next);
        for (id, parameter) in ty.generic_parameters() {
            checker.generics.insert(
                id,
                GenericState {
                    kind: Some(parameter.row.is_some()),
                    parameter,
                },
            );
        }
        debug_assert_eq!(
            next,
            checker.generics.len(),
            "imported binder IDs must be dense"
        );
        let value = checker.node(Kind::Import(slot), ty.clone(), Span::default());
        if slot == 0 {
            library = Some(value.id);
            let Type::Record { fields, .. } = ty else {
                return Err(Diagnostic::new(
                    Span::default(),
                    "internal invariant: primitive library interface",
                ));
            };
            for (name, field) in fields.iter() {
                let field = checker.node(
                    Kind::Field(value.id, name.clone()),
                    field.ty.clone(),
                    Span::default(),
                );
                env.insert(
                    name.clone(),
                    Binding {
                        identity: field.id,
                        value: field.id,
                        contract: field.ty.clone(),
                        known: field.ty,
                        owner: 0,
                    },
                );
            }
            checker.primitive_libraries = env.clone();
        } else {
            if !crate::modules::valid_binding(name) {
                return Err(Diagnostic::new(
                    Span::default(),
                    format!("invalid module binding `{name}`"),
                ));
            }
            env.insert(
                name.clone(),
                Binding {
                    identity: value.id,
                    value: value.id,
                    contract: ty.clone(),
                    known: ty,
                    owner: usize::MAX,
                },
            );
        }
    }
    let result = checker.block(ast, &mut env, None)?;
    if result.ty == Type::Unit || result.ty.contains_unresolved() {
        return Err(Diagnostic::new(
            ast.span,
            "a module must produce a value with a resolved type",
        ));
    }
    let mut outputs = vec![(Some("$program".into()), result.id)];
    if let Some(library) = library {
        outputs.push((Some("$library".into()), library));
    }
    let useful = checker.node(Kind::Record(outputs), Type::Any, ast.span);
    checker.roots.insert(0, useful.id);
    checker.check_usefulness()?;
    Ok(Graph {
        nodes: checker.nodes,
        root: result.id,
    }
    .compact())
}

fn map_drops(drops: &mut Drops, ids: &mut TypeIds, next: &mut usize) {
    if let Some(types) = &mut drops.instantiate {
        for ty in types {
            *ty = ids.ty(ty, next);
        }
    }
    for child in drops.fields.values_mut() {
        map_drops(child, ids, next);
    }
    if let Some(child) = &mut drops.element {
        map_drops(child, ids, next);
    }
}

/// Units are supplied in dependency order. Each import names an earlier unit;
/// origin is the source index assigned by the host's canonical module ordering.
pub(crate) fn link_units(units: &[(&Graph, Vec<usize>, usize)], entry: usize) -> Graph {
    let mut needed = vec![false; units.len()];
    let mut pending = vec![entry];
    while let Some(index) = pending.pop() {
        if !std::mem::replace(&mut needed[index], true) {
            for node in &units[index].0.nodes {
                if let Kind::Import(slot) = node.kind {
                    pending.push(units[index].1[slot]);
                }
            }
        }
    }
    let mut nodes = Vec::new();
    let mut roots = vec![usize::MAX; units.len()];
    let mut next_owner = 1;
    let mut next_generic = 0;
    for (index, (graph, imports, origin)) in units.iter().enumerate() {
        if !needed[index] {
            continue;
        }
        let base = nodes.len();
        let mut owners = BTreeMap::from([(0, 0)]);
        let mut types = TypeIds::default();
        let mut owner = |old: usize| {
            *owners.entry(old).or_insert_with(|| {
                let id = next_owner;
                next_owner += 1;
                id
            })
        };
        for original in &graph.nodes {
            let mut node = original.clone();
            node.kind.visit_ids(|id| *id += base);
            node.owner = owner(node.owner);
            node.origin = *origin;
            node.ty = types.ty(&node.ty, &mut next_generic);
            match &mut node.kind {
                Kind::Import(slot) => node.kind = Kind::Alias(roots[imports[*slot]]),
                Kind::Function {
                    owner: id,
                    parameters,
                    ..
                } => {
                    *id = owner(*id);
                    for parameter in parameters {
                        *parameter = types.id(*parameter, &mut next_generic);
                    }
                }
                Kind::For { owner: id, .. } => *id = owner(*id),
                Kind::Project(_, ty, drops) => {
                    *ty = types.ty(ty, &mut next_generic);
                    map_drops(drops, &mut types, &mut next_generic);
                }
                Kind::Is(_, ty) => *ty = types.ty(ty, &mut next_generic),
                Kind::Instantiate(_, arguments) => {
                    for ty in arguments {
                        *ty = types.ty(ty, &mut next_generic);
                    }
                }
                _ => (),
            }
            nodes.push(node);
        }
        roots[index] = base + graph.root;
    }
    Graph {
        nodes,
        root: roots[entry],
    }
    .compact()
}
