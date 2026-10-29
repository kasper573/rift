use std::time::Duration;

use driver::{DeployEdit, UiTarget, WaitBudget, arrived_in, in_world, resident_home};
use engine::core::math::Offset;
use engine::systems::npc::NpcDef;

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "deploy",
    title: "Deploy",
    summary: "New content goes live while you play: a restart, a short download, and everyone rejoins",
    play,
};

const EDITION: &str = "Rift, second edition";

// The restart, the reconnect's backoff and the download each take a while.
const REJOINING: WaitBudget = WaitBudget {
    time: Duration::from_secs(60),
    frames: 3600,
};

fn play(shoot: &mut Shoot) {
    let harbour = resident_home("tobb");
    let player = shoot.join_as("talker", Some(harbour));
    let mut me = shoot.roll(player);
    me.caption("A morning at the harbour, on the content the server started with");
    let before = me.wait_for("the player at the harbour", |snapshot| {
        arrived_in(snapshot, harbour.area)
            .then(|| snapshot.me.clone())
            .flatten()
    });
    me.walk_by(Offset::new(3.0, 0.0));
    me.caption(
        "Meanwhile the game's folder changes and the server restarts on it: a content deploy",
    );
    let changed = shoot.deploy(
        &[
            DeployEdit::game(|game| game.name = EDITION.to_owned()),
            DeployEdit::file("mara", |npc: &mut NpcDef| {
                npc.name = "Mara the Harbourmaster".to_owned()
            }),
            DeployEdit::file("tobb", |npc: &mut NpcDef| {
                npc.name = "Tobb, town crier".to_owned()
            }),
            DeployEdit::file("grisha", |npc: &mut NpcDef| {
                npc.name = "Grisha of the quay".to_owned()
            }),
        ],
        || {
            me.wait_within("the client to lose the server", REJOINING, |snapshot| {
                UiTarget::text("Reconnecting…").find(snapshot).map(|_| ())
            });
            me.caption(
                "The world freezes under a veil while the client reconnects, with backoff so a crowd doesn't dogpile the server",
            );
            me.hold();
        },
    );
    me.wait_within("the new edition to load", REJOINING, |snapshot| {
        UiTarget::text(EDITION).find(snapshot).map(|_| ())
    });
    me.caption(&format!(
        "The new manifest differs, so only the {changed} changed rows download; every other file is still on the device"
    ));
    me.wait_within("a new character in the world", REJOINING, |snapshot| {
        (in_world(snapshot) && snapshot.me.as_ref().is_some_and(|now| now.id != before.id))
            .then_some(())
    });
    me.caption("Characters live in the server's memory, so everyone rejoins as a new character at the spawn, on the new content");
    me.hold();
}
