pub mod actions;
pub mod combat;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod regen;
pub mod replication;
pub mod rewards;
pub mod sfx;
pub mod spectate;
pub mod visibility;

use rift::Feature;

// Registration order is run order: reset → regen → npc_ai → combat → movement → npc_respawn.
pub fn all() -> Vec<Feature> {
    vec![
        replication::feature,
        actions::feature,
        regen::feature,
        npc::spawner,
        npc::ai,
        movement::input,
        combat::feature,
        items::feature,
        rewards::feature,
        movement::step,
        player::feature,
        spectate::feature,
        npc::respawn,
        visibility::feature,
    ]
}
