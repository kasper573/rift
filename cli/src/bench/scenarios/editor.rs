//! The editor's scenarios, played as a designer works an area: on the area the game starts in, the
//! stage turned, panned and zoomed and the terrain sculpted; and on the area of the game's home
//! place, which may be far larger, the stage drawn back over all it takes in. Every target comes
//! from what the editor's probe reports, so a scenario names nothing of any game.

use engine::core::math::{Offset, Pos};
use engine::core::units::WindowPx;
use engine::systems::input::map::KeyModifiers;

use crate::bench::editor::EditorDriver;
use crate::bench::frames::Class;
use crate::bench::hands::{self, BUDGET, Failure};
use crate::bench::scenario::Button;

/// The opening: from the editor's first frame until the area stands in view, measured on every
/// editor the bench starts.
pub const OPENING: &str = "editor-open";

pub struct EditorScenario {
    pub name: &'static str,
    pub class: Class,
    /// Played before the frames count, to bring the scene about.
    pub setup: fn(&mut EditorDriver) -> Result<(), Failure>,
    /// Played over and over until the scenario's least time has passed, then through to its end.
    pub script: fn(&mut EditorDriver) -> Result<(), Failure>,
    /// Played after the frames counted, before the driver dismisses what is left open and undoes
    /// every edit the scenario made.
    pub teardown: fn(&mut EditorDriver) -> Result<(), Failure>,
}

/// The scenarios played on the area the game starts in, in the order they play.
pub fn catalogue() -> Vec<EditorScenario> {
    vec![
        scenario("editor-camera", Class::Typical, stage_camera),
        EditorScenario {
            setup: take_sculpt,
            teardown: take_select,
            ..scenario("editor-brush", Class::Busy, terrain_strokes)
        },
    ]
}

/// The scenarios played on the area the game's home place lies in, which may be far larger than
/// the one the game starts in.
pub fn overview_catalogue() -> Vec<EditorScenario> {
    vec![EditorScenario {
        setup: draw_back,
        ..scenario("editor-overview", Class::Typical, overview_camera)
    }]
}

fn scenario(
    name: &'static str,
    class: Class,
    script: fn(&mut EditorDriver) -> Result<(), Failure>,
) -> EditorScenario {
    EditorScenario {
        name,
        class,
        setup: nothing,
        script,
        teardown: nothing,
    }
}

fn nothing(_: &mut EditorDriver) -> Result<(), Failure> {
    Ok(())
}

/// The stage turned and panned there and back, flicked in, flicked back out as far as the
/// camera draws back, and turned quickly there.
fn stage_camera(driver: &mut EditorDriver) -> Result<(), Failure> {
    let middle = driver.stage_at_rest()?.viewport.center();
    turn(driver, middle, ORBIT_DRAG)?;
    pan(driver, middle, PAN_DRAG)?;
    pan(driver, middle, -PAN_DRAG)?;
    turn(driver, middle, -ORBIT_DRAG)?;
    let far = driver.stage_at_rest()?.distance;
    hands::move_pointer(driver, middle)?;
    hands::wheel(driver, -FLICK)?;
    let near = driver.stage_at_rest()?.distance;
    if near.0 >= far.0 {
        return Err(harness("the wheel flicked the camera no nearer"));
    }
    hands::wheel(driver, FLICK)?;
    let stage = driver.stage_at_rest()?;
    if stage.distance.0 < stage.farthest.0 {
        return Err(harness(&format!(
            "the wheel flicked the camera out only to {:.0} m of the {:.0} m it draws back to",
            stage.distance.0, stage.farthest.0
        )));
    }
    for dx in [QUICK_TURN, -QUICK_TURN, QUICK_TURN, -QUICK_TURN] {
        quick_turn(driver, middle, dx)?;
    }
    Ok(())
}

/// The stage drawn back as far as the camera draws back.
fn draw_back(driver: &mut EditorDriver) -> Result<(), Failure> {
    let middle = driver.stage_at_rest()?.viewport.center();
    hands::move_pointer(driver, middle)?;
    hands::wheel(driver, FLICK)?;
    let stage = driver.stage_at_rest()?;
    if stage.distance.0 < stage.farthest.0 {
        return Err(harness(&format!(
            "the wheel drew the camera back only to {:.0} m of the {:.0} m it draws back to",
            stage.distance.0, stage.farthest.0
        )));
    }
    Ok(())
}

