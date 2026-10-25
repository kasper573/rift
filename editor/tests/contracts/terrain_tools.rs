use editor::views::area::brush::{Brush, SculptMode, Stroke, StrokeKind, commit};
use editor::views::area::{AreaSnapshot, contours, nodes, place, scatter};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Size};
use engine::core::units::Meters;
use engine::systems::area::{AreaDef, AreaModule};
use serde_json::json;
use testkit::{Game, Map, path};

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
    let mut stroke = Stroke::begin(&before, at, StrokeKind::Sculpt(SculptMode::Raise));
    stroke.apply(&Brush::default(), at, 0.2, Meters(0.0));
    assert!(commit(&mut editor, &area(), &stroke));
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

    let mut lowered = Stroke::begin(&raised, at, StrokeKind::Sculpt(SculptMode::Level));
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
fn a_paint_stroke_lands_in_the_paint_map() {
    let scratch = tempfile::tempdir().expect("scratch");
    let mut editor = support::open(support::game(), &scratch);
    let at = Pos::new(4.0, 4.0);
    let before = terrain(&editor);
    let mut stroke = Stroke::begin(&before, at, StrokeKind::Paint { slot: 1 });
    stroke.apply(&Brush::default(), at, 4.0, Meters(0.0));
    assert!(commit(&mut editor, &area(), &stroke));
    support::settled(&mut editor);
    let painted = terrain(&editor);
    let (column, row) = painted.paint.pixel_at(at);
    let weights = painted.paint.weights(column, row);
    assert!(
        weights[1] > weights[0],
        "the painted material leads under the brush: {weights:?}"
    );
    let (column, row) = painted.paint.pixel_at(Pos::new(0.5, 0.5));
    assert_eq!(
        painted.paint.weights(column, row),
        before.paint.weights(column, row),
        "the paint beyond the brush keeps its weights"
    );
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
    let modules = nodes::modules(&nodes::scene_of(&editor, &area()));
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
fn the_brush_grows_and_shrinks_within_its_range() {
    let mut brush = Brush::default();
    for _ in 0..40 {
        brush.grow();
    }
    assert_eq!(brush.radius, editor::views::area::brush::LARGEST_RADIUS);
    for _ in 0..40 {
        brush.shrink();
    }
    assert_eq!(brush.radius, editor::views::area::brush::SMALLEST_RADIUS);
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
    let mut stroke = Stroke::begin(&flat, at, StrokeKind::Sculpt(SculptMode::Raise));
    stroke.apply(&Brush::default(), at, 8.0, Meters(0.0));
    let traced = contours::trace(&stroke.heights);
    let heights: Vec<f32> = traced.iter().map(|contour| contour.height.0).collect();
    assert!(
        heights.contains(&1.0),
        "a hill crosses the metre: {heights:?}"
    );
    assert!(
        traced.iter().all(|contour| !contour.segments.is_empty()),
        "every contour has a line"
    );
}
