use actor::{ActorModel, Direction, SfxId, Timing};
use image::{Image, encode_png};
use math::{Size, Tiles};

const MANIFEST: &str = r#"{
    "frame": { "w": 2, "h": 2 },
    "hitbox": [1, 2],
    "actions": {
        "idle": { "dir": { "s": { "file": "sheet.png", "y": 2 } } },
        "attack": {
            "base": { "file": "east.png", "sfx": [{"id":"swing","frame":1}] },
            "dir": {
                "e": { "cues": {"apex": [1]} },
                "w": { "flip": true }
            }
        }
    }
}"#;

fn pixel(x: u32, y: u32) -> [u8; 4] {
    [(10 + x * 10) as u8, (10 + y * 10) as u8, 200, 255]
}

fn fixture(width: u32, height: u32) -> &'static [u8] {
    let mut image = Image::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let at = ((y * width + x) * 4) as usize;
            image.rgba[at..at + 4].copy_from_slice(&pixel(x, y));
        }
    }
    Box::leak(encode_png(&image).into_boxed_slice())
}

fn model() -> ActorModel {
    let sheet = fixture(4, 4);
    let east = fixture(4, 2);
    ActorModel::load("fixture", MANIFEST, |file| match file {
        "sheet.png" => Some(sheet),
        "east.png" => Some(east),
        _ => None,
    })
}

#[test]
fn hitbox_comes_from_the_manifest() {
    assert_eq!(model().hitbox(), Size::new(Tiles(1.0), Tiles(2.0)));
}

#[test]
fn strips_address_rows_inside_files() {
    let model = model();
    let idle = model.frame("idle", Direction::S as u8, 0.0, 1.0);
    assert_eq!((idle.size.x.0, idle.size.y.0), (2.0, 2.0));
    for dy in 0..2 {
        for dx in 0..2 {
            // idle was declared at y=2 in sheet.png, so its pixels are the sheet's bottom row.
            assert_eq!(
                model.image().pixel(
                    idle.pos.x.0 as usize + dx as usize,
                    idle.pos.y.0 as usize + dy as usize
                ),
                pixel(dx, 2 + dy),
            );
        }
    }
}

#[test]
fn flip_mirrors_each_frame() {
    let model = model();
    let east = model.frame("attack", Direction::E as u8, 0.0, 1.0);
    let west = model.frame("attack", Direction::W as u8, 0.0, 1.0);
    assert_ne!(east, west);
    for dy in 0..2 {
        for dx in 0..2 {
            assert_eq!(
                model.image().pixel(
                    east.pos.x.0 as usize + dx as usize,
                    east.pos.y.0 as usize + dy as usize
                ),
                pixel(dx, dy),
            );
            assert_eq!(
                model.image().pixel(
                    west.pos.x.0 as usize + dx as usize,
                    west.pos.y.0 as usize + dy as usize
                ),
                pixel(1 - dx, dy),
            );
        }
    }
}

#[test]
fn frames_advance_at_100ms() {
    let model = model();
    let first = model.frame("idle", Direction::S as u8, 0.0, 1.0);
    let second = model.frame("idle", Direction::S as u8, 0.15, 1.0);
    assert_ne!(first, second);
    assert_eq!(first, model.frame("idle", Direction::S as u8, 0.25, 1.0));
}

#[test]
fn missing_actions_fall_back_to_idle() {
    let model = model();
    assert_eq!(
        model.frame("walk", Direction::S as u8, 0.0, 1.0),
        model.frame("idle", Direction::S as u8, 0.0, 1.0),
    );
}

#[test]
fn death_plays_once_and_holds_the_last_frame() {
    let model = model();
    let first = model.frame("death", Direction::S as u8, 0.0, 1.0);
    let last = model.frame("death", Direction::S as u8, 0.15, 1.0);
    assert_ne!(first, last);
    assert_eq!(last, model.frame("death", Direction::S as u8, 5.0, 1.0));
}

