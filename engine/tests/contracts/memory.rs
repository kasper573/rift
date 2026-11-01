use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::script::Value;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::Seconds;
use engine::systems::memory::{self, Memory, MemoryDef, MemoryId};
use serde_json::json;
use testkit::{Declared, Game, Map, Sim};

const HOUR: u64 = 3_600_000;

#[test]
fn a_memory_reads_its_default_until_set_and_again_once_forgotten() {
    let mut sim = Sim::new(
        memories()
            .area("village", &Map::open(5, 5), json!([]))
            .hero("village", 2.0, 2.0)
            .load(),
    );
    sim.serve("village");
    let player = sim.join(1);
    let [bribed, visits] = [sim.id::<MemoryDef>("bribed"), sim.id::<MemoryDef>("visits")];

    assert_eq!(memory::get(&sim.world, player, bribed), Value::Bool(false));
    assert_eq!(memory::get(&sim.world, player, visits), Value::Int(0));
    memory::set(&mut sim.world, player, bribed, Value::Bool(true));
    memory::set(&mut sim.world, player, visits, Value::Int(3));
    sim.later(1e7);
    assert_eq!(memory::get(&sim.world, player, bribed), Value::Bool(true));
    assert_eq!(memory::get(&sim.world, player, visits), Value::Int(3));
    memory::forget(&mut sim.world, player, bribed);
    assert_eq!(memory::get(&sim.world, player, bribed), Value::Bool(false));
}

#[test]
fn a_memory_keeps_the_time_it_was_last_set() {
    let content = memories().load();
    let grudge = id(&content, "grudge");
    let start = at(100 * HOUR);
    let mut memory = Memory::default();
    memory.set(&content, grudge, Value::Bool(true), start);
    assert!(memory.set_within(&content, grudge, Seconds(1_800.0), later(start, 1_799.0)));
    assert!(!memory.set_within(&content, grudge, Seconds(1_800.0), later(start, 1_800.0)));
    assert!(memory.is_set(&content, grudge, later(start, 1_800.0)));
}

#[test]
fn daily_memories_end_at_the_reset_hour() {
    let content = memories().load();
    let looted = id(&content, "looted");
    let before_reset = at(100 * 24 * HOUR + 3 * HOUR);
    let mut memory = Memory::default();
    memory.set(&content, looted, Value::Bool(true), before_reset);
    assert!(memory.is_set(&content, looted, later(before_reset, 3_599.0)));
    assert!(!memory.is_set(&content, looted, later(before_reset, 3_600.0)));
}

#[test]
fn leaving_an_area_forgets_what_only_lasts_there() {
    let content = memories()
        .area("village", &Map::open(5, 5), json!([]))
        .area("forest", &Map::open(5, 5), json!([]))
        .hero("village", 2.0, 2.0)
        .load();
    let mut village = Sim::new(content.clone());
    village.serve("village");
    let mut forest = Sim::new(content);
    forest.serve("forest");
    let player = village.join(1);
    let [fighting, grudge] = [
        village.id::<MemoryDef>("fighting"),
        village.id::<MemoryDef>("grudge"),
    ];
    memory::set(&mut village.world, player, fighting, Value::Bool(true));
    memory::set(&mut village.world, player, grudge, Value::Bool(true));

    let player = village.cross(player, &mut forest, Pos::new(2.0, 2.0));

    assert_eq!(
        memory::get(&forest.world, player, fighting),
        Value::Bool(false)
    );
    assert_eq!(
        memory::get(&forest.world, player, grudge),
        Value::Bool(true)
    );
}

fn memories() -> Game {
    let memory = |game: Game, key: &str, ty: &str, resets: &str| {
        game.row(
            "memory",
            key,
            json!({ "label": key, "type": ty, "resets": resets }),
        )
    };
    let game = memory(Game::new(), "bribed", "boolean", "never");
    let game = memory(game, "grudge", "boolean", "never");
    let game = memory(game, "visits", "integer", "never");
    let game = memory(game, "looted", "boolean", "daily");
    memory(game, "fighting", "boolean", "leaving_area")
}

fn id(content: &Content, key: &str) -> MemoryId {
    content
        .declared::<MemoryDef>(key)
        .expect("a declared memory")
}

fn at(millis: u64) -> WallClock {
    WallClock {
        now: UnixMillis(millis),
        reset: UtcHour::try_from(4).expect("hour"),
    }
}

fn later(clock: WallClock, seconds: f32) -> WallClock {
    WallClock {
        now: clock.now.after(Seconds(seconds)),
        ..clock
    }
}
