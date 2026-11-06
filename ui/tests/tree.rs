use std::collections::BTreeSet;

use egui::{Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use ui::tree::{DropSpot, TreeLine, TreeList, TreeOutput, TreeState, VirtualTree};

const ROW: f32 = 20.0;
const SCREEN: Vec2 = Vec2::new(300.0, 200.0);

/// A tree of `folders` folders of `files` files each, drawn frame after frame as a user works it.
struct Tree {
    ctx: Context,
    time: f64,
    folders: usize,
    files: usize,
    open: BTreeSet<String>,
    state: TreeState,
    list: TreeList,
    /// What a right click's menu was asked for, last.
    menu: Option<Vec<String>>,
}

impl Tree {
    fn new(folders: usize, files: usize) -> Tree {
        let ctx = Context::default();
        ui::apply(&ctx);
        let mut tree = Tree {
            ctx,
            time: 0.0,
            folders,
            files,
            open: BTreeSet::new(),
            state: TreeState::default(),
            list: TreeList::default(),
            menu: None,
        };
        tree.relist();
        tree
    }

    fn relist(&mut self) {
        let mut lines = Vec::new();
        for folder in 0..self.folders {
            let id = format!("f{folder}");
            let open = self.open.contains(&id);
            lines.push(TreeLine {
                id: id.clone(),
                depth: 0,
                folder: true,
                open,
            });
            if open {
                lines.extend((0..self.files).map(|file| TreeLine {
                    id: format!("{id}/{file}"),
                    depth: 1,
                    folder: false,
                    open: false,
                }));
            }
        }
        self.list = TreeList::new(lines);
    }

    /// One frame of `events` with `modifiers` held: what the tree said, the folders it opened or
    /// closed then opened or closed.
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) -> TreeOutput {
        self.time += 0.05;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            time: Some(self.time),
            events: [Event::ModifiersChanged(modifiers)]
                .into_iter()
                .chain(events)
                .collect(),
            ..RawInput::default()
        };
        let mut said = None;
        let mut menu = None;
        let mut output = self.ctx.run_ui(input, |ui| {
            let out = VirtualTree::new(Id::new("tree"), &self.list)
                .row_height(ROW)
                .show(
                    ui,
                    &mut self.state,
                    |ui, line, _| {
                        ui.label(&line.id);
                    },
                    |ui, ids| {
                        menu = Some(ids.to_vec());
                        ui.label("menu");
                    },
                );
            said = Some(out);
        });
        output.textures_delta.clear();
        if menu.is_some() {
            self.menu = menu;
        }
        let said = said.expect("the tree drawn");
        for (folder, open) in &said.opened {
            if *open {
                self.open.insert(folder.clone());
            } else {
                self.open.remove(folder);
            }
        }
        if !said.opened.is_empty() {
            self.relist();
        }
        said
    }

    fn rest(&mut self) -> TreeOutput {
        self.frame(Vec::new(), Modifiers::NONE)
    }

    /// Where the row of `id` was drawn last frame.
    fn at(&mut self, id: &str) -> Pos2 {
        let out = self.rest();
        out.rows
            .iter()
            .find(|(row, _)| row == id)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{id} is drawn: {:?}", out.rows))
    }

    /// A click on the row of `id`, with `modifiers` held: what the tree said as it let go.
    fn click(&mut self, id: &str, modifiers: Modifiers) -> TreeOutput {
        let at = self.at(id);
        self.press(at, PointerButton::Primary, modifiers)
    }

    fn press(&mut self, at: Pos2, button: PointerButton, modifiers: Modifiers) -> TreeOutput {
        let event = |pressed| Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers,
        };
        self.frame(vec![Event::PointerMoved(at)], modifiers);
        self.frame(vec![event(true)], modifiers);
        self.frame(vec![event(false)], modifiers)
    }

    /// A press of `key` with `modifiers` held, a moment after what came before.
    fn key(&mut self, key: Key, modifiers: Modifiers) -> TreeOutput {
        self.rest();
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![event(true), event(false)], modifiers)
    }

    /// Rows pressed at `from` and dragged to `to`, resting there `rest` frames: what the tree said
    /// while the drag rested, and as it let go, unless `cancelled` first.
    fn drag(
        &mut self,
        from: Pos2,
        to: Pos2,
        rest: usize,
        cancelled: bool,
    ) -> (TreeOutput, TreeOutput) {
        let button = |pressed, pos| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(from)], Modifiers::NONE);
        self.frame(vec![button(true, from)], Modifiers::NONE);
        for step in 1..=4 {
            let at = from + (to - from) * (step as f32 / 4.0);
            self.frame(vec![Event::PointerMoved(at)], Modifiers::NONE);
        }
        let mut resting = self.rest();
        for _ in 0..rest {
            resting = self.rest();
        }
        if cancelled {
            self.key(Key::Escape, Modifiers::NONE);
        }
        let dropped = self.frame(vec![button(false, to)], Modifiers::NONE);
        (resting, dropped)
    }
}

