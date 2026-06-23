//! The static game catalog, baked into the binary and shared by client and server: actor models,
//! areas, items, and sound effects. Server-only content (npc, reward, and spawn tables) lives with the
//! systems that consume it in [`crate::systems`], since the client never sees it.

pub mod actors;
pub mod area;
pub mod items;
pub mod sfx;
