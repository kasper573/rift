use std::time::Duration;

use driver::{WaitBudget, clickable_point, content, id, marker_at, nearest_npc, npc_is};
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::Action;
use engine::systems::input::map::InputAction;
use engine::systems::npc::NpcDef;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};

use crate::chapters::Chapter;
use crate::shoot::{Shoot, Star};

pub const CHAPTER: Chapter = Chapter {
    name: "bodies",
    title: "Bodies",
    summary: "Wolves, spiders, slimes, bats and more: every body moves the way it is built",
    play,
};

/// A body in the wilds: its npc, what is said of it and what must show for it to be seen doing so.
type Body = (&'static str, &'static str, fn(&ProbeBody) -> bool);

/// In the order a walk round the island takes them.
const BODIES: [Body; 6] = [
    (
        "wolf",
        "A wolf trots on planted paws: each foot stays where it touched the ground until it steps",
        |wolf| wolf.layered && !wolf.resting,
    ),
    ("piranha", "Piranhas wave through the shallows", |piranha| {
        piranha.layered
    }),
    (
        "slime",
        "A slime squashes as it hops, keeping its volume",
        |slime| slime.layered,
    ),
    (
        "giant_spider",
        "Spiders walk on eight planted legs, four at a time",
        |spider| spider.layered && !spider.resting,
    ),
    (
        "spider",
        "Smaller spiders share the gait on a body of their own",
        |spider| spider.layered,
    ),
    (
        "vampire_bat",
        "A bat keeps aloft on trailing wings. It is listed with a sword, but has no hands: the sword is never shown, and it bites",
        |bat| {
            let listed = id::<NpcDef>("vampire_bat").get(content()).wearing.len();
            bat.flies && listed > 0 && bat.held.is_empty() && bat.action == Action::Attack
        },
    ),
];

fn play(shoot: &mut Shoot) {
    let player = shoot.join_as("veteran", None);
    let mut me = shoot.roll(player);
    me.caption("Every body wears a skeleton: its rig, its clips and the layers that move it");
    me.hold();

    close_up(
        &mut me,
        "By the stone hum a turret turns on its base and fires at whoever comes near",
        ("turret", marker_at(AREA, "turret")),
        |turret| turret.action == Action::Attack,
    );
    for (npc, says, shows) in BODIES {
        let ground = me.look_out(npc, |snapshot| nearest_npc(snapshot, npc).is_some());
        close_up(&mut me, says, (npc, ground), shows);
    }
}

const AREA: &str = "island";

/// Close and low, as a walker would see it, since what a body's layers move is its feet, arms and
/// tail: heads on toward `ground` until one of the `npc`s on screen shows what `shows` asks. It
/// never walks up to the body, which would set a hostile one on the player rather than moving.
fn close_up(
    me: &mut Star,
    says: &str,
    (npc, ground): (&'static str, Pos<Meters>),
    shows: fn(&ProbeBody) -> bool,
) {
    me.zoom_camera(1);
    for _ in 0..2 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.look_for(
        &format!("a {npc} showing it close up"),
        ground,
        |snapshot| {
            showing(snapshot, npc, shows).is_some_and(|at| clickable_point(snapshot, at).is_some())
        },
    );
    me.caption(says);
    me.hold();
    me.camera_home();
    see_off(me);
}

/// Hunters keep pace with a walker and never let it go: whatever set on the player while it looked
/// on is fought off, rather than following it into the next body's shot.
fn see_off(me: &mut Star) {
    while me
        .try_wait_for(BETWEEN_SWINGS, |snapshot| set_upon(snapshot).then_some(()))
        .is_some()
    {
        me.fight(sets_on_me, |snapshot| !set_upon(snapshot));
    }
}

/// Longer than any hunter takes between one swing and the next.
const BETWEEN_SWINGS: WaitBudget = WaitBudget {
    time: Duration::from_secs(2),
    frames: 120,
};

/// Within a bite or a swing of the player.
const SET_UPON_WITHIN: Meters = Meters(3.0);

fn set_upon(snapshot: &ProbeSnapshot) -> bool {
    snapshot
        .actors
        .iter()
        .any(|body| sets_on_me(snapshot, body))
}

fn sets_on_me(snapshot: &ProbeSnapshot, body: &ProbeBody) -> bool {
    body.health > 0.0
        && body.npc.is_some()
        && !body.friendly
        && body.action == Action::Attack
        && snapshot
            .me
            .as_ref()
            .is_some_and(|me| body.at.distance(me.at) <= SET_UPON_WITHIN)
}

fn showing(
    snapshot: &ProbeSnapshot,
    npc: &str,
    shows: fn(&ProbeBody) -> bool,
) -> Option<Pos<Meters>> {
    snapshot
        .actors
        .iter()
        .find(|body| body.health > 0.0 && npc_is(body, npc) && shows(body))
        .map(|body| body.at)
}
