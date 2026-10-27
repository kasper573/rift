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

pub use art::{glb, glb_of, library_glb, png, pose_glb, rig, rig_glb, skin_glb, wav, wav_of};
pub use game::{
    APEX, BODY, CLIP, Declared, Game, LIBRARY, Map, ROCK, RUN_PACE, SPRINT_PACE, TURF, WALK_PACE,
    path, rig_clips, stats,
};
pub use script::{dialogue, hunting, said};
pub use sim::{Sim, drawn_seed};
pub use talk::{labels, lines, locked};
