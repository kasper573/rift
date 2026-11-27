use editor::views::area::brush::{Brush, PaintMask, SculptMode, Stroke, StrokeKind, commit};
use editor::views::area::water::{Flood, FloodRefused, OpenSides};
use editor::views::area::{AreaSnapshot, contours, nodes, place, scatter};
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::{Degrees, Meters};
use engine::systems::area::{AreaDef, AreaModule, TerrainDef};
use engine::systems::water::WaterDef;
use serde_json::json;
use testkit::{Game, HEIGHTS, Map, POND, path};

use crate::support::{self, area};

const STONE: &str = "stone";

fn game_with_a_stone() -> Game {
    support::game().prop(STONE, json!({ "type": "box", "width": 1.0, "depth": 1.0 }))
}

fn terrain(editor: &editor::app::Editor) -> AreaDef {
    editor
        .read::<AreaDef>(&area())
        .expect("the area is in the project")
        .expect("the area reads")
        .as_ref()
        .clone()
}

#[test]
fn a_sculpt_stroke_lands_in_the_height_map() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game(), &scratch);
    let at = Pos::new(4.0, 4.0);
    let before = terrain(&editor);
    let mut stroke =
        Stroke::begin(&before, at, StrokeKind::Sculpt(SculptMode::Raise)).expect("a sculpt stroke");
    stroke.apply(&Brush::default(), at, 0.2, Meters(0.0));
    assert!(commit(&mut editor, &area(), &stroke));
    let written = editor.project.as_ref().expect("a project").files.unsaved();
    assert!(
        !written.is_empty() && written.iter().all(|file| file.ends_with(".png")),
        "a stroke writes only the maps of the ground it shapes, never the scene: {written:?}"
    );
    support::settled(&mut editor);
    let raised = terrain(&editor);
    assert!(
        raised.heights.height_at(at) > before.heights.height_at(at) + Meters(0.1),
        "the ground under the brush rose"
    );
    assert_eq!(
        raised.heights.height_at(Pos::new(0.5, 0.5)),
        before.heights.height_at(Pos::new(0.5, 0.5)),
        "the ground beyond the brush keeps its height"
    );

    let mut lowered =
        Stroke::begin(&raised, at, StrokeKind::Sculpt(SculptMode::Level)).expect("a sculpt stroke");
    lowered.apply(&Brush::default(), at, 60.0, Meters(0.0));
    assert!(commit(&mut editor, &area(), &lowered));
    support::settled(&mut editor);
    assert!(
        terrain(&editor).heights.height_at(at).0.abs() < 0.05,
        "a long level stroke pulls the ground back to the level"
    );
    editor.undo();
    assert!(
        terrain(&editor).heights.height_at(at) > Meters(0.1),
        "undoing one stroke brings the hill back"
    );
}

#[test]
fn a_paint_stroke_lands_in_the_paint_map_on_the_ground_its_mask_lets_it() {
    let pond = WaterSurface::from_fn(
        GroundSize {
            width: 8,
            height: 8,
        },
        HEIGHTS,
        |column, _| Meters(if column >= 8 { 0.5 } else { 0.0 }),
    );
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game().water(support::AREA, &pond), &scratch);
    let before = terrain(&editor);
    let rock = editor
        .reader()
        .expect("a reader")
        .id_of::<TerrainDef>(&path("terrain", testkit::ROCK))
        .expect("rock is a terrain");
    let (middle, wet, dry, beyond) = (
        Pos::new(4.0, 4.0),
        Pos::new(6.0, 4.0),
        Pos::new(2.0, 4.0),
        Pos::new(0.5, 0.5),
    );
    let wide = Brush {
        falloff: 0.0,
        ..Brush::default()
    };
    for (mask, painted, kept) in [
        (PaintMask::Anywhere, vec![wet, dry], vec![beyond]),
        (PaintMask::UnderWater, vec![wet], vec![dry, beyond]),
        (PaintMask::DryGround, vec![dry], vec![wet, beyond]),
    ] {
        let masked = StrokeKind::Paint {
            terrain: rock,
            mask,
        };
        let mut stroke = Stroke::begin(&before, middle, masked).expect("room for a terrain");
        stroke.apply(&wide, middle, 0.1, Meters(0.0));
        assert!(commit(&mut editor, &area(), &stroke));
        support::settled(&mut editor);
        let after = terrain(&editor);
        for at in painted {
            assert_eq!(
                after.terrains[usize::from(after.paint.at(at))],
                rock,
                "{mask:?}: rock lies at {at:?}, listed among the area's terrains"
            );
        }
        for at in kept {
            assert_eq!(
                after.paint.at(at),
                before.paint.at(at),
                "{mask:?}: the ground at {at:?} the brush stays off keeps its terrain"
            );
        }
        editor.undo();
        support::settled(&mut editor);
    }
}