/// What a user does to a tree, and what the tree says of it.
type Act = fn(&mut Tree) -> TreeOutput;

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// However many rows a tree lists, it draws only those in view, and scrolls a row asked for into
/// view wherever it lies.
#[test]
fn a_tree_draws_only_the_rows_in_view_and_scrolls_to_any_row() {
    let mut tree = Tree::new(20, 5000);
    tree.open = (0..20).map(|folder| format!("f{folder}")).collect();
    tree.relist();
    assert_eq!(tree.list.len(), 20 * 5001);

    let drawn = tree.rest();
    assert!(
        drawn.rows.len() <= (SCREEN.y / ROW) as usize + 2,
        "{} rows drawn of {}",
        drawn.rows.len(),
        tree.list.len()
    );
    assert_eq!(drawn.rows.first().map(|(id, _)| id.as_str()), Some("f0"));

    tree.state.scroll_to("f13/4321");
    tree.rest();
    tree.rest();
    let row = tree.at("f13/4321");
    assert!(
        row.y > 0.0 && row.y < SCREEN.y,
        "the row scrolled to shows, at {row:?}"
    );
}

/// A click selects a row alone, the command key adds or takes a row away, and shift picks every
/// row from the last picked; the keys move the selection, shift stretching it, and open, close
/// and leave folders.
#[test]
fn rows_are_selected_by_click_and_by_keys() {
    let mut tree = Tree::new(3, 3);
    tree.open.insert("f1".to_owned());
    tree.relist();

    let cases: [(&str, Act, Vec<String>); 8] = [
        (
            "a click",
            |tree| tree.click("f1/0", Modifiers::NONE),
            ids(&["f1/0"]),
        ),
        (
            "a command click",
            |tree| tree.click("f1/2", Modifiers::COMMAND),
            ids(&["f1/0", "f1/2"]),
        ),
        (
            "a command click on a picked row",
            |tree| tree.click("f1/0", Modifiers::COMMAND),
            ids(&["f1/2"]),
        ),
        (
            "a shift click",
            |tree| tree.click("f0", Modifiers::SHIFT),
            ids(&["f0", "f1", "f1/0"]),
        ),
        (
            "down",
            |tree| tree.key(Key::ArrowDown, Modifiers::NONE),
            ids(&["f1"]),
        ),
        (
            "shift down",
            |tree| tree.key(Key::ArrowDown, Modifiers::SHIFT),
            ids(&["f1", "f1/0"]),
        ),
        (
            "end",
            |tree| tree.key(Key::End, Modifiers::NONE),
            ids(&["f2"]),
        ),
        (
            "home",
            |tree| tree.key(Key::Home, Modifiers::NONE),
            ids(&["f0"]),
        ),
    ];
    for (what, act, expected) in cases {
        let said = act(&mut tree);
        assert_eq!(tree.state.selected(), expected, "{what} selects");
        assert_eq!(
            said.selected.as_deref(),
            Some(expected.as_slice()),
            "{what} says it selected"
        );
    }

    tree.key(Key::ArrowDown, Modifiers::NONE);
    tree.key(Key::ArrowLeft, Modifiers::NONE);
    assert!(!tree.open.contains("f1"), "left closes an open folder");
    tree.key(Key::ArrowRight, Modifiers::NONE);
    assert!(tree.open.contains("f1"), "right opens a closed one");
    tree.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(
        tree.state.selected(),
        ids(&["f1/0"]),
        "right goes into an open folder"
    );
    tree.key(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        tree.state.selected(),
        ids(&["f1"]),
        "left goes up to the folder holding the row"
    );
    let said = tree.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        said.activated,
        ids(&["f1"]),
        "Enter activates the selection"
    );
}

