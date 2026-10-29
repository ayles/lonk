use super::{Graph, Id, Kind, Part};

impl Kind {
    /// All runtime references. Unlike usefulness dependencies this includes
    /// loop back-edges and return outputs. Keep this match exhaustive.
    pub(crate) fn visit_ids(&mut self, mut visit: impl FnMut(&mut Id)) {
        match self {
            Self::Int(_)
            | Self::Bool(_)
            | Self::Str(_)
            | Self::Unit
            | Self::Descriptor
            | Self::Import(_) => (),
            Self::Alias(id)
            | Self::Field(id, _)
            | Self::Unary(_, id)
            | Self::Project(id, _, _)
            | Self::Instantiate(id, _)
            | Self::Is(id, _)
            | Self::LiteralTest(id, _) => visit(id),
            Self::Index(a, b) | Self::Binary(_, a, b) | Self::After(a, b) => {
                visit(a);
                visit(b);
            }
            Self::If(a, b, c) => {
                visit(a);
                visit(b);
                visit(c);
            }
            Self::Record(fields) => {
                for (_, id) in fields {
                    visit(id);
                }
            }
            Self::Array(items) => {
                for (_, id) in items {
                    visit(id);
                }
            }
            Self::Slice(base, start, end) => {
                visit(base);
                for id in start.iter_mut().chain(end) {
                    visit(id);
                }
            }
            Self::Builtin(_, args) => {
                for id in args {
                    visit(id);
                }
            }
            Self::Function { body, defaults, .. } => {
                visit(body);
                for id in defaults {
                    visit(id);
                }
            }
            Self::Parameter(_, default) => {
                if let Some(id) = default {
                    visit(id);
                }
            }
            Self::Call(function, args, places) => {
                visit(function);
                for id in args.iter_mut().chain(places) {
                    visit(id);
                }
            }
            Self::BorrowCheck(base, path, guard) => {
                visit(base);
                if let Some(id) = guard {
                    visit(id);
                }
                for part in path {
                    if let Part::Index(id) = part {
                        visit(id);
                    }
                }
            }
            Self::Update(base, path, value) => {
                visit(base);
                visit(value);
                for part in path {
                    if let Part::Index(id) = part {
                        visit(id);
                    }
                }
            }
            Self::For {
                source,
                initial,
                body,
                updates,
                exit,
                ..
            } => {
                visit(source);
                visit(body);
                for id in initial {
                    visit(id);
                }
                for (_, _, id) in updates {
                    visit(id);
                }
                if let Some((flag, value)) = exit {
                    visit(flag);
                    visit(value);
                }
            }
        }
    }
}

impl Graph {
    /// Checking visits every body; the evaluator retains only nodes reachable
    /// from the exported value. Recursive functions may contain cycles.
    pub(super) fn compact(mut self) -> Self {
        let mut live = vec![false; self.nodes.len()];
        let mut pending = vec![self.root];
        while let Some(id) = pending.pop() {
            if !std::mem::replace(&mut live[id], true) {
                self.nodes[id].kind.visit_ids(|next| pending.push(*next));
            }
        }
        let mut remap = vec![usize::MAX; self.nodes.len()];
        let mut count = 0;
        for (id, used) in live.iter().enumerate() {
            if *used {
                remap[id] = count;
                count += 1;
            }
        }
        let mut id = 0;
        self.nodes.retain(|_| {
            let keep = live[id];
            id += 1;
            keep
        });
        for node in &mut self.nodes {
            node.kind.visit_ids(|id| *id = remap[*id]);
        }
        self.root = remap[self.root];
        self.nodes.shrink_to_fit();
        self
    }
}
