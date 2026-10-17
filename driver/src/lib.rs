//! Plays the game as a player would: a native game server, and desktop clients on screens and
//! speakers of their own, driven through real input and watched through each client's probe. The
//! gameplay tests and the showcase both play through it.

mod content;
mod desktop;
mod player;
mod playtest;
mod recording;
mod seen;
mod server;

pub use content::{
    area_named, beside_object, beside_resident, content, from_json, ground_near, hunting_ground,
    id, marker_at, object_at, open_ground_beside, pack_ground, resident_home, to_json,
};
pub use desktop::{BUDGET, WaitBudget};
pub use player::{LONG, Pace, Player, reading};
pub use playtest::{ContentEdit, Playtest, PlaytestSetup};
pub use recording::{Footage, FootageOverlay, InsetCut, seconds};
pub use seen::{
    UiTarget, aiming, area_key, arrived_in, at_rest, bare_spot, beside_other_player, blocked,
    carried, clickable_point, cooling, erred, finished, fixture, holding, in_warp, in_world,
    learned, marked, me_at_rest, nearest_npc, noticed, npc_is, on_quest, open, other_player,
    other_player_within, planned, played, played_on, playing, portal, pressed, reset_prompted,
    slot_item, slot_skill, status_on_me, taken, to_spend, told_by, townsperson, tree_view,
};
