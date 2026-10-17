// Each scene of the playthrough is set up through the driver and then only watched: the probe takes no
// snapshots meanwhile, so what's measured is the client drawing the scene, not answering the driver.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use driver::{
    Pace, Player, Playtest, PlaytestSetup, aiming, area_key, area_named, arrived_in,
    clickable_point, ground_near, hunting_ground, open, portal,
};
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::tiling::TilePos;
use engine::core::tiling::Tiles;
use engine::systems::actor::Action;
use engine::systems::input::map::InputAction;
use engine::systems::probe::{ProbeBody, ProbeSnapshot};

// Runs play out alike, so they compare.
const SEED: u64 = 1931;

const WINDOWS: [InputAction; 6] = [
    InputAction::ToggleInventory,
    InputAction::ToggleEquipment,
    InputAction::ToggleStats,
    InputAction::ToggleQuestLog,
    InputAction::ToggleSettings,
    InputAction::ToggleSkills,
];

fn main() {
    let mut playtest = Playtest::start(PlaytestSetup {
        server_binary: env!("CARGO_BIN_EXE_game-server").into(),
        client_binary: env!("CARGO_BIN_EXE_desktop-client").into(),
        content: Path::new(env!("CARGO_MANIFEST_DIR")).join("../example_game"),
        folder: Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("client-bench")
            .join(std::process::id().to_string()),
        pace: Pace::Brisk,
        seed: Some(SEED),
        recorded: false,
    });
    let mut me = playtest.join_as("pass_holder", None);
    me.in_world();
    let scenes = [
        watch(&mut me, "town", Duration::from_secs(4)),
        {
            set_off(&mut me);
            watch(&mut me, "walk", Duration::from_secs(3))
        },
        {
            toggle(&mut me, WINDOWS);
            let scene = watch(&mut me, "windows", Duration::from_secs(4));
            toggle(&mut me, WINDOWS);
            scene
        },
        {
            me.talk_to("wren");
            let scene = watch(&mut me, "conversation", Duration::from_secs(4));
            me.leave_conversation();
            scene
        },
        {
            step_into(&mut me, "forest-road");
            let scene = watch(&mut me, "crossing", Duration::from_millis(2500));
            me.wait_until("the forest", |snapshot| arrived_in(snapshot, "forest"));
            scene
        },
        watch(&mut me, "forest", Duration::from_secs(4)),
        {
            me.look_for(
                "a foe in sight",
                hunting_ground("forest", "orc"),
                |snapshot| {
                    snapshot.actors.iter().any(|actor| {
                        foe(snapshot, actor) && clickable_point(snapshot, actor.aim).is_some()
                    })
                },
            );
            me.fight(foe, |snapshot| {
                snapshot
                    .me
                    .as_ref()
                    .is_some_and(|me| me.action == Action::Attack)
            });
            watch(&mut me, "combat", Duration::from_secs(4))
        },
        {
            cast_on_nearest_foe(&mut me, InputAction::Hotbar1);
            me.act(InputAction::Hotbar2);
            cast_on_nearest_foe(&mut me, InputAction::Hotbar3);
            watch(&mut me, "skills", Duration::from_secs(4))
        },
    ];
    report(&scenes);
}

struct Scene {
    name: &'static str,
    frames: u32,
    lasted: Duration,
}

impl Scene {
    fn fps(&self) -> f64 {
        f64::from(self.frames) / self.lasted.as_secs_f64()
    }
}

fn watch(me: &mut Player, name: &'static str, lasting: Duration) -> Scene {
    let first = me.snapshot().frame;
    let since = Instant::now();
    me.linger_until(SystemTime::now() + lasting);
    let last = me.snapshot().frame;
    Scene {
        name,
        frames: last - first,
        lasted: since.elapsed(),
    }
}

fn report(scenes: &[Scene]) {
    for scene in scenes {
        println!("[client-bench] {:<12} {:>6.0} fps", scene.name, scene.fps());
    }
    let frames: u32 = scenes.iter().map(|scene| scene.frames).sum();
    let lasted: Duration = scenes.iter().map(|scene| scene.lasted).sum();
    let lowest = scenes
        .iter()
        .min_by(|a, b| a.fps().total_cmp(&b.fps()))
        .expect("scenes were watched");
    println!(
        "[client-bench] RESULT fps={:.0} lowest={:.0} ({})",
        f64::from(frames) / lasted.as_secs_f64(),
        lowest.fps(),
        lowest.name
    );
}

fn set_off(me: &mut Player) {
    let seen = me.snapshot();
    let area = area_key(&seen).expect("the player is in an area");
    let at = seen.me.as_ref().expect("the player").at;
    let far = ground_near(area, at, Tiles(5.0)..=Tiles(8.0))
        .into_iter()
        .rev()
        .find(|tile| open(&seen, *tile))
        .expect("open ground a few tiles off");
    me.point_at_tile(far);
    me.click();
}

fn toggle(me: &mut Player, windows: [InputAction; 6]) {
    for window in windows {
        me.act(window);
    }
}

fn step_into(me: &mut Player, warp: &str) {
    let gate = portal(&me.in_world(), warp).clone();
    me.travel_to(
        |_| Some(gate.at),
        |snapshot| snapshot.transition != ScreenTransitionPhase::Idle,
    );
}

// Arms the slot and clicks the nearest foe in view, as a player casting at it would; with none in
// view the cast is let go, so the scene still plays.
fn cast_on_nearest_foe(me: &mut Player, slot: InputAction) {
    me.act(slot);
    me.wait_until("the skill armed", aiming);
    let seen = me.snapshot();
    let at = seen.me.as_ref().expect("the player").at;
    let target = seen
        .actors
        .iter()
        .filter(|actor| foe(&seen, actor) && clickable_point(&seen, actor.aim).is_some())
        .min_by(|a, b| a.at.distance(at).0.total_cmp(&b.at.distance(at).0))
        .cloned();
    match target {
        Some(target) => {
            me.point_at_tile(target.at);
            me.click();
        }
        None => {
            me.act(InputAction::Dismiss);
        }
    }
}

// Bats flying out over the water are out of reach.
fn foe(snapshot: &ProbeSnapshot, actor: &ProbeBody) -> bool {
    let (Some(me), Some(area)) = (snapshot.me.as_ref(), area_key(snapshot)) else {
        return false;
    };
    let grid = &area_named(area).grid;
    !actor.player
        && !actor.friendly
        && actor.health > 0.0
        && grid.walkable(actor.at)
        && grid.component(actor.at) == grid.component(me.at)
}
