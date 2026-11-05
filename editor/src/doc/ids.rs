use std::sync::atomic::{AtomicU32, Ordering};

/// A node of a scene document, known by its place in the document that gave it out and good only
/// within that document: another document finds no node by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    pub(super) index: u32,
    pub(super) generation: u32,
}

/// What tells one document's ids from another's: each document takes the next.
pub(super) fn next_generation() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