/// A click on a folder's row opens or closes it, as does one on its caret; a double click on a
/// row activates it; a right click opens the menu on the rows it picks, or on none below them.
#[test]
fn a_click_opens_a_folder_a_double_click_activates_and_a_right_click_opens_a_menu() {
    let mut tree = Tree::new(3, 2);
    for (what, x) in [("its caret", 8.0), ("its row", 150.0)] {
        let y = tree.at("f0").y;
        tree.press(Pos2::new(x, y), PointerButton::Primary, Modifiers::NONE);
        assert!(
            tree.open.contains("f0"),
            "a click on {what} opens the folder"
        );
        tree.press(Pos2::new(x, y), PointerButton::Primary, Modifiers::NONE);
        assert!(!tree.open.contains("f0"), "another closes it");
    }

    tree.time += 1.0;
    let at = tree.at("f2");
    let event = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    tree.frame(vec![Event::PointerMoved(at)], Modifiers::NONE);
    tree.frame(vec![event(true)], Modifiers::NONE);
    tree.frame(vec![event(false)], Modifiers::NONE);
    tree.frame(vec![event(true)], Modifiers::NONE);
    let said = tree.frame(vec![event(false)], Modifiers::NONE);
    assert_eq!(
        said.activated,
        ids(&["f2"]),
        "a double click activates the row"
    );

    let at = tree.at("f1");
    tree.press(at, PointerButton::Secondary, Modifiers::NONE);
    tree.rest();
    assert_eq!(
        tree.menu,
        Some(ids(&["f1"])),
        "a right click picks the row for its menu"
    );
    tree.key(Key::Escape, Modifiers::NONE);
    let below = Pos2::new(150.0, 6.5 * ROW);
    tree.press(below, PointerButton::Secondary, Modifiers::NONE);
    tree.rest();
    assert_eq!(
        tree.menu,
        Some(Vec::new()),
        "a right click below the rows opens the tree's menu"
    );
}

/// Rows dragged land before, into or after the row they are let go on, or below every row; a
/// drag resting on a closed folder opens it, and one cancelled lands nowhere.
#[test]
fn dragged_rows_land_where_they_are_let_go() {
    let mut tree = Tree::new(3, 2);
    tree.open.insert("f2".to_owned());
    tree.relist();
    tree.rest();
    let row = |tree: &mut Tree, id: &str, share: f32| {
        let center = tree.at(id);
        Pos2::new(center.x, center.y - ROW / 2.0 + ROW * share)
    };
    for (what, share, on, expected) in [
        (
            "on a folder's top",
            0.1,
            "f1",
            DropSpot::Before("f1".to_owned()),
        ),
        (
            "on a folder's middle",
            0.5,
            "f1",
            DropSpot::Into("f1".to_owned()),
        ),
        (
            "on a folder's bottom",
            0.9,
            "f1",
            DropSpot::After("f1".to_owned()),
        ),
        (
            "on a file's top half",
            0.4,
            "f2/1",
            DropSpot::Before("f2/1".to_owned()),
        ),
        (
            "on a file's bottom half",
            0.6,
            "f2/1",
            DropSpot::After("f2/1".to_owned()),
        ),
    ] {
        let from = tree.at("f2/0");
        let to = row(&mut tree, on, share);
        let (resting, dropped) = tree.drag(from, to, 0, false);
        assert_eq!(
            resting.dragging,
            Some((ids(&["f2/0"]), expected.clone())),
            "a drag resting {what} says where it would land"
        );
        assert_eq!(
            dropped.dropped,
            Some((ids(&["f2/0"]), expected)),
            "rows let go {what} land there"
        );
    }

    let from = tree.at("f2/0");
    let below = Pos2::new(150.0, 7.5 * ROW);
    let (_, dropped) = tree.drag(from, below, 0, false);
    assert_eq!(dropped.dropped, Some((ids(&["f2/0"]), DropSpot::End)));

    let from = tree.at("f2/0");
    let to = row(&mut tree, "f0", 0.5);
    tree.drag(from, to, 15, true);
    assert!(
        tree.open.contains("f0"),
        "a drag resting on a closed folder opens it"
    );
    let from = tree.at("f2/0");
    let to = row(&mut tree, "f1", 0.5);
    let (_, dropped) = tree.drag(from, to, 0, true);
    assert_eq!(dropped.dropped, None, "a cancelled drag lands nowhere");
}
