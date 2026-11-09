//! The editor's scenarios, played on the area the game starts in as a designer works it: the scene
//! tree hovered, opened, selected, scrolled and dragged through; the stage turned, panned, zoomed,
//! hovered, clicked, dragged on and sculpted; the pages and tabs switched; edits undone and redone,
//! fields typed into, folders hidden and shown, and the window's panels resized; and on the area of
//! the game's home place, the stage drawn back over all it takes in. Every target comes from what
//! the editor's probe reports, so a scenario names nothing of any game.

use std::time::Instant;

use engine::core::math::{Offset, Pos, Rect};
use engine::core::units::WindowPx;
use engine::systems::input::map::KeyModifiers;
use engine::systems::probe::{EditorSnapshot, EditorStage, EditorTree, EditorTreeRow};

use crate::bench::editor::{CTRL, EditorDriver};
use crate::bench::frames::Class;
use crate::bench::hands::{self, BUDGET, Failure};
use crate::bench::scenario::Button;

/// The opening: from the editor's first frame until the area stands in view, measured on every
/// editor the bench starts.
pub const OPENING: &str = "e1-open";

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

/// Every scenario but the opening, in the order they play.
pub fn catalogue() -> Vec<EditorScenario> {
    vec![
        scenario("e2-tree-hover", Class::Typical, tree_hover),
        scenario("e3-tree-folders", Class::Typical, tree_folders),
        scenario("e4-tree-select", Class::Typical, tree_select),
        scenario("e5-tree-menu", Class::Typical, tree_menu),
        scenario("e6-tree-drag", Class::Typical, tree_drag),
        scenario("e7-tree-scroll", Class::Typical, tree_scroll),
        scenario("e8-stage-camera", Class::Typical, stage_camera),
        scenario("e9-stage-hover", Class::Typical, stage_hover),
        scenario("e10-stage-select", Class::Typical, stage_select),
        scenario("e11-stage-move", Class::Typical, stage_move),
        scenario("e12-stage-box", Class::Typical, stage_box),
        scenario("e13-stage-menu", Class::Typical, stage_menu),
        EditorScenario {
            setup: take_sculpt,
            teardown: take_select,
            ..scenario("e14-terrain-brush", Class::Busy, terrain_strokes)
        },
        EditorScenario {
            setup: open_file_tab,
            teardown: close_file_tab,
            ..scenario("e15-pages-tabs", Class::Typical, pages_and_tabs)
        },
        EditorScenario {
            setup: make_edits,
            ..scenario("e16-undo-redo", Class::Busy, undo_redo)
        },
        EditorScenario {
            setup: select_middle,
            ..scenario("e17-field-typing", Class::Typical, field_typing)
        },
        scenario("e18-visibility", Class::Typical, visibility_toggles),
        EditorScenario {
            setup: open_file_tab,
            teardown: close_file_tab,
            ..scenario("e19-panels-dock", Class::Typical, panels_and_dock)
        },
        scenario("e20-hover-orbit", Class::Typical, hover_while_orbiting),
    ]
}