#[test]
fn a_flood_fills_a_basin_to_its_rim_and_is_undone_as_one_step() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game().pond(), &scratch);
    let (middle, beyond) = (Pos::new(4.0, 4.0), Pos::new(0.5, 0.5));
    let flat = terrain(&editor);
    let mut dug =
        Stroke::begin(&flat, middle, StrokeKind::Sculpt(SculptMode::Lower)).expect("a stroke");
    let small = Brush {
        radius: Meters(2.0),
        ..Brush::default()
    };
    dug.apply(&small, middle, 0.1, Meters(0.0));
    assert!(commit(&mut editor, &area(), &dug));
    support::settled(&mut editor);
    let basin = terrain(&editor);
    let pond = editor
        .reader()
        .expect("a reader")
        .id_of::<WaterDef>(&path("water", POND))
        .expect("the pond is a water");
    let level = Meters(-0.1);
    assert_eq!(
        Flood::begin(&basin, beyond, pond, level).err(),
        Some(FloodRefused::AboveTheLevel {
            ground: Meters(0.0)
        }),
        "ground that stands above the level has nothing to flood"
    );
    let mut flood = Flood::begin(&basin, middle, pond, level).expect("a basin to flood");
    assert!(flood.spread(|| true), "a flood left to spread stands whole");
    assert!(
        flood.flooded > 1.0 && flood.open == OpenSides::default(),
        "the flood fills the basin and stops at its rim, short of the area's edges"
    );
    assert!(commit(&mut editor, &area(), &flood.stroke));
    support::settled(&mut editor);
    let flooded = terrain(&editor);
    assert!(
        flooded.water.depth_at(&flooded.heights, middle) > Meters(0.1),
        "water stands in the basin"
    );
    assert_eq!(
        flooded.water.depth_at(&flooded.heights, beyond),
        Meters(0.0),
        "the ground beyond the rim stays dry"
    );
    assert_eq!(flooded.waters, [pond], "the area names the water laid");
    assert!(
        Flood::begin(&flooded, middle, pond, level).is_err(),
        "water already standing at the level takes no more"
    );
    editor.undo();
    let undone = terrain(&editor);
    assert_eq!(
        undone.water.depth_at(&undone.heights, middle),
        Meters(0.0),
        "undoing the flood leaves the basin dry"
    );
    assert!(undone.waters.is_empty(), "and the area names no water");
}

#[test]
fn placing_and_scattering_a_prop_land_as_modules() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(game_with_a_stone(), &scratch);
    let stone = path("prop", STONE);
    let snapshot = AreaSnapshot::for_bundle(&mut editor, &area()).expect("the area opens");
    place::place_prop(
        &mut editor,
        &snapshot,
        &stone,
        Pos::new(3.0, 3.5),
        Degrees(45.0),
    );
    let region = Rect::new(Pos::new(4.0, 4.0), Size::new(3.0, 2.0));
    scatter::scatter(&mut editor, &snapshot, &stone, region);
    editor.finish_busy();
    let modules = nodes::modules(&support::scene(&editor, &area()));
    let placed: Vec<&AreaModule> = modules.iter().map(|node| node.module()).collect();
    assert!(placed.iter().any(|module| matches!(
        module,
        AreaModule::Prop(prop) if prop.at == Pos::new(3.0, 3.5) && prop.yaw == Degrees(45.0)
    )));
    assert!(placed.iter().any(|module| matches!(
        module,
        AreaModule::Scatter(scatter) if scatter.region == region
    )));
}

#[test]
fn contours_follow_the_whole_metres_of_a_hill() {
    let scratch = tempfile::tempdir().expect("scratch");
    let editor = support::open(
        Game::new()
            .area(support::AREA, &Map::open(16, 16), json!([]))
            .hero(support::AREA, 1.0, 1.0)
            .engine_rows()
            .whole(),
        &scratch,
    );
    let flat = terrain(&editor);
    assert!(contours::trace(&flat.heights).is_empty());
    let at = Pos::new(8.0, 8.0);
    let mut stroke =
        Stroke::begin(&flat, at, StrokeKind::Sculpt(SculptMode::Raise)).expect("a sculpt stroke");
    stroke.apply(&Brush::default(), at, 8.0, Meters(0.0));
    let traced = contours::trace(&stroke.heights);
    let heights: Vec<f32> = traced.iter().map(|contour| contour.level.0).collect();
    assert!(
        heights.contains(&1.0),
        "a hill crosses the metre: {heights:?}"
    );
    assert!(
        traced.iter().all(|contour| !contour.segments.is_empty()),
        "every contour has a line"
    );
}
