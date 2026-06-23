//! The static game catalog, baked into the binary and shared by client and host: actor models,
//! areas, items, and sound effects. Host-only content (npc, reward, and spawn tables) lives with the
//! simulation systems that consume it in [`crate::sim`], since the client never sees it.

pub mod actors;
pub mod area;
pub mod items;
pub mod sfx;