/// The scenarios played on the area the game's home place lies in, which may be far larger than the
/// one the game starts in, with the stage drawn back over as much of it as the camera takes in.
pub fn overview_catalogue() -> Vec<EditorScenario> {
    vec![EditorScenario {
        setup: draw_back,
        ..scenario("e21-stage-overview", Class::Typical, overview_camera)
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

/// E2: the pointer down over every row of the scene tree in view, and back up.
fn tree_hover(driver: &mut EditorDriver) -> Result<(), Failure> {
    let rows = shown_rows(&driver.tree(SCENE)?);
    if rows.is_empty() {
        return Err(harness("the scene tree shows no rows"));
    }
    for row in rows.iter().chain(rows.iter().rev()) {
        hands::move_pointer(driver, row_point(row))?;
    }
    Ok(())
}

/// E3: the folders of the scene tree in view closed and opened again, or opened and closed, by a
/// click on each row.
fn tree_folders(driver: &mut EditorDriver) -> Result<(), Failure> {
    let mut folders: Vec<EditorTreeRow> = shown_rows(&driver.tree(SCENE)?)
        .into_iter()
        .filter(|row| row.folder)
        .collect();
    // An open folder shows what it holds, so closing it changes the most.
    folders.sort_by_key(|row| !row.open);
    folders.truncate(FOLDERS);
    if folders.is_empty() {
        return Err(harness("the scene tree shows no folder"));
    }
    for folder in folders {
        for open in [!folder.open, folder.open] {
            let row = driver
                .tree(SCENE)?
                .rows
                .into_iter()
                .find(|row| row.id == folder.id)
                .ok_or_else(|| harness("a folder went from the scene tree"))?;
            click_row(driver, &row)?;
            driver.wait_for_tree(SCENE, "the folder to open or close", |tree| {
                tree.rows
                    .iter()
                    .any(|row| row.id == folder.id && row.open == open)
                    .then_some(())
            })?;
        }
    }
    Ok(())
}

/// E4: a row of the scene tree clicked, then the selection walked down and back up by the keys.
fn tree_select(driver: &mut EditorDriver) -> Result<(), Failure> {
    let rows = shown_rows(&driver.tree(SCENE)?);
    let first = rows
        .first()
        .ok_or_else(|| harness("the scene tree shows no rows"))?;
    click_row(driver, first)?;
    for key in ["Down"; KEY_STEPS].into_iter().chain(["Up"; KEY_STEPS]) {
        let before = driver.tree(SCENE)?.selected;
        hands::key(driver, key)?;
        driver.wait_for_tree(SCENE, "the selection to move", |tree| {
            (tree.selected != before).then_some(())
        })?;
    }
    Ok(())
}

/// E5: the menu of rows across the scene tree opened by the right button, and dismissed.
fn tree_menu(driver: &mut EditorDriver) -> Result<(), Failure> {
    let rows = spread(shown_rows(&driver.tree(SCENE)?), MENUS);
    if rows.is_empty() {
        return Err(harness("the scene tree shows no rows"));
    }
    for row in rows {
        open_and_dismiss_menu(driver, row_point(&row))?;
    }
    Ok(())
}

/// E6: a row of the scene tree dragged into a folder, and the move undone.
fn tree_drag(driver: &mut EditorDriver) -> Result<(), Failure> {
    let rows = shown_rows(&driver.tree(SCENE)?);
    let folders: Vec<&EditorTreeRow> = rows.iter().filter(|row| row.folder).collect();
    for leaf in rows.iter().filter(|row| !row.folder).take(DRAG_TRIES) {
        // The folder farthest off is the least likely to hold the row already.
        let Some(folder) = folders.iter().max_by(|a, b| {
            let away = |row: &EditorTreeRow| (row.rect.center().y - leaf.rect.center().y).abs();
            away(a).total_cmp(&away(b))
        }) else {
            break;
        };
        let before = driver.snapshot()?.undoable;
        hands::drag(
            driver,
            Button::Left,
            &KeyModifiers::default(),
            row_point(leaf),
            row_point(folder),
        )?;
        if driver.edited_since(before)? {
            return driver.undo();
        }
    }
    Err(harness(
        "no row of the scene tree moved when dragged onto a folder",
    ))
}

/// E7: the scene tree scrolled down and back up a line, three and ten lines at a turn.
fn tree_scroll(driver: &mut EditorDriver) -> Result<(), Failure> {
    let tree = driver.tree(SCENE)?;
    hands::move_pointer(driver, tree.rect.center())?;
    for lines in [1, 3, 10] {
        for _ in 0..SCROLLS {
            hands::wheel(driver, lines)?;
        }
        for _ in 0..SCROLLS {
            hands::wheel(driver, -lines)?;
        }
    }
    Ok(())
}

/// E8: the stage turned and panned there and back, flicked in, flicked back out as far as the
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

/// E21: the stage, drawn back as far as it goes, turned and panned there and back, and turned
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

/// E9: the pointer over each of what the stage holds nearest its middle in turn.
fn stage_hover(driver: &mut EditorDriver) -> Result<(), Failure> {
    for at in marks(&driver.snapshot()?, HOVERED)? {
        hands::move_pointer(driver, at)?;
    }
    Ok(())
}

/// E10: what the stage holds nearest its middle clicked, one after another, and the selection
/// let go.
fn stage_select(driver: &mut EditorDriver) -> Result<(), Failure> {
    for at in marks(&driver.snapshot()?, CLICKED)? {
        hands::move_pointer(driver, at)?;
        hands::click(driver, Button::Left, 1)?;
        hands::wait_for(driver, "the click to select", BUDGET, |snapshot| {
            (!snapshot.selected.is_empty()).then_some(())
        })?;
    }
    hands::key(driver, "Escape").map(drop)
}

/// E11: something the stage holds dragged a way off, and the move undone.
fn stage_move(driver: &mut EditorDriver) -> Result<(), Failure> {
    if !move_marks(driver, 1)? {
        return Err(harness("nothing the stage holds moved when dragged"));
    }
    driver.undo()
}

/// E12: a box dragged out from the stage's emptiest ground, selecting what it takes in.
fn stage_box(driver: &mut EditorDriver) -> Result<(), Failure> {
    let snapshot = driver.snapshot()?;
    let stage = stage_of(&snapshot)?;
    let mut from = None;
    for at in emptiest_points(&stage, &snapshot.covered)
        .into_iter()
        .take(EMPTY_TRIES)
    {
        let rested = hands::move_pointer(driver, at)?;
        if rested.stage.is_some_and(|stage| stage.picked.is_none()) {
            from = Some(at);
            break;
        }
    }
    let from = from.ok_or_else(|| {
        harness("everywhere the box was tried from, a click picks something the stage holds")
    })?;
    hands::drag(
        driver,
        Button::Left,
        &KeyModifiers::default(),
        from,
        toward_middle(&stage, from, BOX_DRAG),
    )?;
    hands::key(driver, "Escape").map(drop)
}

/// E13: the stage's menu opened by the right button over what it holds and over empty ground,
/// and dismissed.
fn stage_menu(driver: &mut EditorDriver) -> Result<(), Failure> {
    let snapshot = driver.snapshot()?;
    let stage = stage_of(&snapshot)?;
    let mut points = marks(&snapshot, MENUS)?;
    points.extend(emptiest_points(&stage, &snapshot.covered).first());
    for at in points {
        open_and_dismiss_menu(driver, at)?;
    }
    Ok(())
}

/// E14: strokes of the sculpting brush across the stage's middle.
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

/// E15: the area's pages shown one after another by their chips, and the tabs switched between
/// by their titles.
fn pages_and_tabs(driver: &mut EditorDriver) -> Result<(), Failure> {
    for page in PAGES {
        click_text(driver, page)?;
        wait_for_scene_tree(driver, page == SCENE_PAGE)?;
    }
    let area = front_tab(driver)?;
    let other = tab_beside(driver, &area)?;
    click_tab(driver, &other)?;
    click_tab(driver, &area)
}

/// A file of the game opened in a tab beside the area's, the area's brought back to the front.
fn open_file_tab(driver: &mut EditorDriver) -> Result<(), Failure> {
    let area = front_tab(driver)?;
    let mut opened: Vec<String> = Vec::new();
    let leaf = loop {
        let files = driver.tree(FILES)?;
        if let Some(leaf) = first_file(&files) {
            break leaf;
        }
        let rows = shown_rows(&files);
        // Each folder opened inside the one before, as a designer looks deeper for a file.
        let inside = opened
            .last()
            .and_then(|last| rows.iter().position(|row| &row.id == last))
            .map_or(0, |at| at + 1);
        let folder = rows[inside.min(rows.len())..]
            .iter()
            .find(|row| row.folder && !row.open)
            .cloned()
            .filter(|_| opened.len() < FILE_DEPTH)
            .ok_or_else(|| {
                let shown: Vec<String> = rows
                    .iter()
                    .map(|row| format!("{}{}", row.id, if row.folder { "/" } else { "" }))
                    .collect();
                harness(&format!(
                    "the files tree shows no file and no folder to open after opening {opened:?}: {shown:?}"
                ))
            })?;
        click_row(driver, &folder)?;
        driver.wait_for_tree(FILES, "the folder to open", |tree| {
            tree.rows
                .iter()
                .any(|row| row.id == folder.id && row.open)
                .then_some(())
        })?;
        opened.push(folder.id);
    };
    hands::move_pointer(driver, row_point(&leaf))?;
    driver.arriving();
    hands::click(driver, Button::Left, 2)?;
    hands::wait_for(driver, "the file to open in a tab", BUDGET, |snapshot| {
        (snapshot.tab.as_ref() != Some(&area)).then_some(())
    })?;
    driver.wait_stood()?;
    click_tab(driver, &area)
}

fn close_file_tab(driver: &mut EditorDriver) -> Result<(), Failure> {
    let area = front_tab(driver)?;
    let other = tab_beside(driver, &area)?;
    click_tab(driver, &other)?;
    driver.shortcut(CTRL, "w")?;
    hands::wait_for(driver, "the file's tab to close", BUDGET, |snapshot| {
        (snapshot.tab.as_ref() == Some(&area)).then_some(())
    })
}

/// E16: the latest edits undone and redone, fifty times each.
fn undo_redo(driver: &mut EditorDriver) -> Result<(), Failure> {
    for _ in 0..UNDO_ROUNDS {
        for _ in 0..EDITS {
            driver.undo()?;
        }
        for _ in 0..EDITS {
            driver.redo()?;
        }
    }
    Ok(())
}

fn make_edits(driver: &mut EditorDriver) -> Result<(), Failure> {
    if move_marks(driver, EDITS)? {
        Ok(())
    } else {
        Err(harness(
            "too little of what the stage holds moved when dragged to make the edits",
        ))
    }
}

/// E17: a field of the inspector clicked, typed into, the typing taken back, and let go.
fn field_typing(driver: &mut EditorDriver) -> Result<(), Failure> {
    let snapshot = driver.snapshot()?;
    let stage = stage_of(&snapshot)?;
    let field = snapshot
        .ui
        .iter()
        .find(|element| element.editable && element.rect.min_x() >= stage.viewport.max_x())
        .ok_or_else(|| harness("the inspector shows no field to type into"))?
        .rect;
    hands::move_pointer(driver, field.center())?;
    hands::click(driver, Button::Left, 1)?;
    hands::wait_for(driver, "the field to take the typing", BUDGET, |snapshot| {
        snapshot
            .ui
            .iter()
            .any(|element| element.editable && element.focused)
            .then_some(())
    })?;
    hands::type_text(driver, TYPED)?;
    for _ in TYPED.chars() {
        hands::key(driver, "BackSpace")?;
    }
    hands::key(driver, "Escape")?;
    hands::wait_for(driver, "the field to let go", BUDGET, |snapshot| {
        (!snapshot
            .ui
            .iter()
            .any(|element| element.editable && element.focused))
        .then_some(())
    })
}

fn select_middle(driver: &mut EditorDriver) -> Result<(), Failure> {
    let at = marks(&driver.snapshot()?, 1)?[0];
    hands::move_pointer(driver, at)?;
    hands::click(driver, Button::Left, 1)?;
    hands::wait_for(driver, "the click to select", BUDGET, |snapshot| {
        (!snapshot.selected.is_empty()).then_some(())
    })
}

/// E18: the folders of the scene tree in view hidden by their eyes, and shown again.
fn visibility_toggles(driver: &mut EditorDriver) -> Result<(), Failure> {
    let tree = driver.tree(SCENE)?;
    let mut toggled = 0;
    for folder in shown_rows(&tree).into_iter().filter(|row| row.folder) {
        if toggled == FOLDERS {
            break;
        }
        if eye(&driver.snapshot()?, &folder, HIDE).is_none() {
            continue;
        }
        for (press, then) in [(HIDE, SHOW), (SHOW, HIDE)] {
            let at = eye(&driver.snapshot()?, &folder, press)
                .ok_or_else(|| harness("a folder's eye went from its row"))?;
            hands::move_pointer(driver, at)?;
            hands::click(driver, Button::Left, 1)?;
            hands::wait_for(driver, "the eye to turn", BUDGET, |snapshot| {
                eye(snapshot, &folder, then).map(drop)
            })?;
        }
        toggled += 1;
    }
    if toggled == 0 {
        return Err(harness("no folder of the scene tree shows an eye to press"));
    }
    Ok(())
}

/// E19: the content panel widened by its edge and narrowed back, the bottom panel raised and
/// lowered back, and the file's tab docked beside the area's, closed there and opened again.
fn panels_and_dock(driver: &mut EditorDriver) -> Result<(), Failure> {
    panel_resize(driver)?;
    let area = front_tab(driver)?;
    let other = tab_beside(driver, &area)?;
    let whole = driver.stage()?.viewport;
    let beside = Pos::new(
        whole.max_x() - whole.width() * DOCK_ACROSS,
        whole.center().y,
    );
    let from = tab_title(&driver.snapshot()?, &other)?.center();
    hands::drag(driver, Button::Left, &KeyModifiers::default(), from, beside)?;
    let docked = hands::wait_for(
        driver,
        "the tab to dock beside the area's",
        BUDGET,
        |snapshot| {
            snapshot
                .stage
                .clone()
                .filter(|stage| stage.viewport.width() < whole.width() * DOCKED_SHARE)
        },
    )?;
    let snapshot = driver.snapshot()?;
    let in_docked = snapshot
        .ui
        .iter()
        .find(|element| {
            element.text.as_deref() == Some(other.as_str())
                && !docked.viewport.contains(element.rect.center())
        })
        .map(|element| element.rect.center())
        .ok_or_else(|| harness("the docked tab shows nothing reading its title"))?;
    hands::move_pointer(driver, in_docked)?;
    hands::click(driver, Button::Left, 1)?;
    hands::wait_for(
        driver,
        "the docked tab to come to the front",
        BUDGET,
        |snapshot| (snapshot.tab.as_deref() == Some(other.as_str())).then_some(()),
    )?;
    driver.shortcut(CTRL, "w")?;
    hands::wait_for(driver, "the docked tab to close", BUDGET, |snapshot| {
        snapshot
            .stage
            .as_ref()
            .is_some_and(|stage| stage.viewport.width() >= whole.width() * DOCKED_SHARE)
            .then_some(())
    })?;
    open_file_tab(driver)
}

/// The content panel widened by its edge and narrowed back, the bottom panel raised and lowered
/// back.
fn panel_resize(driver: &mut EditorDriver) -> Result<(), Failure> {
    for (name, edge, by) in [
        (CONTENT_PANEL, Edge::Right, Offset::new(PANEL_DRAG, 0.0)),
        (BOTTOM_PANEL, Edge::Top, Offset::new(0.0, -PANEL_DRAG)),
    ] {
        for by in [by, -by] {
            let rect = panel(&driver.snapshot()?, name)?;
            let from = match edge {
                Edge::Right => Pos::new(rect.max_x() - 1.0, rect.center().y),
                Edge::Top => Pos::new(rect.center().x, rect.min_y() + 1.0),
            };
            hands::drag(
                driver,
                Button::Left,
                &KeyModifiers::default(),
                from,
                from + by,
            )?;
            hands::wait_for(
                driver,
                "the panel to take its new size",
                BUDGET,
                |snapshot| {
                    panel(snapshot, name)
                        .is_ok_and(|now| now.size != rect.size)
                        .then_some(())
                },
            )?;
        }
    }
    Ok(())
}

/// E20: the stage turned a little, the scene tree swept over, the stage turned back, and the tree
/// swept over again.
fn hover_while_orbiting(driver: &mut EditorDriver) -> Result<(), Failure> {
    let middle = driver.stage()?.viewport.center();
    for dx in [ORBIT_DRAG / 2.0, -ORBIT_DRAG / 2.0] {
        turn(driver, middle, dx)?;
        let rows: Vec<Pos<WindowPx>> = shown_rows(&driver.tree(SCENE)?)
            .iter()
            .map(row_point)
            .collect();
        hands::sweep(driver, &rows)?;
    }
    Ok(())
}

/// Waits for the scene tree to be drawn, as only the scene page draws it, or not.
fn wait_for_scene_tree(driver: &mut EditorDriver, drawn: bool) -> Result<(), Failure> {
    let since = Instant::now();
    while driver.tree(SCENE).is_ok() != drawn {
        if since.elapsed() >= BUDGET {
            return Err(harness("the page never showed"));
        }
    }
    Ok(())
}

/// The rows of `tree` scrolled into its rect.
fn shown_rows(tree: &EditorTree) -> Vec<EditorTreeRow> {
    tree.rows
        .iter()
        .filter(|row| tree.rect.contains(row.rect.center()))
        .cloned()
        .collect()
}

/// Where a row's name shows: on its left, clear of the eye and padlock at its right end.
fn row_point(row: &EditorTreeRow) -> Pos<WindowPx> {
    Pos::new(
        row.rect.min_x() + row.rect.width() * NAME_ACROSS,
        row.rect.center().y,
    )
}

fn click_row(driver: &mut EditorDriver, row: &EditorTreeRow) -> Result<(), Failure> {
    hands::move_pointer(driver, row_point(row))?;
    hands::click(driver, Button::Left, 1).map(drop)
}

/// As many as `count` of `rows`, evenly apart from the first to the last.
fn spread(rows: Vec<EditorTreeRow>, count: usize) -> Vec<EditorTreeRow> {
    if rows.len() <= count {
        return rows;
    }
    (0..count)
        .map(|index| rows[index * (rows.len() - 1) / (count - 1).max(1)].clone())
        .collect()
}

/// The first file the tree shows but for those selected, as the file of the tab in front is.
fn first_file(tree: &EditorTree) -> Option<EditorTreeRow> {
    shown_rows(tree).into_iter().find(|row| {
        !row.folder
            && !tree
                .selected
                .iter()
                .any(|selected| row.id.starts_with(selected.as_str()))
    })
}

fn open_and_dismiss_menu(driver: &mut EditorDriver, at: Pos<WindowPx>) -> Result<(), Failure> {
    hands::move_pointer(driver, at)?;
    hands::click(driver, Button::Right, 1)?;
    hands::wait_for(driver, "the menu to open", BUDGET, |snapshot| {
        snapshot.menu_open.then_some(())
    })?;
    hands::key(driver, "Escape")?;
    hands::wait_for(driver, "the menu to close", BUDGET, |snapshot| {
        (!snapshot.menu_open).then_some(())
    })
}

fn stage_of(snapshot: &EditorSnapshot) -> Result<EditorStage, Failure> {
    snapshot
        .stage
        .clone()
        .ok_or_else(|| harness("the editor shows no stage"))
}

/// Where up to `count` of what the stage holds nearest its middle show, clear of the interface
/// floating over it.
fn marks(snapshot: &EditorSnapshot, count: usize) -> Result<Vec<Pos<WindowPx>>, Failure> {
    let stage = stage_of(snapshot)?;
    let marks: Vec<Pos<WindowPx>> = stage
        .marks
        .iter()
        .map(|mark| mark.at)
        .filter(|&at| clear(&stage, &snapshot.covered, at))
        .take(count)
        .collect();
    if marks.is_empty() {
        return Err(harness("the stage shows nothing it holds"));
    }
    Ok(marks)
}

/// Drags `count` of what the stage holds a little way off, each its own edit: whether as many
/// moved.
fn move_marks(driver: &mut EditorDriver, count: usize) -> Result<bool, Failure> {
    let mut moved = 0;
    for at in marks(&driver.snapshot()?, MARKS_TRIED)? {
        if moved == count {
            break;
        }
        let before = driver.snapshot()?.undoable;
        hands::drag(
            driver,
            Button::Left,
            &KeyModifiers::default(),
            at,
            at + Offset::new(MOVE_DRAG, MOVE_DRAG / 2.0),
        )?;
        if driver.edited_since(before)? {
            moved += 1;
        }
    }
    Ok(moved == count)
}

/// Points of the stage's middle ground clear of the interface floating over it, those farthest
/// from anything it holds first.
fn emptiest_points(stage: &EditorStage, covered: &[Rect<WindowPx>]) -> Vec<Pos<WindowPx>> {
    let viewport = stage.viewport;
    let share = |step: u32| EMPTY_MARGIN + (1.0 - 2.0 * EMPTY_MARGIN) * step as f32 / GRID as f32;
    let mut points: Vec<(Pos<WindowPx>, f32)> = (0..=GRID)
        .flat_map(|row| (0..=GRID).map(move |column| (row, column)))
        .map(|(row, column)| {
            Pos::new(
                viewport.min_x() + viewport.width() * share(column),
                viewport.min_y() + viewport.height() * share(row),
            )
        })
        .filter(|&at| clear(stage, covered, at))
        .map(|at| {
            let room = stage
                .marks
                .iter()
                .map(|mark| mark.at.distance_to(at))
                .fold(f32::MAX, f32::min);
            (at, room)
        })
        .collect();
    points.sort_by(|a, b| b.1.total_cmp(&a.1));
    points.into_iter().map(|(at, _)| at).collect()
}

fn toward_middle(stage: &EditorStage, from: Pos<WindowPx>, by: f32) -> Pos<WindowPx> {
    let middle = stage.viewport.center();
    let away = middle - from;
    let length = away.length().max(1.0);
    from + away * (by.min(length) / length)
}

fn clear(stage: &EditorStage, covered: &[Rect<WindowPx>], at: Pos<WindowPx>) -> bool {
    stage.viewport.contains(at) && !covered.iter().any(|rect| rect.contains(at))
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

/// Clicks the element of the interface reading exactly `text`.
fn click_text(driver: &mut EditorDriver, text: &str) -> Result<(), Failure> {
    let element = driver
        .snapshot()?
        .ui
        .into_iter()
        .find(|element| element.text.as_deref() == Some(text))
        .ok_or_else(|| harness(&format!("the editor shows nothing reading {text:?}")))?;
    hands::move_pointer(driver, element.rect.center())?;
    hands::click(driver, Button::Left, 1).map(drop)
}

fn front_tab(driver: &mut EditorDriver) -> Result<String, Failure> {
    driver
        .snapshot()?
        .tab
        .ok_or_else(|| harness("the editor shows no tab"))
}

/// Where the title of the tab `title` shows: right of the content panel, the highest of what reads
/// so.
fn tab_title(snapshot: &EditorSnapshot, title: &str) -> Result<Rect<WindowPx>, Failure> {
    let dock = panel(snapshot, CONTENT_PANEL)?.max_x();
    snapshot
        .ui
        .iter()
        .filter(|element| element.text.as_deref() == Some(title) && element.rect.min_x() >= dock)
        .map(|element| element.rect)
        .min_by(|a, b| a.min_y().total_cmp(&b.min_y()))
        .ok_or_else(|| harness(&format!("no tab shows the title {title:?}")))
}

/// The title of the tab next right of the tab `title`.
fn tab_beside(driver: &mut EditorDriver, title: &str) -> Result<String, Failure> {
    let snapshot = driver.snapshot()?;
    let shown = tab_title(&snapshot, title)?;
    snapshot
        .ui
        .iter()
        .filter(|element| {
            element.text.is_some()
                && element.rect.min_x() > shown.max_x()
                && (element.rect.center().y - shown.center().y).abs() < shown.height() / 2.0
        })
        .min_by(|a, b| a.rect.min_x().total_cmp(&b.rect.min_x()))
        .and_then(|element| element.text.clone())
        .ok_or_else(|| harness("no tab shows beside the area's"))
}

fn click_tab(driver: &mut EditorDriver, title: &str) -> Result<(), Failure> {
    let at = tab_title(&driver.snapshot()?, title)?.center();
    hands::move_pointer(driver, at)?;
    hands::click(driver, Button::Left, 1)?;
    hands::wait_for(driver, "the tab to come to the front", BUDGET, |snapshot| {
        (snapshot.tab.as_deref() == Some(title)).then_some(())
    })
}

/// Where the eye of `folder` shows when it reads as starting with `verb`.
fn eye(snapshot: &EditorSnapshot, folder: &EditorTreeRow, verb: &str) -> Option<Pos<WindowPx>> {
    snapshot
        .ui
        .iter()
        .find(|element| {
            element.button
                && element
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with(verb) && !text.contains(SET_ABOVE))
                && folder.rect.contains(element.rect.center())
        })
        .map(|element| element.rect.center())
}

fn panel(snapshot: &EditorSnapshot, name: &str) -> Result<Rect<WindowPx>, Failure> {
    snapshot
        .panels
        .iter()
        .find(|panel| panel.name == name)
        .map(|panel| panel.rect)
        .ok_or_else(|| harness(&format!("the editor shows no {name} panel")))
}

#[derive(Clone, Copy)]
enum Edge {
    Right,
    Top,
}

fn harness(why: &str) -> Failure {
    Failure::Harness(why.to_owned())
}

/// The trees and panels as the editor names them.
const SCENE: &str = "scene";
const FILES: &str = "files";
const CONTENT_PANEL: &str = "content";
const BOTTOM_PANEL: &str = "bottom";
/// The area's pages by their chips, ending on the scene the other scenarios play on.
const PAGES: [&str; 4] = ["terrain", "lighting", "climate", SCENE_PAGE];
const SCENE_PAGE: &str = "scene";
/// How a row's eye reads, by what pressing it does; one set by a folder above says so.
const HIDE: &str = "Hide ";
const SHOW: &str = "Show ";
const SET_ABOVE: &str = "now set by a folder above";
/// How far across a row its name shows.
const NAME_ACROSS: f32 = 0.35;
const FOLDERS: usize = 4;
const KEY_STEPS: usize = 8;
const MENUS: usize = 4;
const DRAG_TRIES: usize = 3;
const SCROLLS: u32 = 4;
/// Pixels of drag that turn the stage, and pan it.
const ORBIT_DRAG: f32 = 320.0;
const PAN_DRAG: Offset<WindowPx> = Offset::new(220.0, 120.0);
/// Lines of the wheel, out when positive, in one quick flick: as many as take the camera from as
/// far as it draws back to as near as it comes, and back.
const FLICK: i32 = 40;
const QUICK_TURN: f32 = 480.0;
const QUICK_STEPS: u32 = 16;
const HOVERED: usize = 16;
const CLICKED: usize = 8;
/// Of what the stage holds, how many are tried to make a move of.
const MARKS_TRIED: usize = 16;
const MOVE_DRAG: f32 = 48.0;
const BOX_DRAG: f32 = 240.0;
/// The grid of the stage the emptiest ground is looked for on, clear of its edges by this share.
const GRID: u32 = 8;
const EMPTY_MARGIN: f32 = 0.15;
const EMPTY_TRIES: usize = 12;
const STROKE: f32 = 160.0;
const STROKE_ROWS: [f32; 3] = [-80.0, 0.0, 80.0];
const EDITS: usize = 5;
/// Rounds of undoing and redoing every edit made: fifty of each.
const UNDO_ROUNDS: usize = 10;
const TYPED: &str = "quiet";
const PANEL_DRAG: f32 = 140.0;
/// How far in from the stage's right edge a tab is dropped to dock beside it, and how much of its
/// width the stage keeps at most once one has.
const DOCK_ACROSS: f32 = 0.1;
const DOCKED_SHARE: f32 = 0.8;
/// How many folders of the files tree are opened, one inside another, to find a file in.
const FILE_DEPTH: usize = 6;