#[test]
fn timing_reflects_frames_and_apex() {
    let model = model();
    assert_eq!(
        model.timing("attack", Direction::E as u8),
        Timing {
            duration: 0.2,
            apex: 0.1,
        },
    );
    // The west strip declares no apex, so it defaults to the run's start.
    assert_eq!(
        model.timing("attack", Direction::W as u8),
        Timing {
            duration: 0.2,
            apex: 0.0,
        },
    );
    // Undeclared actions fall back to idle, like frame().
    assert_eq!(
        model.timing("walk", Direction::S as u8),
        model.timing("idle", Direction::S as u8),
    );
}

#[test]
#[should_panic(expected = "cue 'apex'")]
fn an_out_of_range_cue_frame_fails_the_load() {
    let east = fixture(4, 2);
    ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 2],
            "actions": { "idle": { "dir": { "e": { "file": "east.png", "cues": {"apex": [2]} } } } }
        }"#,
        |_| Some(east),
    );
}

#[test]
fn absent_directions_resolve_to_the_nearest_declared() {
    let model = model();
    // attack declares only e and w; north is a quarter turn from w but
    // three quarters from e going clockwise from east.
    assert_eq!(
        model.frame("attack", Direction::N as u8, 0.0, 1.0),
        model.frame("attack", Direction::W as u8, 0.0, 1.0),
    );
    assert_eq!(
        model.frame("attack", Direction::SE as u8, 0.0, 1.0),
        model.frame("attack", Direction::E as u8, 0.0, 1.0),
    );
}

fn swing() -> SfxId {
    SfxId("swing".to_owned())
}

#[test]
fn sfx_ids_lists_referenced_cues() {
    // attack's base sfx reaches both its declared directions, so the id surfaces once per direction.
    let model = model();
    assert!(model.sfx_ids().next().is_some());
    assert!(model.sfx_ids().all(|id| *id == swing()));
}

#[test]
fn an_action_without_cues_is_silent() {
    assert!(
        model()
            .sfx("idle", Direction::S as u8, -1.0, 1.0, 1.0)
            .is_empty()
    );
}

#[test]
fn a_cue_fires_as_its_frame_is_entered() {
    let model = model();
    let e = Direction::E as u8;
    // swing sits on frame 1 (100ms in): entering frame 0 at the start is silent, frame 1 fires it.
    assert!(model.sfx("attack", e, -1.0, 0.0, 1.0).is_empty());
    assert_eq!(model.sfx("attack", e, 0.0, 0.15, 1.0), vec![&swing()]);
}

#[test]
fn a_cue_does_not_refire_within_its_frame() {
    let model = model();
    let e = Direction::E as u8;
    assert_eq!(model.sfx("attack", e, 0.0, 0.15, 1.0), vec![&swing()]);
    assert!(model.sfx("attack", e, 0.15, 0.18, 1.0).is_empty());
}

#[test]
fn a_looping_action_refires_its_cue_each_cycle() {
    // attack has two frames (a 200ms loop), so the frame-1 cue recurs 0.3s in.
    let model = model();
    assert_eq!(
        model.sfx("attack", Direction::E as u8, 0.25, 0.35, 1.0),
        vec![&swing()]
    );
}

#[test]
fn attack_speed_scales_a_cues_timing() {
    let model = model();
    let e = Direction::E as u8;
    // doubled attack speed halves the frame time, so the frame-1 cue arrives at 0.05s, not 0.1s.
    assert!(model.sfx("attack", e, 0.0, 0.04, 2.0).is_empty());
    assert_eq!(model.sfx("attack", e, 0.04, 0.06, 2.0), vec![&swing()]);
}

#[test]
fn a_long_pause_fires_a_cue_at_most_once() {
    // A lag spike spanning hundreds of loops still sounds the cue a single time.
    let model = model();
    assert_eq!(
        model.sfx("attack", Direction::E as u8, -1.0, 100.0, 1.0),
        vec![&swing()]
    );
}

#[test]
fn a_death_cue_fires_once_and_never_repeats() {
    let sheet = fixture(8, 2);
    let model = ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 1],
            "actions": {
                "idle": { "dir": { "s": { "file": "sheet.png", "frames": 1 } } },
                "death": { "base": { "file": "sheet.png", "sfx": [{"id":"thud","frame":2}] }, "dir": { "s": { "frames": 4 } } }
            }
        }"#,
        |_| Some(sheet),
    );
    let s = Direction::S as u8;
    let thud = SfxId("thud".to_owned());
    assert!(model.sfx("death", s, -1.0, 0.0, 1.0).is_empty());
    assert_eq!(model.sfx("death", s, 0.0, 0.25, 1.0), vec![&thud]);
    // Holding the last frame past the cue never sounds it again.
    assert!(model.sfx("death", s, 0.25, 5.0, 1.0).is_empty());
}

