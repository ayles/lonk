use super::*;
use crate::{
    Type,
    check::{Graph, Kind},
};

fn invalid(message: &str) -> ArtifactError {
    ArtifactError(format!("invalid checked artifact: {message}"))
}

pub(super) fn diagnostic(error: &ModuleDiagnostic) -> Result<(), ArtifactError> {
    let span = error.diagnostic.span;
    if span.start > span.end
        || span.end > error.source.len()
        || !error.source.is_char_boundary(span.start)
        || !error.source.is_char_boundary(span.end)
    {
        return Err(invalid("diagnostic span"));
    }
    Ok(())
}

/// Structural validation of trusted producer data, not a second type checker.
pub(super) fn graph(graph: &mut Graph, sources: &Sources) -> Result<(), ArtifactError> {
    unit(graph, sources, 0)
}

pub(super) fn unit(
    graph: &mut Graph,
    sources: &Sources,
    imports: usize,
) -> Result<(), ArtifactError> {
    if graph.root >= graph.nodes.len() {
        return Err(invalid("root"));
    }
    let mut owners = BTreeMap::from([(0, usize::MAX)]);
    for node in &graph.nodes {
        let owner = match &node.kind {
            Kind::Function { owner, .. } => {
                let mut ty = &node.ty;
                if let Type::Generic { body, .. } = ty {
                    ty = body;
                }
                let Type::Function { parameters, .. } = ty else {
                    return Err(invalid("function type"));
                };
                Some((*owner, parameters.len()))
            }
            Kind::For {
                owner,
                initial,
                updates,
                ..
            } => {
                if updates
                    .iter()
                    .any(|(_, i, id)| *i >= initial.len() || *id >= graph.nodes.len())
                {
                    return Err(invalid("loop update"));
                }
                Some((*owner, initial.len() + 1))
            }
            _ => None,
        };
        if let Some((owner, arity)) = owner {
            if owners.insert(owner, arity).is_some() {
                return Err(invalid("duplicate owner"));
            }
        }
    }
    let node_count = graph.nodes.len();
    for node in &mut graph.nodes {
        let Some((_, source)) = sources.get(node.origin) else {
            return Err(invalid("origin"));
        };
        if node.span.start > node.span.end
            || node.span.end > source.len()
            || !source.is_char_boundary(node.span.start)
            || !source.is_char_boundary(node.span.end)
        {
            return Err(invalid("span"));
        }
        if !owners.contains_key(&node.owner) {
            return Err(invalid("owner"));
        }
        let mut bad_id = false;
        node.kind.visit_ids(|id| bad_id |= *id >= node_count);
        if bad_id {
            return Err(invalid("node reference"));
        }
        if !node.ty.within_depth(64) {
            return Err(invalid("type nesting"));
        }
        match &node.kind {
            Kind::Import(slot) if *slot >= imports => return Err(invalid("unlinked import")),
            Kind::Parameter(index, _) if *index >= owners[&node.owner] => {
                return Err(invalid(&format!(
                    "parameter index {index}, owner {}, arity {}",
                    node.owner, owners[&node.owner]
                )));
            }
            Kind::Builtin(op, args) if args.len() != op.arity() => {
                return Err(invalid("builtin arity"));
            }
            _ => (),
        }
    }
    Ok(())
}

pub(super) fn value_bytes(value: &Value) -> usize {
    let mut pending = vec![value];
    let mut bytes: usize = 512;
    while let Some(value) = pending.pop() {
        bytes = bytes.saturating_add(std::mem::size_of::<Value>());
        match value {
            Value::Str(s) => bytes = bytes.saturating_add(s.capacity()),
            Value::Array(v) => pending.extend(v),
            Value::Record(v) => {
                for (key, value) in v {
                    bytes = bytes.saturating_add(key.capacity()).saturating_add(64);
                    pending.push(value);
                }
            }
            _ => (),
        }
    }
    bytes
}
