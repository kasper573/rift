//! Plays the game as a player would: a native game server, and desktop clients on screens and
//! speakers of their own, driven through real input and watched through each client's probe. The
//! showcase and the client bench play through it.

mod content;
mod desktop;
mod memory;
mod player;
mod playtest;
mod recording;
mod seen;
mod server;

pub use content::{
    area_named, content, east_of, ground_near, held_in_stance, hunting_ground, id, initial_spawn,
    input_name, input_names, lamps_near, marker_at, open_ground_beside, pack_ground,
    pictured_ground, props_near, resident_home,
};
pub use desktop::{BUDGET, WaitBudget};
pub use player::{LONG, Pace, Player, reading};
pub use playtest::{DeployEdit, Playtest, PlaytestSetup};
pub use recording::{Footage, FootageOverlay, InsetCut, seconds};
pub use seen::{
    UiTarget, aiming, area_name, arrived_in, bare_spot, beside_other_player, blocked, carried,
    clickable_point, cooling, erred, finished, holding, in_world, learned, marked, nearest_npc,
    noticed, npc_is, object_named, on_quest, open, other_player, other_player_within, planned,
    planned_rank, played, played_on, player_named, playing, pressed, reset_prompted, slot_item,
    slot_skill, status_on_me, surrounded_by, taken, to_spend, told_by, townsperson, tree_view,
    warp_named,
};