#[test]
#[should_panic(expected = "sfx frame")]
fn an_out_of_range_sfx_frame_fails_the_load() {
    let east = fixture(4, 2);
    ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 1],
            "actions": { "idle": { "dir": { "e": { "file": "east.png", "sfx": [{"id":"x","frame":5}] } } } }
        }"#,
        |_| Some(east),
    );
}

fn walker() -> ActorModel {
    let sheet = fixture(8, 2);
    ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 1],
            "actions": {
                "idle": { "dir": { "s": { "file": "sheet.png", "frames": 1 } } },
                "walk": { "dir": { "s": { "file": "sheet.png", "frames": 4, "cues": { "steps": [2] } } } }
            }
        }"#,
        |_| Some(sheet),
    )
}

#[test]
fn a_named_cue_fires_as_its_frame_is_entered() {
    let walk = walker();
    let s = Direction::S as u8;
    // steps sits on frame 2: entering frame 0 at the start is silent, crossing into frame 2 fires it.
    assert!(!walk.cue_crossed("walk", s, "steps", -1.0, 0.0, 1.0));
    assert!(walk.cue_crossed("walk", s, "steps", 0.0, 0.25, 1.0));
}

#[test]
fn a_named_cue_refires_each_loop() {
    // walk is a 4-frame (400ms) loop, so the frame-2 step recurs 0.6s in.
    assert!(walker().cue_crossed("walk", Direction::S as u8, "steps", 0.55, 0.65, 1.0));
}

#[test]
fn a_cue_fires_at_each_frame_in_its_list() {
    let sheet = fixture(8, 2);
    let walk = ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 1],
            "actions": {
                "idle": { "dir": { "s": { "file": "sheet.png", "frames": 1 } } },
                "walk": { "dir": { "s": { "file": "sheet.png", "frames": 4, "cues": { "steps": [0, 2] } } } }
            }
        }"#,
        |_| Some(sheet),
    );
    let s = Direction::S as u8;
    // steps [0, 2]: frame 0 fires at the start, frame 2 mid-cycle, the frame between fires nothing.
    assert!(walk.cue_crossed("walk", s, "steps", -1.0, 0.05, 1.0));
    assert!(!walk.cue_crossed("walk", s, "steps", 0.05, 0.15, 1.0));
    assert!(walk.cue_crossed("walk", s, "steps", 0.15, 0.25, 1.0));
}

#[test]
fn base_fields_fill_in_under_dir_overrides() {
    let sheet = fixture(4, 4);
    let model = ActorModel::load(
        "fixture",
        r#"{
            "frame": { "w": 2, "h": 2 },
            "hitbox": [1, 1],
            "actions": {
                "idle": {
                    "base": { "file": "sheet.png", "y": 2 },
                    "dir": { "s": {}, "n": { "y": 0 } }
                }
            }
        }"#,
        |_| Some(sheet),
    );
    // s inherits base's file and y=2 (sheet's bottom row); n overrides y to 0 (the top row).
    let s = model.frame("idle", Direction::S as u8, 0.0, 1.0);
    let n = model.frame("idle", Direction::N as u8, 0.0, 1.0);
    assert_eq!(
        model.image().pixel(s.pos.x.0 as usize, s.pos.y.0 as usize),
        pixel(0, 2)
    );
    assert_eq!(
        model.image().pixel(n.pos.x.0 as usize, n.pos.y.0 as usize),
        pixel(0, 0)
    );
}

#[test]
fn an_absent_named_cue_never_fires() {
    let walk = walker();
    let s = Direction::S as u8;
    // walk declares no "apex" cue, and idle declares no cues at all.
    assert!(!walk.cue_crossed("walk", s, "apex", 0.0, 0.5, 1.0));
    assert!(!walk.cue_crossed("idle", s, "steps", -1.0, 0.5, 1.0));
}
