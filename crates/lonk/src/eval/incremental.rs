//! Restartable demand memoization. All identities describe suspended work; key
//! construction never forces an argument or a sibling field.
use super::*;
use crate::artifact::{self, ArtifactError, ArtifactKey};
use crate::codec;
use identity::Identities;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) enum Scalar {
    Int(i64),
    Bool(bool),
    Str(String),
}
impl Scalar {
    fn from_ready(value: &Ready) -> Option<Self> {
        match value {
            Ready::Int(n) => Some(Self::Int(*n)),
            Ready::Bool(b) => Some(Self::Bool(*b)),
            Ready::Str(s) => Some(Self::Str(s.to_string())),
            _ => None,
        }
    }
    fn ready(self) -> Ready {
        match self {
            Self::Int(n) => Ready::Int(n),
            Self::Bool(b) => Ready::Bool(b),
            Self::Str(s) => Ready::Str(s.into()),
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        128usize.saturating_add(match self {
            Self::Str(s) => s.len(),
            _ => 0,
        })
    }
}

pub(crate) trait Computations {
    fn get(&mut self, key: ArtifactKey) -> Result<Option<Scalar>, CacheError>;
    fn put(&mut self, key: ArtifactKey, value: &Scalar) -> Result<(), CacheError>;
}
struct Slot {
    key: Option<ArtifactKey>,
    cacheable: bool,
    checked: bool,
}
pub(super) struct Incremental<'a> {
    identities: Identities,
    frames: Vec<Option<ArtifactKey>>,
    slots: Vec<Slot>,
    store: &'a mut dyn Computations,
}
impl<'a> Incremental<'a> {
    pub fn new(graph: &Graph, store: &'a mut dyn Computations) -> Result<Self, ArtifactError> {
        Ok(Self {
            identities: Identities::new(graph)?,
            frames: vec![Some(artifact::key(8, &[]))],
            slots: Vec::new(),
            store,
        })
    }
    fn expression(&self, id: Id, frame: FrameId) -> Option<ArtifactKey> {
        Some(artifact::key(
            9,
            &[&self.identities.nodes[id].0, &self.frames[frame]?.0],
        ))
    }
    fn work_key(&self, work: &Work) -> Option<ArtifactKey> {
        match work {
            Work::Expr(id, frame) => self.expression(*id, *frame),
            Work::Forward(id) => self.slots[*id].key,
            Work::Project(id, ty, drops) => Some(artifact::key(
                10,
                &[
                    &self.slots[*id].key?.0,
                    &codec::fingerprint(&(ty, drops)).ok()?,
                ],
            )),
            Work::Update(id, path, value) => Some(artifact::key(
                11,
                &[
                    &self.slots[*id].key?.0,
                    &codec::fingerprint(path).ok()?,
                    &self.slots[*value].key?.0,
                ],
            )),
            // Loop continuations replace an existing expression, retaining its
            // original slot key. A free-standing continuation has no identity.
            Work::Loop(_) => None,
        }
    }
    pub fn work(&mut self, index: ThunkId, work: &Work) {
        debug_assert_eq!(index, self.slots.len());
        let cacheable = matches!(work, Work::Expr(id, _) if self.identities.boundaries[*id]);
        self.slots.push(Slot {
            key: self.work_key(work),
            cacheable,
            checked: false,
        });
    }
    pub fn ready(&mut self, index: ThunkId, value: &Ready) {
        debug_assert_eq!(index, self.slots.len());
        let key = Scalar::from_ready(value)
            .and_then(|v| codec::fingerprint(&v).ok())
            .map(|h| artifact::key(12, &[&h]));
        self.slots.push(Slot {
            key,
            cacheable: false,
            checked: true,
        });
    }
    pub fn frame(&mut self, index: FrameId, frame: &Frame) {
        debug_assert_eq!(index, self.frames.len());
        let key = (|| {
            let owner = self.identities.nodes[*self.identities.owners.get(&frame.owner)?];
            let parent = self.frames[frame.parent?]?;
            let types = codec::fingerprint(frame.types.as_ref()).ok()?;
            let arguments: Vec<_> = frame
                .arguments
                .iter()
                .map(|id| self.slots[*id].key)
                .collect::<Option<_>>()?;
            let mut parts = vec![owner.0.as_slice(), parent.0.as_slice(), types.as_slice()];
            parts.extend(arguments.iter().map(|k| k.0.as_slice()));
            Some(artifact::key(13, &parts))
        })();
        self.frames.push(key);
    }
    pub fn load(&mut self, id: ThunkId) -> Result<Option<Ready>, CacheError> {
        let slot = &mut self.slots[id];
        if !slot.cacheable || slot.checked {
            return Ok(None);
        }
        let result = if let Some(key) = slot.key {
            self.store.get(artifact::key(14, &[&key.0]))?
        } else {
            None
        };
        slot.checked = true;
        Ok(result.map(Scalar::ready))
    }
    pub fn save(&mut self, id: ThunkId, value: &Ready) -> Result<(), CacheError> {
        let slot = &self.slots[id];
        if slot.cacheable {
            if let (Some(key), Some(value)) = (slot.key, Scalar::from_ready(value)) {
                self.store.put(artifact::key(14, &[&key.0]), &value)?;
            }
        }
        Ok(())
    }
}

impl<'a> Evaluation<'a> {
    pub(crate) fn with_computations(
        graph: &'a Graph,
        store: &'a mut dyn Computations,
    ) -> Result<Self, ArtifactError> {
        let mut evaluation = Self::new(graph);
        evaluation.incremental = Some(Incremental::new(graph, store)?);
        Ok(evaluation)
    }
    pub(crate) fn select_computed(&mut self, path: &[&str]) -> Result<Value, EvaluationError> {
        self.select_inner(path)
    }
    pub(super) fn push_frame(&mut self, frame: Frame) -> FrameId {
        let id = self.frames.len();
        if let Some(incremental) = &mut self.incremental {
            incremental.frame(id, &frame);
        }
        self.frames.push(frame);
        id
    }
    pub(super) fn memoized(&mut self, id: ThunkId) -> Result<(), CacheError> {
        if let Some(value) = self
            .incremental
            .as_mut()
            .map(|m| m.load(id))
            .transpose()?
            .flatten()
        {
            self.register_ready(id, &value);
            self.thunks[id].state = State::Ready(value);
            self.thunks[id].work = None;
        }
        Ok(())
    }
}
