use std::time::Duration;

use driver::{WaitBudget, clickable_point, content, id, marker_at, nearest_npc};
use engine::core::math::{Pos, Rect};
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::{Action, Pose};
use engine::systems::area::{self, AreaDef, AreaModule};
use engine::systems::input::map::InputAction;
use engine::systems::npc::{NpcDef, NpcId};
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
            let listed = id::<NpcDef>("vampire_bat").get(content()).held.len();
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
        (id("turret"), marker_at(AREA, "turret")),
        |turret| turret.action == Action::Attack,
    );
    for (npc, says, shows) in BODIES {
        let ground = me.look_out(npc, |snapshot| nearest_npc(snapshot, npc).is_some());
        close_up(&mut me, says, (id(npc), ground), shows);
    }
    let (npc, ground, build) = of_a_build_of_its_own();
    close_up(
        &mut me,
        &format!(
            "Bodies of one skeleton come in builds, and a build may be given clips of its own: \
             this {} is a {build}, and moves in its build's clips rather than the first build's",
            npc.get(content()).name
        ),
        (npc, ground),
        plays_its_builds_own,
    );
}

/// An npc of the area of a build its skeleton gives clips of its own, other than the first build,
/// with where it keeps to and its build.
fn of_a_build_of_its_own() -> (NpcId, Pos<Meters>, String) {
    let content = content();
    let area = id::<AreaDef>(AREA);
    let whole = Rect::from_size(area::load(content, area).size);
    area::modules(content, area)
        .find_map(|module| {
            let (npc, ground) = match module {
                AreaModule::Population(population) => {
                    (population.npc, population.roams.unwrap_or(whole).center())
                }
                AreaModule::Resident(resident) => (resident.npc, resident.at),
                _ => return None,
            };
            let model = npc.get(content).appearance.model.get(content);
            let build = model.build.as_ref()?;
            let skeleton = model.skeleton.get(content);
            (skeleton.builds.first() != Some(build) && skeleton.build_sets.contains_key(build))
                .then(|| (npc, ground, build.to_string()))
        })
        .unwrap_or_else(|| panic!("nobody in {AREA} is of a build given clips of its own"))
}

/// Whether `body` shows what it does in a clip its build's own sets give it, where the first
/// build's would play another.
fn plays_its_builds_own(body: &ProbeBody) -> bool {
    let skeleton = body.skeleton.get(content());
    let Some(build) = body.appearance.model.get(content()).build.as_ref() else {
        return false;
    };
    let named = |build| {
        skeleton
            .named(build, body.stance, &body.shown, Pose::Idle)
            .map(|(cell, _)| cell)
    };
    named(Some(build)).is_some_and(|own| {
        named(skeleton.builds.first()) != Some(own)
            && own.clips().iter().any(|clip| *clip.clip() == body.clip)
    })
}

const AREA: &str = "island";

/// Close and low, as a walker would see it, since what a body's layers move is its feet, arms and
/// tail: heads on toward `ground` until one of the `npc`s on screen shows what `shows` asks. It
/// never walks up to the body, which would set a hostile one on the player rather than moving.
fn close_up(
    me: &mut Star,
    says: &str,
    (npc, ground): (NpcId, Pos<Meters>),
    shows: fn(&ProbeBody) -> bool,
) {
    for _ in 0..2 {
        me.tilt_camera(InputAction::TiltUp);
    }
    me.look_for(
        &format!("a {} showing it close up", npc.name(content())),
        ground,
        |snapshot| {
            showing(snapshot, npc, shows).is_some_and(|at| {
                clickable_point(snapshot, at).is_some()
                    && snapshot
                        .me
                        .as_ref()
                        .is_some_and(|me| at.distance(me.at) <= CLOSE_UP_WITHIN)
            })
        },
    );
    me.caption(says);
    me.hold();
    me.close_in();
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
const CLOSE_UP_WITHIN: Meters = Meters(5.0);

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
    npc: NpcId,
    shows: fn(&ProbeBody) -> bool,
) -> Option<Pos<Meters>> {
    snapshot
        .actors
        .iter()
        .find(|body| body.health > 0.0 && body.npc == Some(npc) && shows(body))
        .map(|body| body.at)
}
