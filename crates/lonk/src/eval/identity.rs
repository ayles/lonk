//! Content identities for checked computations, without demanding their inputs.
//!
//! Spans, source origins and arena offsets are not semantic. Owner edges name
//! lexical scopes structurally. Generic IDs are retained conservatively:
//! renumbering them can cause a miss, but cannot confuse two binders.
use crate::{
    artifact::{self, ArtifactError, ArtifactKey},
    check::{Graph, Id, Kind},
    codec,
};
use std::collections::BTreeMap;

pub(super) struct Identities {
    pub nodes: Vec<ArtifactKey>,
    pub owners: BTreeMap<usize, Id>,
    pub boundaries: Vec<bool>,
}

impl Identities {
    pub fn new(graph: &Graph) -> Result<Self, ArtifactError> {
        let n = graph.nodes.len();
        let mut owners = BTreeMap::new();
        let mut boundaries = vec![false; n];
        boundaries[graph.root] = true;
        for (id, node) in graph.nodes.iter().enumerate() {
            match &node.kind {
                Kind::Function { owner, body, .. } => {
                    owners.insert(*owner, id);
                    boundaries[*body] = true;
                }
                Kind::For { owner, .. } => {
                    owners.insert(*owner, id);
                }
                Kind::Record(fields) => {
                    for (_, id) in fields {
                        boundaries[*id] = true;
                    }
                }
                Kind::Array(items) => {
                    for (_, id) in items {
                        boundaries[*id] = true;
                    }
                }
                Kind::Call(..) => boundaries[id] = true,
                _ => (),
            }
        }
        let mut edges = vec![Vec::new(); n];
        let mut reverse = vec![Vec::new(); n];
        let mut payloads = Vec::with_capacity(n);
        for (id, node) in graph.nodes.iter().enumerate() {
            let mut kind = node.kind.clone();
            kind.visit_ids(|child| {
                edges[id].push(*child);
                *child = 0;
            });
            match &mut kind {
                Kind::Function { owner, .. } | Kind::For { owner, .. } => *owner = 0,
                _ => (),
            }
            if node.owner != 0 {
                let owner = owners
                    .get(&node.owner)
                    .ok_or_else(|| ArtifactError("missing computation owner".into()))?;
                edges[id].push(*owner);
            }
            for child in &edges[id] {
                reverse[*child].push(id);
            }
            payloads.push(codec::fingerprint(&(kind, &node.ty))?);
            // Literal loads and container construction are cheaper than a store
            // lookup. Their identities still participate in callers' keys.
            boundaries[id] &= !matches!(
                node.kind,
                Kind::Int(_)
                    | Kind::Bool(_)
                    | Kind::Str(_)
                    | Kind::Unit
                    | Kind::Descriptor
                    | Kind::Record(_)
                    | Kind::Array(_)
                    | Kind::Function { .. }
                    | Kind::Parameter(..)
                    | Kind::Alias(_)
            );
        }
        // Iterative Kosaraju: neither a long expression nor recursive functions
        // can overflow the Rust stack while identities are being constructed.
        let mut seen = vec![false; n];
        let mut order = Vec::with_capacity(n);
        for root in 0..n {
            if seen[root] {
                continue;
            }
            seen[root] = true;
            let mut pending = vec![(root, 0)];
            while let Some((id, at)) = pending.last_mut() {
                if let Some(child) = edges[*id].get(*at) {
                    *at += 1;
                    if !std::mem::replace(&mut seen[*child], true) {
                        pending.push((*child, 0));
                    }
                } else {
                    order.push(*id);
                    pending.pop();
                }
            }
        }
        let mut component = vec![usize::MAX; n];
        let mut groups = Vec::new();
        for root in order.into_iter().rev() {
            if component[root] != usize::MAX {
                continue;
            }
            let index = groups.len();
            component[root] = index;
            let mut pending = vec![root];
            let mut group = Vec::new();
            while let Some(id) = pending.pop() {
                group.push(id);
                for parent in &reverse[id] {
                    if component[*parent] == usize::MAX {
                        component[*parent] = index;
                        pending.push(*parent);
                    }
                }
            }
            group.sort_unstable();
            groups.push(group);
        }
        let mut positions = vec![0; n];
        for group in &groups {
            for (position, id) in group.iter().enumerate() {
                positions[*id] = position;
            }
        }
        let mut nodes = vec![ArtifactKey([0; 32]); n];
        for (index, group) in groups.iter().enumerate().rev() {
            // Explicit tags distinguish internal references from external hashes.
            #[derive(serde::Serialize)]
            enum Edge {
                Internal(usize),
                External([u8; 32]),
            }
            let members: Vec<_> = group
                .iter()
                .map(|id| {
                    let dependencies: Vec<_> = edges[*id]
                        .iter()
                        .map(|child| {
                            if component[*child] == index {
                                Edge::Internal(positions[*child])
                            } else {
                                debug_assert!(component[*child] > index);
                                Edge::External(nodes[*child].0)
                            }
                        })
                        .collect();
                    (payloads[*id], dependencies)
                })
                .collect();
            let hash = codec::fingerprint(&members)?;
            for (position, id) in group.iter().enumerate() {
                nodes[*id] = artifact::key(7, &[&hash, &(position as u64).to_le_bytes()]);
            }
        }
        Ok(Self {
            nodes,
            owners,
            boundaries,
        })
    }
}
