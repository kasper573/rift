//! What the engine's tests stand on: a game of only the rows a test declares, loaded the way the
//! engine loads any game, and a bare world running only the systems a test names. The server bench
//! stands on one such game too, the crowd.

mod art;
mod bag;
mod crowd;
mod fixtures;
mod game;
mod notices;
mod players;
mod script;
mod sim;
mod talk;

pub use art::{
    box_glb, glb, glb_naming_image, glb_of, glb_with_image_inside, library_glb, png, pose_glb, rig,
    rig_glb, skin_glb, wav,
};
pub use crowd::crowd;
pub use game::{
    APEX, BODY, CLIP, Declared, Game, HEIGHTS, Map, ROCK, RUN_PACE, TURF, WALK_PACE, clip, clip_on,
    look, part_file, path, rig_clips, rig_file, speech, stats, typing,
};
pub use script::{dialogue, hunting, said};
pub use sim::{Sim, drawn_seed, sent};
pub use talk::{labels, lines, locked};
