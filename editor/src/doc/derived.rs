use std::sync::OnceLock;

use super::{NodeId, SceneDoc};

/// A value worked out for each node of a document as it is first asked for, and kept as long as
/// what it is worked out from stays the same.
pub struct Derived<T> {
    generation: u32,
    values: Vec<OnceLock<T>>,
}

impl<T> Derived<T> {
    pub fn of(doc: &SceneDoc) -> Derived<T> {
        Derived {
            generation: doc.generation,
            values: (0..doc.entries.len()).map(|_| OnceLock::new()).collect(),
        }
    }

    /// The value of `id`, worked out by `make` the first time it is asked for; none for a node of
    /// another document.
    pub fn get(&self, id: NodeId, make: impl FnOnce() -> T) -> Option<&T> {
        if id.generation != self.generation {
            return None;
        }
        Some(self.values.get(id.index as usize)?.get_or_init(make))
    }
}
