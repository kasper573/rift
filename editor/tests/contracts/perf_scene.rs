//! A large scene stays light to work in: selecting a module or moving the pointer over the stage
//! costs a frame about the same however many nodes the scene holds, so neither copies or walks
//! the scene.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use editor::app::Editor;
use editor::project::RowSelection;
use engine::core::content::ContentFile;
use engine::systems::area::AreaDef;
use serde_json::{Value, json};
use testkit::{Game, Map, path};
use ui::egui;

use crate::support::{self, Frames, area};

#[test]
fn selecting_or_hovering_in_a_large_scene_costs_a_frame_no_more_than_in_a_small_one() {
    let small = costs(5);
    let large = costs(1850);
    for (what, small, large) in [
        ("selecting a module", small.selected, large.selected),
        (
            "moving the pointer over the stage",
            small.hovering,
            large.hovering,
        ),
    ] {
        assert!(
            large <= small + 1000,
            "{what} adds {large} allocations to a frame of a scene of {} nodes against {small} in \
             one of {}",
            nodes(1850),
            nodes(5),
        );
    }
}

/// How many more times the main thread allocates in a frame of a scene of `FOLDERS` folders of
/// `per_folder` modules each than in a frame where nothing is selected and the pointer rests: with
/// a module selected, and with the pointer moving over the stage, the most of a few frames each.
struct Costs {
    selected: usize,
    hovering: usize,
}

fn costs(per_folder: usize) -> Costs {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let game = Game::new()
        .area(support::AREA, &Map::open(SIDE, SIDE), scene(per_folder))
        .hero(support::AREA, 1.0, 1.0)
        .prop(
            "plank",
            json!({ "type": "box", "width": 0.4, "depth": 0.4 }),
        )
        .npc("wren", 10.0, json!([]))
        .file("audio/wind.wav", testkit::wav())
        .engine_rows()
        .whole();
    let mut editor = support::open(game, &scratch);
    let mut frames = Frames::new(egui::vec2(1600.0, 900.0));
    let mut frame = |editor: &mut Editor, events: Vec<egui::Event>| {
        allocations(|| {
            frames.run(events, |ui| editor::views::area::show(editor, ui, &area()));
        })
    };
    let rest = egui::pos2(1590.0, 890.0);
    let still = |editor: &mut Editor,
                 frame: &mut dyn FnMut(&mut Editor, Vec<egui::Event>) -> usize| {
        (0..4)
            .map(|_| frame(editor, vec![egui::Event::PointerMoved(rest)]))
            .skip(1)
            .min()
            .unwrap_or_default()
    };
    let resting = still(&mut editor, &mut frame);
    editor.selection = RowSelection::parts(AreaDef::TABLE, &area(), vec![format!("/{FOLDERS}")]);
    let selected = (0..4)
        .map(|_| frame(&mut editor, vec![egui::Event::PointerMoved(rest)]))
        .skip(1)
        .max()
        .unwrap_or_default();
    editor.selection = RowSelection::default();
    let resting_again = still(&mut editor, &mut frame);
    let hovering = (0..12)
        .map(|step| {
            let at = egui::pos2(640.0 + step as f32 * 30.0, 380.0 + step as f32 * 10.0);
            frame(&mut editor, vec![egui::Event::PointerMoved(at)])
        })
        .skip(1)
        .max()
        .unwrap_or_default();
    Costs {
        selected: selected.saturating_sub(resting),
        hovering: hovering.saturating_sub(resting_again),
    }
}

/// The top-level folders of every scene, each holding objects and soundscapes among its props, and
/// the first a resident; the well, an object, follows them.
const FOLDERS: usize = 20;
/// The side of the area, in metres.
const SIDE: usize = 1280;

/// How many nodes the scene of `per_folder` modules a folder holds.
fn nodes(per_folder: usize) -> usize {
    FOLDERS * (per_folder + 1) + 1
}

fn scene(per_folder: usize) -> Value {
    let place = |index: usize, off: f32| {
        json!({
            "x": 2.0 + off + (index * 7 % (SIDE - 4)) as f32,
            "y": 2.0 + off + (index * 13 % (SIDE - 4)) as f32,
        })
    };
    let at = |index: usize| place(index, 0.0);
    let between = |index: usize| place(index, 0.5);
    let module = |index: usize| match index % 50 {
        _ if index == 0 => {
            json!({ "type": "resident", "npc": path("npc", "wren"), "at": between(index) })
        }
        1 => json!({
            "type": "object",
            "name": format!("post-{index}"),
            "shape": { "type": "point", "x": between(index)["x"], "y": between(index)["y"] },
        }),
        2 => json!({
            "type": "soundscape",
            "shape": { "type": "rect", "x": at(index)["x"], "y": at(index)["y"], "w": 4.0, "h": 4.0 },
            "reach": 2.0,
            "channels": [{ "channel": 1, "file": "audio/wind.wav" }],
        }),
        _ => json!({ "type": "prop", "prop": path("prop", "plank"), "at": at(index) }),
    };
    let labelled = |index: usize| {
        let mut node = module(index);
        if index.is_multiple_of(4) {
            node["label"] = json!(format!("plank {index}"));
        }
        node
    };
    let folders = (0..FOLDERS).map(|folder| {
        json!({
            "type": "folder",
            "label": format!("quarter {folder}"),
            "children": (0..per_folder)
                .map(|index| labelled(folder * per_folder + index))
                .collect::<Vec<_>>(),
        })
    });
    let well = json!({
        "type": "object",
        "name": "well",
        "shape": { "type": "point", "x": 1.5, "y": 1.5 },
    });
    Value::Array(folders.chain([well]).collect())
}

/// How many times `run` allocates on this thread.
fn allocations(run: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    run();
    ALLOCATIONS.with(Cell::get) - before
}

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// The system's allocator, counting each thread's allocations.
struct Counting;

#[global_allocator]
static COUNTING: Counting = Counting;

// SAFETY: every call goes to the system allocator as it came; counting touches only a
// thread-local cell, which allocates nothing and is gone with its thread.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: the caller's layout, passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: a pointer this allocator gave out with this layout, passed on unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: a pointer this allocator gave out with this layout, passed on unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn count() {
    let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
}
