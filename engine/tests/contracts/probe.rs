use engine::core::content::{Content, ContentScope, with_content};
use engine::core::math::Pos;
use engine::core::render::sample::{GpuReport, RenderReport};
use engine::core::units::{Bytes, Meters, Millis};
use engine::systems::area::AreaDef;
use engine::systems::probe::{
    FrameSample, FrameSamples, ProbeFrames, ProbeGame, ProbeGameQuestion, WORST_KEPT, answer,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use testkit::{Game, Map, path};

#[test]
fn a_driver_finds_the_games_things_through_the_probe() {
    let content = Game::new()
        .area(
            "yard",
            &Map::of(&[
                "..........",
                "..........",
                "....##....",
                "....##....",
                "..........",
                "..........",
            ]),
            json!([
                { "type": "resident", "npc": path("npc", "keeper"), "at": { "x": 1.5, "y": 1.5 } },
            ]),
        )
        .hero("yard", 8.0, 1.0)
        .sword()
        .npc("keeper", 40.0, json!([]))
        .engine_rows()
        .load();
    let area = content
        .by_path::<AreaDef>(&path("area", "yard"))
        .expect("the yard");

    let game: ProbeGame = ask(&content, &ProbeGameQuestion::Game);
    assert_eq!(game.initial_spawn.at, Pos::new(8.0, 1.0));
    let yard = game
        .areas
        .iter()
        .find(|shown| shown.area == area)
        .expect("the yard");
    let keeper = &yard.residents[0];
    assert_eq!(keeper.npc.path(&content), path("npc", "keeper"));
    assert_eq!(keeper.at, Pos::new(1.5, 1.5));
    let npc = game
        .npcs
        .iter()
        .find(|npc| npc.npc == keeper.npc)
        .expect("the keeper among the npcs");
    assert_eq!(npc.name, "keeper");
    let sword = game
        .items
        .iter()
        .find(|item| item.item.path(&content) == path("item", "sword"))
        .expect("the sword among the items");
    assert_eq!(
        sword.stance.map(|stance| stance.path(&content).to_owned()),
        Some(path("stance", "sword"))
    );
    assert!(game.skills.iter().any(|skill| skill.name == "Strike"));
    assert!(game.spacing > Meters(0.0));

    let walkable: Vec<bool> = ask(
        &content,
        &ProbeGameQuestion::Batch {
            questions: [Pos::new(1.5, 1.5), Pos::new(4.5, 2.5)]
                .map(|at| ProbeGameQuestion::Walkable { area, at })
                .to_vec(),
        },
    );
    assert_eq!(walkable, [true, false]);
    let route: Option<Vec<Pos<Meters>>> = ask(
        &content,
        &ProbeGameQuestion::Route {
            area,
            from: Pos::new(2.0, 3.0),
            to: Pos::new(8.0, 3.0),
        },
    );
    let corners = route.expect("a way around the wall");
    assert_eq!(corners.last(), Some(&Pos::new(8.0, 3.0)));
    assert!(corners.len() > 1, "the way bends around the wall");
}

fn ask<Answer: DeserializeOwned>(content: &Content, question: &ProbeGameQuestion) -> Answer {
    let line = answer(content, question);
    with_content(ContentScope::of(content), || serde_json::from_str(&line))
        .0
        .unwrap_or_else(|error| panic!("an unreadable answer {line}: {error}"))
}

#[test]
fn a_driver_reads_the_frames_on_from_the_last_it_saw_and_is_told_what_the_ring_let_go() {
    let mut frames = FrameSamples::new(4, 0);
    for frame in 1..=6 {
        frames.push(sample(frame, Millis(10.0)));
    }
    let read = frames.since(0);
    assert_eq!(seen(&read), [3, 4, 5, 6]);
    assert_eq!(read.dropped, 2);
    let read = frames.since(4);
    assert_eq!(seen(&read), [5, 6]);
    assert_eq!(read.dropped, 0);
    assert!(frames.since(6).samples.is_empty());

    frames.drawn(&RenderReport {
        frame: 5,
        cpu_render_ms: Millis(3.0),
        draws: 7,
        instances: 70,
        upload_bytes: Bytes(1024),
    });
    frames.timed(&GpuReport {
        frame: 5,
        gpu_ms: Millis(4.0),
        passes: vec![("main_opaque_pass_3d".to_owned(), Millis(4.0))],
    });
    let read = frames.since(4);
    assert_eq!(read.samples[0].cpu_render_ms, Some(Millis(3.0)));
    assert_eq!(read.samples[0].draws, 7);
    assert_eq!(read.samples[0].upload_bytes, Bytes(1024));
    assert_eq!(read.samples[0].gpu_ms, Some(Millis(4.0)));
    assert_eq!(read.samples[1].cpu_render_ms, None);
    assert_eq!(read.samples[1].gpu_ms, None);
}

#[test]
fn a_frame_is_read_once_what_trails_it_has_had_time_to_land() {
    let mut frames = FrameSamples::new(8, 2);
    for frame in 1..=5 {
        frames.push(sample(frame, Millis(10.0)));
    }
    assert_eq!(seen(&frames.since(0)), [1, 2, 3]);
}

#[test]
fn the_slowest_frames_keep_the_gpus_time_by_pass() {
    let mut frames = FrameSamples::new(64, 0);
    for frame in 1..=40 {
        let dt = if frame == 17 { 60.0 } else { 10.0 };
        frames.push(sample(frame, Millis(dt)));
    }
    frames.timed(&GpuReport {
        frame: 17,
        gpu_ms: Millis(50.0),
        passes: vec![("shadows".to_owned(), Millis(50.0))],
    });
    let read = frames.since(0);
    assert_eq!(read.worst.len(), WORST_KEPT);
    assert_eq!(read.worst[0].frame, 17);
    assert_eq!(read.worst[0].passes[0].name, "shadows");
    assert_eq!(read.worst[0].passes[0].gpu_ms, Millis(50.0));
}

fn sample(frame: u32, dt_ms: Millis) -> FrameSample {
    FrameSample {
        frame,
        dt_ms,
        cpu_main_ms: Millis(1.0),
        cpu_render_ms: None,
        gpu_ms: None,
        missing: 0,
        draws: 0,
        instances: 0,
        entities: 0,
        visible_chunks: 0,
        upload_bytes: Bytes(0),
    }
}

fn seen(frames: &ProbeFrames) -> Vec<u32> {
    frames.samples.iter().map(|sample| sample.frame).collect()
}