/// The stage, drawn back as far as it goes, turned and panned there and back, and turned
/// quickly there.
fn overview_camera(driver: &mut EditorDriver) -> Result<(), Failure> {
    let middle = driver.stage_at_rest()?.viewport.center();
    turn(driver, middle, ORBIT_DRAG)?;
    pan(driver, middle, PAN_DRAG)?;
    pan(driver, middle, -PAN_DRAG)?;
    turn(driver, middle, -ORBIT_DRAG)?;
    for dx in [QUICK_TURN, -QUICK_TURN, QUICK_TURN, -QUICK_TURN] {
        quick_turn(driver, middle, dx)?;
    }
    Ok(())
}

/// Strokes of the sculpting brush across the stage's middle.
fn terrain_strokes(driver: &mut EditorDriver) -> Result<(), Failure> {
    let middle = driver.stage()?.viewport.center();
    for dy in STROKE_ROWS {
        let before = driver.snapshot()?.undoable;
        hands::drag(
            driver,
            Button::Left,
            &KeyModifiers::default(),
            middle + Offset::new(-STROKE, dy),
            middle + Offset::new(STROKE, dy),
        )?;
        if !driver.edited_since(before)? {
            return Err(harness("a brush stroke left the terrain as it was"));
        }
    }
    Ok(())
}

fn take_sculpt(driver: &mut EditorDriver) -> Result<(), Failure> {
    take_tool(driver, "s")
}

fn take_select(driver: &mut EditorDriver) -> Result<(), Failure> {
    take_tool(driver, "v")
}

/// Turns the stage by dragging `dx` pixels across with the right button, from `from`.
fn turn(driver: &mut EditorDriver, from: Pos<WindowPx>, dx: f32) -> Result<(), Failure> {
    let yaw = driver.stage_at_rest()?.yaw;
    hands::drag(
        driver,
        Button::Right,
        &KeyModifiers::default(),
        from,
        from + Offset::new(dx, 0.0),
    )?;
    hands::wait_for(driver, "the stage to turn", BUDGET, |snapshot| {
        snapshot
            .stage
            .as_ref()
            .is_some_and(|stage| stage.yaw.0 != yaw.0)
            .then_some(())
    })
}

/// Turns the stage as a quick flick of the hand with the right button held.
fn quick_turn(driver: &mut EditorDriver, from: Pos<WindowPx>, dx: f32) -> Result<(), Failure> {
    let yaw = driver.stage()?.yaw;
    hands::move_pointer(driver, from)?;
    hands::button_down(driver, Button::Right)?;
    let through: Vec<Pos<WindowPx>> = (1..=QUICK_STEPS)
        .map(|step| from + Offset::new(dx * step as f32 / QUICK_STEPS as f32, 0.0))
        .collect();
    hands::sweep(driver, &through)?;
    hands::button_up(driver, Button::Right)?;
    hands::wait_for(driver, "the stage to turn", BUDGET, |snapshot| {
        snapshot
            .stage
            .as_ref()
            .is_some_and(|stage| stage.yaw.0 != yaw.0)
            .then_some(())
    })
}

/// Pans the stage by dragging `by` pixels with the middle button, from `from`.
fn pan(
    driver: &mut EditorDriver,
    from: Pos<WindowPx>,
    by: Offset<WindowPx>,
) -> Result<(), Failure> {
    let target = driver.stage_at_rest()?.target;
    hands::drag(
        driver,
        Button::Middle,
        &KeyModifiers::default(),
        from,
        from + by,
    )?;
    hands::wait_for(driver, "the stage to pan", BUDGET, |snapshot| {
        snapshot
            .stage
            .as_ref()
            .is_some_and(|stage| stage.target != target)
            .then_some(())
    })
}

/// Takes the tool `key` stands for, with the pointer over the stage the key is for.
fn take_tool(driver: &mut EditorDriver, key: &str) -> Result<(), Failure> {
    let middle = driver.stage()?.viewport.center();
    hands::move_pointer(driver, middle)?;
    hands::key(driver, key).map(drop)
}

fn harness(why: &str) -> Failure {
    Failure::Harness(why.to_owned())
}

/// Pixels of drag that turn the stage, and pan it.
const ORBIT_DRAG: f32 = 320.0;
const PAN_DRAG: Offset<WindowPx> = Offset::new(220.0, 120.0);
/// Lines of the wheel, out when positive, in one quick flick: as many as take the camera from as
/// far as it draws back to as near as it comes, and back.
const FLICK: i32 = 40;
const QUICK_TURN: f32 = 480.0;
const QUICK_STEPS: u32 = 16;
const STROKE: f32 = 160.0;
const STROKE_ROWS: [f32; 3] = [-80.0, 0.0, 80.0];
