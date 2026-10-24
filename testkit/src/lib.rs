//! What the engine's tests stand on: a game of only the rows a test declares, loaded the way the
//! engine loads any game, and a bare world running only the systems a test names.

mod art;
mod bag;
mod fixtures;
mod game;
mod notices;
mod players;
mod script;
mod sim;
mod talk;

pub use art::{glb, png, wav};
pub use game::{APEX, CLIP, Declared, Game, Map, TILE, path, stats};
pub use script::{dialogue, hunting, said};
pub use sim::{Sim, drawn_seed};
pub use talk::{labels, lines, locked};
