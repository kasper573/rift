use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use ui::egui;
use ui::menu;
use ui::theme;
use ui::words;

use engine::core::content::{ContentFile, ErrorPlace, table_of};
use engine::systems::area::blueprint::{BlueprintDef, Placement};
use engine::systems::area::scene::{MAX_FOLDER_DEPTH, SceneNode, scene_file};
use engine::systems::area::{AreaModule, AreaModuleKind, Pictured};
use ui::egui::Ui;

use super::flags::Flag;
use super::modules::ADD_MENU;
use super::nodes::{self, Landing, NodeFlag, Remap};
use super::{AreaSnapshot, shapes};
use crate::app::{Clipboard, Editor, SceneClip};
use crate::keymap::EditorKey;
use crate::picker::Dragged;
use crate::project::{ContentEdit, Project, RowSelection};
use crate::scene_graph;
pub use crate::shell::tree::Toggle;
use crate::shell::tree::{self, DropAt, RowToggles, ToggleState, TreeDrop, TreeRow, TreeSource};
use crate::shell::widgets;
use crate::tabs::file_title;

/// What the scene graph's menus and keys ask for.
#[derive(Clone, Debug)]
pub enum SceneAction {
    Add {
        folder: String,
        kind: AreaModuleKind,
    },
    NewFolder(String),
    Rename(String),
    ShowOnMap(String),
    OpenBlueprint(String),
    EditInPlace(String),
    DoneEditing,
    Unpack(String),
    Replace(String),
    ReturnDoor(String),
    MoveTo(Vec<String>),
    Group(Vec<String>),
    Ungroup(Vec<String>),
    Toggle(Vec<String>, Toggle),
    ShowOnly(String),
    Copy(Vec<String>),
    Cut(Vec<String>),
    Paste(String),
    Duplicate(Vec<String>),
    MakeBlueprint(Vec<String>),
    Delete(Vec<String>),
    OpenAll,
    CloseAll,
    ShowEverything,
    UnlockEverything,
}

impl SceneAction {
    /// Whether it writes to the game folder, as opposed to only changing what the view shows.
    pub fn writes(&self) -> bool {
        !matches!(
            self,
            SceneAction::ShowOnMap(_)
                | SceneAction::OpenBlueprint(_)
                | SceneAction::EditInPlace(_)
                | SceneAction::DoneEditing
                | SceneAction::ShowOnly(_)
                | SceneAction::Copy(_)
                | SceneAction::OpenAll
                | SceneAction::CloseAll
        )
    }
}

/// The scene graph of an area or blueprint as a tree: its nodes in order, and while a placement is
/// edited in place, its blueprint's scene opened under its row.
pub struct SceneSource<'a> {
    snapshot: &'a AreaSnapshot,
    /// The rows of the placements edited in place, outermost first.
    in_place: Vec<String>,
    narrowed: Option<Arc<Narrowed>>,
    needle: String,
    holding: HashSet<String>,
    problems: HashMap<String, String>,
    pub lit: Option<String>,
    /// The folder row a module waiting for its spot on the map lands in, and its kind: shown as a
    /// faded row at the end of the folder.
    pub placing: Option<(String, AreaModuleKind)>,
    read_only: Option<String>,
    can_paste: bool,
}

/// What the filter and Show leave of a scene, kept while neither nor the scene changes: finding
/// it walks every row.
pub struct Narrowed {
    from: NarrowedFrom,
    /// The rows shown, and the folders above them.
    shown: HashSet<String>,
    /// The rows that pass the filter and Show, among those shown.
    hits: HashSet<String>,
    counted: (usize, usize),
}

#[derive(PartialEq)]
struct NarrowedFrom {
    made: u64,
    needle: String,
    hidden_kinds: BTreeSet<String>,
    hide_empty_folders: bool,
}

/// What narrows the scene's rows: the filter typed, what Show hides, and what they left of the
/// scene last.
pub struct SceneNarrowing<'b> {
    pub filter: &'b str,
    pub view: &'b crate::user::TreeView,
    pub kept: &'b mut Option<Arc<Narrowed>>,
}

impl<'a> SceneSource<'a> {
    pub fn new(
        project: &Project,
        snapshot: &'a AreaSnapshot,
        selection: &RowSelection,
        narrowing: SceneNarrowing,
        can_paste: bool,
    ) -> SceneSource<'a> {
        let SceneNarrowing { filter, view, kept } = narrowing;
        let in_place: Vec<String> = (1..=snapshot.in_place.len())
            .map(|depth| place_ids(&snapshot.in_place[..depth]))
            .collect();
        let mut source = SceneSource {
            snapshot,
            in_place,
            narrowed: None,
            needle: filter.trim().to_lowercase(),
            holding: HashSet::new(),
            problems: problems(project, snapshot),
            lit: None,
            placing: None,
            read_only: project.read_only.clone(),
            can_paste,
        };
        let selected = selected_ids(snapshot, selection);
        for id in &selected {
            let mut parent = source.parent(id);
            while !parent.is_empty() {
                source.holding.insert(parent.clone());
                parent = source.parent(&parent);
            }
        }
        if !source.needle.is_empty() || !view.hidden_kinds.is_empty() || view.hide_empty_folders {
            let from = NarrowedFrom {
                made: snapshot.made,
                needle: source.needle.clone(),
                hidden_kinds: view.hidden_kinds.clone(),
                hide_empty_folders: view.hide_empty_folders,
            };
            let narrowed = match kept.take() {
                Some(kept) if kept.from == from => kept,
                _ => Arc::new(source.narrow(from, view)),
            };
            *kept = Some(narrowed.clone());
            source.narrowed = Some(narrowed);
        }
        source
    }

    fn narrow(&self, from: NarrowedFrom, view: &crate::user::TreeView) -> Narrowed {
        let mut shown = HashSet::new();
        let mut hits = HashSet::new();
        let mut stack = vec![String::new()];
        while let Some(id) = stack.pop() {
            for child in self.all_children(&id) {
                if self.is_folder(&child) {
                    if self.needle.is_empty() && !view.hide_empty_folders {
                        shown.insert(child.clone());
                    }
                    stack.push(child.clone());
                }
                let Some(node) = self.snapshot.scene_node(&child) else {
                    continue;
                };
                let kind = nodes::kind_tag(node);
                let label = self.label_of(&child).to_lowercase();
                let passes = !nodes::is_folder(node)
                    && !view.hidden_kinds.contains(kind)
                    && (self.needle.is_empty() || label.contains(&self.needle));
                let folder_matches = nodes::is_folder(node)
                    && !self.needle.is_empty()
                    && label.contains(&self.needle);
                if passes || folder_matches {
                    hits.insert(child.clone());
                    shown.insert(child.clone());
                    let mut parent = self.parent(&child);
                    while !parent.is_empty() {
                        shown.insert(parent.clone());
                        parent = self.parent(&parent);
                    }
                }
            }
        }
        let leaves = hits.iter().filter(|id| !self.is_folder(id)).count();
        Narrowed {
            from,
            shown,
            hits,
            counted: (leaves, self.count_rows()),
        }
    }

    fn count_rows(&self) -> usize {
        let mut total = 0;
        let mut stack = vec![String::new()];
        while let Some(id) = stack.pop() {
            for child in self.all_children(&id) {
                if self.is_folder(&child) {
                    stack.push(child);
                } else {
                    total += 1;
                }
            }
        }
        total
    }

    /// How many rows pass the filter and Show, and how many there are.
    pub fn hit_count(&self) -> Option<(usize, usize)> {
        self.narrowed.as_ref().map(|narrowed| narrowed.counted)
    }

    fn all_children(&self, id: &str) -> Vec<String> {
        let count = rows_in(self.snapshot, id).len();
        let waiting = self
            .placing
            .as_ref()
            .is_some_and(|(folder, _)| folder == id);
        let prefix = match id {
            "" => String::new(),
            id if self.in_place.iter().any(|place| place == id) => format!("{id}/in"),
            id => format!("{id}/children"),
        };
        (0..count + usize::from(waiting))
            .map(|index| format!("{prefix}/{index}"))
            .collect()
    }

    /// The faded row of a module waiting for its spot, when `id` is it.
    fn waiting(&self, id: &str) -> Option<TreeRow> {
        let (folder, kind) = self.placing.as_ref()?;
        if parent_id(id) != *folder || self.snapshot.scene_node(id).is_some() {
            return None;
        }
        Some(TreeRow {
            name: format!("new {}", kind.tag().replace('_', " ")),
            badge: Some((
                widgets::badge_letter(kind.tag()),
                widgets::badge_color(kind.tag()),
            )),
            dim: true,
            detail: "click the map".to_owned(),
            ..TreeRow::default()
        })
    }

    fn is_placement(&self, id: &str) -> bool {
        self.snapshot
            .scene_node(id)
            .and_then(nodes::placement_of)
            .is_some()
    }

    /// Whether the row sits outside the placement edited in place, so it acts locked.
    fn outside(&self, id: &str) -> bool {
        self.in_place
            .last()
            .is_some_and(|place| !id.starts_with(&format!("{place}/in/")))
    }

    /// What a placement's row says on hover: what it places, how often, and the ways into it.
    fn placement_tip(&self, id: &str, placement: &Placement) -> String {
        let blueprint = self
            .snapshot
            .reader
            .path_of(placement.blueprint)
            .unwrap_or_default();
        let mut count = 0;
        nodes::walk(scene_at(self.snapshot, Self::level(id)), &mut |_, node| {
            if let SceneNode::Module(module) = node
                && let AreaModule::Blueprint(other) = &module.module
                && other.blueprint == placement.blueprint
            {
                count += 1;
            }
        });
        let times = match count {
            1 => "once".to_owned(),
            2 => "twice".to_owned(),
            n => format!("{n} times"),
        };
        format!(
            "Places {blueprint}, placed {times} here.\nDouble-click edits it in place; {} opens the blueprint.",
            EditorKey::OpenBlueprint.name()
        )
    }

    fn count_inside(&self, id: &str) -> usize {
        let Some(node) = self.snapshot.scene_node(id) else {
            return 0;
        };
        let mut count = 0;
        nodes::walk(nodes::children_of(node), &mut |_, child| {
            count += usize::from(!nodes::is_folder(child));
        });
        count
    }

    fn label_of(&self, id: &str) -> String {
        self.snapshot
            .scene_node(id)
            .map(|node| shapes::node_label(node, self.snapshot.reader.as_ref()))
            .unwrap_or_default()
    }

    fn what(&self, ids: &[String]) -> String {
        match ids {
            [one] => self.label_of(one),
            many => format!("{} rows", many.len()),
        }
    }

    fn module_menu(&self, ui: &mut Ui, id: &str, node: &SceneNode) -> Option<SceneAction> {
        let one = || vec![id.to_owned()];
        let mut picked = None;
        let mut item = |ui: &mut Ui, text: &str, key: Option<EditorKey>, action: SceneAction| {
            let clicked = match key {
                Some(key) => ui.add(widgets::shortcut_button(text, key)).clicked(),
                None => ui.button(text).clicked(),
            };
            if clicked {
                picked = Some(action);
                ui.close();
            }
        };
        let placement = nodes::placement_of(node).is_some();
        let opened = self.in_place.iter().any(|place| place == id);
        if opened {
            item(
                ui,
                "Done editing",
                Some(EditorKey::Cancel),
                SceneAction::DoneEditing,
            );
        }
        item(
            ui,
            "Rename",
            Some(EditorKey::Rename),
            SceneAction::Rename(id.to_owned()),
        );
        item(
            ui,
            "Show on the map",
            Some(EditorKey::Confirm),
            SceneAction::ShowOnMap(id.to_owned()),
        );
        if placement {
            item(
                ui,
                "Open blueprint",
                Some(EditorKey::OpenBlueprint),
                SceneAction::OpenBlueprint(id.to_owned()),
            );
            if !opened {
                item(
                    ui,
                    "Edit in place",
                    None,
                    SceneAction::EditInPlace(id.to_owned()),
                );
            }
            item(
                ui,
                "Unpack into a folder",
                None,
                SceneAction::Unpack(id.to_owned()),
            );
            item(
                ui,
                "Replace with…",
                None,
                SceneAction::Replace(id.to_owned()),
            );
            ui.separator();
        }
        item(ui, "Move to folder…", None, SceneAction::MoveTo(one()));
        item(
            ui,
            "Group into folder",
            Some(EditorKey::Group),
            SceneAction::Group(one()),
        );
        ui.separator();
        item(
            ui,
            "Hide",
            Some(EditorKey::Hide),
            SceneAction::Toggle(one(), Toggle::Hide),
        );
        item(
            ui,
            "Lock",
            Some(EditorKey::Lock),
            SceneAction::Toggle(one(), Toggle::Lock),
        );
        ui.separator();
        item(ui, "Copy", Some(EditorKey::Copy), SceneAction::Copy(one()));
        item(ui, "Cut", Some(EditorKey::Cut), SceneAction::Cut(one()));
        item(
            ui,
            "Duplicate",
            Some(EditorKey::Duplicate),
            SceneAction::Duplicate(one()),
        );
        if !placement {
            item(
                ui,
                "Make blueprint…",
                Some(EditorKey::MakeBlueprint),
                SceneAction::MakeBlueprint(one()),
            );
            ui.separator();
            let module = match node {
                SceneNode::Module(module) => Some(&module.module),
                _ => None,
            };
            let door = matches!(module, Some(AreaModule::Object(object))
                if !super::placement::warp_destinations(object, self.snapshot.reader.as_ref()).is_empty());
            if ui
                .add_enabled(door, egui::Button::new("Return door…"))
                .clicked()
            {
                picked = Some(SceneAction::ReturnDoor(id.to_owned()));
                ui.close();
            }
        }
        ui.separator();
        let mut delete = |ui: &mut Ui| {
            if ui
                .add(widgets::shortcut_button("Delete", EditorKey::Delete))
                .clicked()
            {
                picked = Some(SceneAction::Delete(one()));
                ui.close();
            }
        };
        delete(ui);
        picked
    }

    fn folder_menu(&self, ui: &mut Ui, id: &str) -> Option<SceneAction> {
        let one = || vec![id.to_owned()];
        let mut picked = None;
        menu::submenu(ui, "Add", |ui| {
            if let Some(action) = self.add_menu(ui, id) {
                picked = Some(action);
            }
        });
        let mut item = |ui: &mut Ui, text: &str, key: Option<EditorKey>, action: SceneAction| {
            let clicked = match key {
                Some(key) => ui.add(widgets::shortcut_button(text, key)).clicked(),
                None => ui.button(text).clicked(),
            };
            if clicked {
                picked = Some(action);
                ui.close();
            }
        };
        ui.separator();
        item(
            ui,
            "Rename",
            Some(EditorKey::Rename),
            SceneAction::Rename(id.to_owned()),
        );
        item(
            ui,
            "Group into folder",
            Some(EditorKey::Group),
            SceneAction::Group(one()),
        );
        item(
            ui,
            "Ungroup",
            Some(EditorKey::Ungroup),
            SceneAction::Ungroup(one()),
        );
        item(ui, "Move to folder…", None, SceneAction::MoveTo(one()));
        ui.separator();
        item(
            ui,
            "Hide",
            Some(EditorKey::Hide),
            SceneAction::Toggle(one(), Toggle::Hide),
        );
        item(
            ui,
            "Lock",
            Some(EditorKey::Lock),
            SceneAction::Toggle(one(), Toggle::Lock),
        );
        item(
            ui,
            "Show only this",
            Some(EditorKey::ShowOnly),
            SceneAction::ShowOnly(id.to_owned()),
        );
        ui.separator();
        item(ui, "Copy", Some(EditorKey::Copy), SceneAction::Copy(one()));
        item(ui, "Cut", Some(EditorKey::Cut), SceneAction::Cut(one()));
        if self.can_paste {
            item(
                ui,
                "Paste into",
                Some(EditorKey::Paste),
                SceneAction::Paste(id.to_owned()),
            );
        }
        item(
            ui,
            "Duplicate",
            Some(EditorKey::Duplicate),
            SceneAction::Duplicate(one()),
        );
        item(
            ui,
            "Make blueprint…",
            Some(EditorKey::MakeBlueprint),
            SceneAction::MakeBlueprint(one()),
        );
        ui.separator();
        let count = self.count_inside(id);
        item(
            ui,
            &format!(
                "Delete folder and its {count} module{}",
                words::plural(count)
            ),
            Some(EditorKey::Delete),
            SceneAction::Delete(one()),
        );
        picked
    }

    fn many_menu(&self, ui: &mut Ui, ids: &[String]) -> Option<SceneAction> {
        let all = || ids.to_vec();
        let mut picked = None;
        let mut item = |ui: &mut Ui, text: &str, key: Option<EditorKey>, action: SceneAction| {
            let clicked = match key {
                Some(key) => ui.add(widgets::shortcut_button(text, key)).clicked(),
                None => ui.button(text).clicked(),
            };
            if clicked {
                picked = Some(action);
                ui.close();
            }
        };
        ui.label(theme::muted(format!("{} rows", ids.len())).size(12.0));
        item(
            ui,
            "Group into folder",
            Some(EditorKey::Group),
            SceneAction::Group(all()),
        );
        item(ui, "Move to folder…", None, SceneAction::MoveTo(all()));
        ui.separator();
        item(
            ui,
            "Hide",
            Some(EditorKey::Hide),
            SceneAction::Toggle(all(), Toggle::Hide),
        );
        item(
            ui,
            "Lock",
            Some(EditorKey::Lock),
            SceneAction::Toggle(all(), Toggle::Lock),
        );
        ui.separator();
        item(ui, "Copy", Some(EditorKey::Copy), SceneAction::Copy(all()));
        item(ui, "Cut", Some(EditorKey::Cut), SceneAction::Cut(all()));
        item(
            ui,
            "Duplicate",
            Some(EditorKey::Duplicate),
            SceneAction::Duplicate(all()),
        );
        item(
            ui,
            "Make blueprint…",
            Some(EditorKey::MakeBlueprint),
            SceneAction::MakeBlueprint(all()),
        );
        ui.separator();
        item(
            ui,
            &format!("Delete {} rows", ids.len()),
            Some(EditorKey::Delete),
            SceneAction::Delete(all()),
        );
        picked
    }

    fn empty_menu(&self, ui: &mut Ui) -> Option<SceneAction> {
        let folder = self.top();
        let mut picked = None;
        menu::submenu(ui, "Add", |ui| {
            if let Some(action) = self.add_menu(ui, &folder) {
                picked = Some(action);
            }
        });
        let mut item = |ui: &mut Ui, text: &str, key: Option<EditorKey>, action: SceneAction| {
            let clicked = match key {
                Some(key) => ui.add(widgets::shortcut_button(text, key)).clicked(),
                None => ui.button(text).clicked(),
            };
            if clicked {
                picked = Some(action);
                ui.close();
            }
        };
        item(
            ui,
            "New folder",
            Some(EditorKey::NewFolder),
            SceneAction::NewFolder(folder.clone()),
        );
        if self.can_paste {
            item(
                ui,
                "Paste",
                Some(EditorKey::Paste),
                SceneAction::Paste(folder.clone()),
            );
        }
        ui.separator();
        item(ui, "Open all", None, SceneAction::OpenAll);
        item(ui, "Close all", None, SceneAction::CloseAll);
        ui.separator();
        item(
            ui,
            "Show everything",
            Some(EditorKey::ShowEverything),
            SceneAction::ShowEverything,
        );
        item(
            ui,
            "Unlock everything",
            Some(EditorKey::UnlockEverything),
            SceneAction::UnlockEverything,
        );
        picked
    }

    /// The row whose scene empty space stands for: the placement edited in place, or the top.
    fn top(&self) -> String {
        self.in_place.last().cloned().unwrap_or_default()
    }

    /// The Add menu into `folder`: a new folder, a ground layer, then the map's own groups of
    /// module kinds.
    pub fn add_menu(&self, ui: &mut Ui, folder: &str) -> Option<SceneAction> {
        add_menu(ui, folder, self.snapshot)
    }

    /// The scene a row belongs to: the bundle's own at level 0, then the blueprint of each
    /// placement edited in place.
    fn level(id: &str) -> usize {
        id.matches("/in/").count()
    }
}

impl TreeSource for SceneSource<'_> {
    type Action = SceneAction;

    fn children(&self, id: &str) -> Vec<String> {
        let all = self.all_children(id);
        match &self.narrowed {
            Some(narrowed) => all
                .into_iter()
                .filter(|child| narrowed.shown.contains(child))
                .collect(),
            None => all,
        }
    }

    fn parent(&self, id: &str) -> String {
        parent_id(id)
    }

    fn is_folder(&self, id: &str) -> bool {
        self.in_place.iter().any(|place| place == id)
            || self.snapshot.scene_node(id).is_some_and(nodes::is_folder)
    }

    fn row(&self, id: &str) -> TreeRow {
        let Some(node) = self.snapshot.scene_node(id) else {
            return self.waiting(id).unwrap_or_default();
        };
        let flags = self.snapshot.flags.row(id);
        let state = |flag: Flag| match flag {
            Flag::Off => ToggleState::Off,
            Flag::Own => ToggleState::Own,
            Flag::Inherited => ToggleState::Inherited,
        };
        let name = self.label_of(id);
        let kind = nodes::kind_tag(node);
        let folder = self.is_folder(id);
        let mut row = TreeRow {
            folder,
            name: name.clone(),
            toggles: Some(RowToggles {
                hidden: state(flags.hidden),
                locked: state(flags.locked),
                disabled: self.read_only.is_some(),
            }),
            dim: flags.hidden.on()
                || self.outside(id)
                || self
                    .narrowed
                    .as_ref()
                    .is_some_and(|narrowed| !narrowed.hits.contains(id)),
            problem: self.problems.get(id).cloned(),
            holds_selected: self.holding.contains(id),
            lit: self.lit.as_deref() == Some(id),
            matched: tree::matched(&name, &self.needle),
            ..TreeRow::default()
        };
        if nodes::is_folder(node) {
            let count = self.count_inside(id);
            row.detail = count.to_string();
            row.spoken = Some(format!("{name}, folder, {count} inside"));
        } else if let SceneNode::Module(module) = node {
            row.badge = Some((widgets::badge_letter(kind), widgets::badge_color(kind)));
            row.detail = shapes::node_detail(&module.module, self.snapshot.reader.as_ref());
            row.spoken = Some(format!("{name}, {}", kind.replace('_', " ")));
            if let AreaModule::Blueprint(placement) = &module.module {
                row.tip = Some(self.placement_tip(id, placement));
            }
        }
        if folder && !nodes::is_folder(node) {
            row.icon = None;
        }
        if let Some(reason) = &self.read_only {
            row.tip = Some(reason.clone());
        }
        row
    }

    fn menu(&self, ui: &mut Ui, ids: &[String]) -> Option<SceneAction> {
        if self.read_only.is_some() {
            ui.label(theme::muted("The game folder is open in another editor").size(12.5));
            return None;
        }
        match ids {
            [] => self.empty_menu(ui),
            [one] => {
                let node = self.snapshot.scene_node(one)?.clone();
                if self.outside(one) && !self.in_place.contains(one) {
                    ui.label(theme::muted("Done editing in place to change this").size(12.5));
                    return None;
                }
                if nodes::is_folder(&node) {
                    self.folder_menu(ui, one)
                } else {
                    self.module_menu(ui, one, &node)
                }
            }
            many => self.many_menu(ui, many),
        }
    }

    fn drop_tip(&self, dragged: &[String], drop: &TreeDrop) -> Result<String, String> {
        if let Some(reason) = &self.read_only {
            return Err(reason.clone());
        }
        if dragged
            .iter()
            .any(|id| self.outside(id) || self.in_place.contains(id))
        {
            return Err("Done editing in place to move this".to_owned());
        }
        let level = match self.in_place.iter().position(|place| *place == drop.folder) {
            Some(depth) => depth + 1,
            None => Self::level(&drop.folder),
        };
        if dragged.iter().any(|id| Self::level(id) != level) {
            return Err("Rows move within their own scene".to_owned());
        }
        if let Some(onto) = &drop.onto
            && self.is_placement(onto)
            && !self.in_place.contains(onto)
        {
            return Err(
                "A placement holds nothing of its own: double-click it to edit its blueprint in place"
                    .to_owned(),
            );
        }
        if dragged
            .iter()
            .any(|id| drop.folder == *id || drop.folder.starts_with(&format!("{id}/")))
        {
            return Err("A folder can't go inside itself".to_owned());
        }
        let below = local(&drop.folder);
        let folder_depth = match below {
            "" => 0,
            folder => nodes::depth(folder) + 1,
        };
        let deepest = dragged
            .iter()
            .filter_map(|id| self.snapshot.scene_node(id))
            .map(|node| usize::from(nodes::is_folder(node)) + nodes::folders_inside(node))
            .max()
            .unwrap_or(0);
        if folder_depth + deepest > MAX_FOLDER_DEPTH {
            return Err(format!("Folders nest at most {MAX_FOLDER_DEPTH} deep"));
        }
        let into = match drop.folder.as_str() {
            "" => "the top level".to_owned(),
            folder => self.label_of(folder),
        };
        Ok(format!("Move {} into {into}", self.what(dragged)))
    }

    fn ordered(&self) -> bool {
        true
    }

    fn narrowed(&self) -> bool {
        self.narrowed.is_some()
    }

    fn carried(&self, _: &[String]) -> Option<Dragged> {
        None
    }

    fn rename_tip(&self, _: &str) -> Option<String> {
        None
    }
}

/// The Add menu into `folder` of a scene: a new folder, then the map's groups of module kinds. In
/// a blueprint, the area-wide group runs once per placement.
pub fn add_menu(ui: &mut Ui, folder: &str, snapshot: &AreaSnapshot) -> Option<SceneAction> {
    let mut picked = None;
    ui.set_min_width(220.0);
    ui.label(theme::faint("ADD").size(11.0));
    if ui
        .add(widgets::shortcut_button("New folder", EditorKey::NewFolder))
        .clicked()
    {
        picked = Some(SceneAction::NewFolder(folder.to_owned()));
        ui.close();
    }
    let blueprint =
        matches!(snapshot.pictured, Pictured::Blueprint(_)) || !snapshot.in_place.is_empty();
    for (group, kinds) in ADD_MENU {
        let group = match *group {
            "Area-wide" if blueprint => "Per placement",
            group => group,
        };
        ui.label(theme::faint(group.to_uppercase()).size(11.0));
        for &(kind, description) in *kinds {
            if super::modules::module_menu_row(ui, kind.tag(), description) {
                picked = Some(SceneAction::Add {
                    folder: folder.to_owned(),
                    kind,
                });
                ui.close();
            }
        }
    }
    picked
}

/// The row of the node holding the row `id`, "" at the top.
pub fn parent_id(id: &str) -> String {
    let children = id.rfind("/children/");
    let opened = id.rfind("/in/");
    match (children, opened) {
        (Some(c), Some(o)) if c > o => id[..c].to_owned(),
        (Some(c), None) => id[..c].to_owned(),
        (_, Some(o)) => id[..o].to_owned(),
        (None, None) => String::new(),
    }
}

/// A row's node pointer within its own scene.
pub fn local(id: &str) -> &str {
    match id.rfind("/in/") {
        Some(at) => &id[at + 3..],
        None => id,
    }
}

/// The rows of the placements edited in place along `chain`, joined into the innermost's row.
fn place_ids(chain: &[super::placement::AreaPlacement]) -> String {
    chain
        .iter()
        .map(|place| place.node.as_str())
        .collect::<Vec<_>>()
        .join("/in")
}

/// The rows the editor's selection names in the tree of `snapshot`.
pub fn selected_ids(snapshot: &AreaSnapshot, selection: &RowSelection) -> Vec<String> {
    let Some(key) = selection.keys.first() else {
        return Vec::new();
    };
    if *key == snapshot.bundle {
        return selection.parts.clone();
    }
    match snapshot.in_place.last() {
        Some(place) if place.blueprint == *key => {
            let prefix = place_ids(&snapshot.in_place);
            selection
                .parts
                .iter()
                .map(|part| format!("{prefix}/in{part}"))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The selection the rows `ids` of the tree of `snapshot` make: rows of one scene, the scene of the
/// first.
pub fn selection_of(snapshot: &AreaSnapshot, ids: &[String]) -> RowSelection {
    let level = ids.first().map_or(0, |id| SceneSource::level(id));
    let parts: Vec<String> = ids
        .iter()
        .filter(|id| SceneSource::level(id) == level)
        .map(|id| local(id).to_owned())
        .collect();
    let bundle = match level {
        0 => snapshot.bundle.clone(),
        depth => snapshot
            .in_place
            .get(depth - 1)
            .map_or_else(|| snapshot.bundle.clone(), |place| place.blueprint.clone()),
    };
    RowSelection::parts(table_of(&bundle).unwrap_or_default(), &bundle, parts)
}

/// The bundle whose scene holds the rows at `level`.
fn bundle_at(snapshot: &AreaSnapshot, level: usize) -> String {
    match level {
        0 => snapshot.bundle.clone(),
        depth => snapshot
            .in_place
            .get(depth - 1)
            .map_or_else(|| snapshot.bundle.clone(), |place| place.blueprint.clone()),
    }
}

/// Problems the loader found in the scene, on the rows of the nodes they're in and on every
/// folder above, which counts them; a placement also carries its blueprint's own.
fn problems(project: &Project, snapshot: &AreaSnapshot) -> HashMap<String, String> {
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    let scene = scene_file(&snapshot.bundle);
    let mut broken_bundles: HashMap<String, String> = HashMap::new();
    for error in project.report.iter().flat_map(|report| &report.errors) {
        let text = error.problem.to_string();
        if error.file == scene {
            let ErrorPlace::Json { pointer, .. } = &error.at else {
                continue;
            };
            let node = scene_graph::node_pointer(pointer);
            if !node.is_empty() {
                found.entry(node).or_default().push(text);
            }
        } else if let Some(bundle) = owning_bundle(&error.file) {
            broken_bundles.entry(bundle).or_insert(text);
        }
    }
    nodes::walk(snapshot.nodes(), &mut |pointer, node| {
        if let Some(placement) = nodes::placement_of(node)
            && let Some(blueprint) = snapshot.reader.path_of(placement.blueprint)
            && let Some(text) = broken_bundles.get(blueprint)
        {
            found
                .entry(pointer.to_owned())
                .or_default()
                .push(format!("its blueprint doesn't load: {text}"));
        }
    });
    let mut out: HashMap<String, String> = HashMap::new();
    let mut inside: HashMap<String, usize> = HashMap::new();
    for (pointer, texts) in &found {
        out.insert(pointer.clone(), texts.join("; "));
        let mut parent = parent_id(pointer);
        while !parent.is_empty() {
            *inside.entry(parent.clone()).or_default() += texts.len();
            parent = parent_id(&parent);
        }
    }
    for (folder, count) in inside {
        out.entry(folder)
            .or_insert_with(|| format!("{count} problem{} inside", words::plural(count)));
    }
    out
}

/// The area or blueprint bundle a file of the game folder sits in, when it sits in one.
fn owning_bundle(file: &str) -> Option<String> {
    let mut at = 0;
    while let Some(slash) = file[at..].find('/') {
        let folder = &file[..at + slash];
        if matches!(
            table_of(folder),
            Some(BlueprintDef::TABLE | engine::systems::area::AreaDef::TABLE)
        ) {
            return Some(folder.to_owned());
        }
        at += slash + 1;
    }
    None
}

/// Turns `toggle` over on the rows `ids` of `bundle`'s scene as one step: on for all unless all
/// have it on already.
pub fn toggle(editor: &mut Editor, bundle: &str, ids: &[String], toggle: Toggle) {
    let flag = flag_of(toggle);
    let pointers: Vec<String> = ids.iter().map(|id| local(id).to_owned()).collect();
    let Some(reader) = editor.reader() else {
        return;
    };
    nodes::change_scene(editor, bundle, |scene| {
        let all_on = pointers
            .iter()
            .all(|pointer| nodes::node(scene, pointer).is_some_and(|node| flag.of(node)));
        nodes::SceneTree::new(scene).set_flag(&pointers, flag, !all_on);
        let verb = match (toggle, all_on) {
            (Toggle::Hide, false) => "Hide",
            (Toggle::Hide, true) => "Show",
            (Toggle::Lock, false) => "Lock",
            (Toggle::Lock, true) => "Unlock",
        };
        let what = match pointers.as_slice() {
            [one] => shapes::node_label(nodes::node(scene, one)?, reader.as_ref()),
            many => format!("{} rows", many.len()),
        };
        Some(format!("{verb} {what}"))
    });
}

/// Clears `toggle` on every node of `bundle`'s scene as one step.
pub fn clear_all(editor: &mut Editor, bundle: &str, toggle: Toggle) {
    let verb = match toggle {
        Toggle::Hide => "Show",
        Toggle::Lock => "Unlock",
    };
    let mut cleared = 0;
    nodes::change_scene(editor, bundle, |scene| {
        cleared = nodes::SceneTree::new(scene).clear_flag(flag_of(toggle));
        (cleared > 0).then(|| format!("{verb} everything in {}", file_title(bundle)))
    });
    if cleared == 0 {
        editor.toast(format!(
            "Nothing in {} to {}",
            file_title(bundle),
            verb.to_lowercase()
        ));
    }
}

/// Puts the rows `ids` into a new folder where the first was, as one step; answers its row and
/// the step's label.
pub fn group(
    editor: &mut Editor,
    snapshot: &AreaSnapshot,
    ids: &[String],
) -> Option<(String, String)> {
    let level = SceneSource::level(ids.first()?);
    let bundle = bundle_at(snapshot, level);
    let pointers: Vec<String> = ids
        .iter()
        .filter(|id| SceneSource::level(id) == level)
        .map(|id| local(id).to_owned())
        .collect();
    let mut made = None;
    let mut remap = Remap::new();
    let mut step = String::new();
    nodes::change_scene(editor, &bundle, |scene| {
        let (folder, moved) = nodes::restructure(scene, |tree| tree.group(&pointers, NEW_FOLDER));
        remap = moved;
        let folder = folder?;
        let label = nodes::node(scene, &folder)?.label()?.to_owned();
        made = Some(folder);
        step = group_step(pointers.len(), &label);
        Some(step.clone())
    });
    let folder = made?;
    follow(editor, snapshot, level, &remap);
    let prefix = level_prefix(snapshot, level);
    nodes::select(editor, &bundle, vec![folder.clone()]);
    Some((format!("{prefix}{folder}"), step))
}

fn group_step(count: usize, label: &str) -> String {
    let what = match count {
        1 => "1 row".to_owned(),
        n => format!("{n} rows"),
    };
    format!("Group {what} into {label}")
}

/// Names the folder a grouping just made, folded into the grouping's own step.
pub fn rename_grouped(
    editor: &mut Editor,
    snapshot: &AreaSnapshot,
    id: &str,
    typed: &str,
    step: &str,
) {
    let typed = typed.trim();
    let level = SceneSource::level(id);
    let bundle = bundle_at(snapshot, level);
    let pointer = local(id).to_owned();
    let Some(mut edit) = nodes::scene_edit(editor, &bundle, |scene| {
        let (folder, index) = nodes::place_of(&pointer)?;
        let siblings = nodes::children_mut(scene, folder)?;
        let others: Vec<SceneNode> = siblings
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != index)
            .map(|(_, sibling)| sibling.clone())
            .collect();
        let label = nodes::free_label(typed, &others);
        let node = siblings.get_mut(index)?;
        let count = nodes::children_of(node).len();
        nodes::set_label(node, label.clone());
        Some(group_step(count, &label))
    }) else {
        return;
    };
    if typed.is_empty() {
        return;
    }
    edit = edit.amending(step);
    editor.apply(edit);
}

/// Takes the folders `ids` away, what each held taking its place, as one step.
pub fn ungroup(editor: &mut Editor, bundle: &str, ids: &[String]) {
    let pointers: Vec<String> = ids.iter().map(|id| local(id).to_owned()).collect();
    let mut remap = Remap::new();
    let Some(reader) = editor.reader() else {
        return;
    };
    nodes::change_scene(editor, bundle, |scene| {
        let labels: Vec<String> = pointers
            .iter()
            .filter_map(|pointer| nodes::node(scene, pointer))
            .filter(|node| nodes::is_folder(node))
            .map(|node| shapes::node_label(node, reader.as_ref()))
            .collect();
        if labels.is_empty() {
            return None;
        }
        let ((), moved) = nodes::restructure(scene, |tree| tree.ungroup(&pointers));
        remap = moved;
        Some(format!("Ungroup {}", labels.join(", ")))
    });
    let selected: Vec<String> = editor
        .selection
        .parts
        .iter()
        .filter_map(|part| nodes::remapped(&remap, part))
        .collect();
    if editor.selection.keys.first().map(String::as_str) == Some(bundle) {
        editor.selection.parts = selected;
    }
}

pub const NEW_FOLDER: &str = "New folder";

/// Moves the rows `ids` into the folder row `folder` at `at`, as one step, the selection and open
/// folders following them.
pub fn move_rows(
    editor: &mut Editor,
    snapshot: &AreaSnapshot,
    ids: &[String],
    folder: &str,
    at: &DropAt,
) -> Result<(), String> {
    let level = ids.first().map_or(0, |id| SceneSource::level(id));
    let bundle = bundle_at(snapshot, level);
    let pointers: Vec<String> = ids.iter().map(|id| local(id).to_owned()).collect();
    let into = if folder.is_empty()
        || !folder.contains('/')
        || folder_is_level_top(snapshot, folder, level)
    {
        String::new()
    } else {
        local(folder).to_owned()
    };
    let landing = match at {
        DropAt::First => Landing::First,
        DropAt::End => Landing::End,
        DropAt::Before(id) => Landing::Before(local(id).to_owned()),
        DropAt::After(id) => Landing::After(local(id).to_owned()),
    };
    let reader = snapshot.reader.clone();
    let into_label = match folder {
        "" => "the top level".to_owned(),
        folder => snapshot
            .scene_node(folder)
            .map(|node| shapes::node_label(node, reader.as_ref()))
            .unwrap_or_default(),
    };
    let mut remap = Remap::new();
    let mut landed = Vec::new();
    let applied = nodes::change_scene(editor, &bundle, |scene| {
        let what = match pointers.as_slice() {
            [one] => shapes::node_label(nodes::node(scene, one)?, reader.as_ref()),
            many => format!("{} rows", many.len()),
        };
        let (moved, map) =
            nodes::restructure(scene, |tree| tree.move_nodes(&pointers, &into, &landing));
        landed = moved?;
        remap = map;
        Some(format!("Move {what} into {into_label}"))
    });
    if !applied {
        return Err("Nothing moved".to_owned());
    }
    follow(editor, snapshot, level, &remap);
    editor.selection = RowSelection::parts(table_of(&bundle).unwrap_or_default(), &bundle, landed);
    Ok(())
}

fn folder_is_level_top(snapshot: &AreaSnapshot, folder: &str, level: usize) -> bool {
    level > 0 && place_ids(&snapshot.in_place[..level]) == folder
}

fn level_prefix(snapshot: &AreaSnapshot, level: usize) -> String {
    match level {
        0 => String::new(),
        depth => format!("{}/in", place_ids(&snapshot.in_place[..depth])),
    }
}

/// Keeps the open folders of the scene at `level` open where a restructure moved them.
fn follow(editor: &mut Editor, snapshot: &AreaSnapshot, level: usize, remap: &Remap) {
    let Some(project) = &editor.project else {
        return;
    };
    let key = tree_key(project, &snapshot.bundle);
    let prefix = level_prefix(snapshot, level);
    if let Some(view) = editor.settings.scene_trees.get_mut(&key) {
        view.moved(&|id: &str| {
            let pointer = id
                .strip_prefix(prefix.as_str())
                .filter(|_| SceneSource::level(id) == level)?;
            nodes::remapped(remap, pointer).map(|now| format!("{prefix}{now}"))
        });
    }
    let view_key = super::view_key(&snapshot.bundle);
    if let Some(view) = editor.views.area.views.get_mut(&view_key) {
        view.scene.tree.forget();
    }
}

/// The key a scene's open folders and Show choice are kept under, per user.
pub fn tree_key(project: &Project, bundle: &str) -> String {
    format!(
        "{}:{bundle}",
        crate::user::UserFolder::folder_key(&project.root)
    )
}

/// Adds a node of `kind` at the end of the folder row `folder`. One that needs a spot arms the map
/// with a ghost; one that doesn't is added at once and selected.
pub fn add(editor: &mut Editor, snapshot: &AreaSnapshot, folder: &str, kind: AreaModuleKind) {
    let level = match folder {
        "" => 0,
        folder if folder_is_place(snapshot, folder) => SceneSource::level(folder) + 1,
        folder => SceneSource::level(folder),
    };
    let bundle = bundle_at(snapshot, level);
    let into = if level > 0 && folder_is_place(snapshot, folder) {
        String::new()
    } else {
        local(folder).to_owned()
    };
    let parts = match into.as_str() {
        "" => Vec::new(),
        into => vec![into.to_owned()],
    };
    nodes::select(editor, &bundle, parts);
    let view_key = super::view_key(&snapshot.bundle);
    let needs_spot =
        !matches!(kind, AreaModuleKind::Script) && !(kind == AreaModuleKind::Population);
    if needs_spot {
        let view = editor.views.area.view_mut(&view_key);
        view.set_tool(super::AreaTool::Module);
        view.placing = Some(kind);
        editor.toast(format!(
            "Click the map to place the {} into {}; {} cancels",
            kind.tag().replace('_', " "),
            match into.as_str() {
                "" => "the top level".to_owned(),
                _ => snapshot
                    .scene_node(folder)
                    .map(|node| shapes::node_label(node, snapshot.reader.as_ref()))
                    .unwrap_or_default(),
            },
            EditorKey::Cancel.name()
        ));
        return;
    }
    super::pick::pick_then_add(editor, &bundle, kind, None);
}

fn folder_is_place(snapshot: &AreaSnapshot, id: &str) -> bool {
    is_place_row(snapshot, id)
}

/// Whether the row `id` is a placement edited in place, opened on its blueprint's scene.
pub fn is_place_row(snapshot: &AreaSnapshot, id: &str) -> bool {
    (1..=snapshot.in_place.len()).any(|depth| place_ids(&snapshot.in_place[..depth]) == id)
}

/// The row of the innermost placement edited in place, "" for none.
pub fn place_row(snapshot: &AreaSnapshot) -> String {
    match snapshot.in_place.len() {
        0 => String::new(),
        depth => place_ids(&snapshot.in_place[..depth]),
    }
}

/// The bundle whose scene holds the row `id`.
pub fn bundle_of_row(snapshot: &AreaSnapshot, id: &str) -> String {
    bundle_at(snapshot, SceneSource::level(id))
}

/// The pointer within its own scene of the folder row `folder` new rows go into: "" for the top
/// of a scene, the bundle's or a blueprint's edited in place.
pub fn folder_pointer(snapshot: &AreaSnapshot, folder: &str) -> String {
    if folder.is_empty() || is_place_row(snapshot, folder) {
        String::new()
    } else {
        local(folder).to_owned()
    }
}

/// What the ids of rows in the scene of `bundle` start with.
pub fn prefix_of(snapshot: &AreaSnapshot, bundle: &str) -> String {
    if bundle == snapshot.bundle {
        return String::new();
    }
    match snapshot
        .in_place
        .iter()
        .position(|place| place.blueprint == bundle)
    {
        Some(depth) => level_prefix(snapshot, depth + 1),
        None => String::new(),
    }
}

/// The rows directly inside the row `parent`, "" for the top, whatever narrows the tree.
pub fn rows_in(snapshot: &AreaSnapshot, parent: &str) -> Vec<String> {
    let (list, prefix): (&[SceneNode], String) = if parent.is_empty() {
        (snapshot.nodes(), String::new())
    } else if is_place_row(snapshot, parent) {
        let depth = SceneSource::level(parent);
        (&snapshot.in_place[depth].def.scene, format!("{parent}/in"))
    } else {
        match snapshot.scene_node(parent) {
            Some(SceneNode::Folder(folder)) => (&folder.children, format!("{parent}/children")),
            _ => return Vec::new(),
        }
    };
    (0..list.len())
        .map(|index| format!("{prefix}/{index}"))
        .collect()
}

/// The scene whose nodes are the rows at `level`: the bundle's own, then the blueprint of each
/// placement edited in place.
fn scene_at(snapshot: &AreaSnapshot, level: usize) -> &[SceneNode] {
    match level {
        0 => snapshot.nodes(),
        depth => &snapshot.in_place[depth - 1].def.scene,
    }
}

fn flag_of(toggle: Toggle) -> NodeFlag {
    match toggle {
        Toggle::Hide => NodeFlag::Hidden,
        Toggle::Lock => NodeFlag::Locked,
    }
}

/// Every row of the tree, depth first in order.
pub fn rows_in_order(snapshot: &AreaSnapshot) -> Vec<String> {
    fn inner(snapshot: &AreaSnapshot, parent: &str, out: &mut Vec<String>) {
        for row in rows_in(snapshot, parent) {
            out.push(row.clone());
            inner(snapshot, &row, out);
        }
    }
    let mut out = Vec::new();
    inner(snapshot, "", &mut out);
    out
}

/// The folder row a paste into `folder` lands in.
pub fn paste_target(snapshot: &AreaSnapshot, folder: &str) -> String {
    match snapshot.scene_node(folder) {
        Some(node) if nodes::is_folder(node) => folder.to_owned(),
        _ if is_place_row(snapshot, folder) => folder.to_owned(),
        Some(_) => parent_id(folder),
        None => place_row(snapshot),
    }
}

/// The topmost of the rows `ids` of one scene, by their pointers in it.
fn topmost(ids: &[String]) -> Vec<String> {
    let mut pointers: Vec<String> = ids.iter().map(|id| local(id).to_owned()).collect();
    nodes::sort_in_scene_order(&mut pointers);
    pointers.dedup();
    let all = pointers.clone();
    pointers.retain(|pointer| {
        !all.iter()
            .any(|other| other != pointer && pointer.starts_with(&format!("{other}/")))
    });
    pointers
}

/// Copies the rows `ids`, folders with all they hold, placed from the top-left of what they
/// cover.
pub fn copy_rows(editor: &mut Editor, snapshot: &AreaSnapshot, ids: &[String]) {
    let Some(first) = ids.first() else {
        return;
    };
    let bundle = bundle_of_row(snapshot, first);
    let scene = nodes::scene_of(editor, &bundle);
    let copied: Vec<SceneNode> = topmost(ids)
        .iter()
        .filter_map(|pointer| nodes::node(&scene, pointer).cloned())
        .collect();
    if copied.is_empty() {
        return;
    }
    let reader = snapshot.reader.clone();
    let origin = super::blueprint::covered(&snapshot.content, &reader, &copied)
        .map(|bounds| bounds.min())
        .unwrap_or(engine::core::math::Pos::new(0.0, 0.0));
    let shifted: Vec<SceneNode> = copied
        .into_iter()
        .map(|mut node| {
            super::blueprint::shift_tree(&mut node, -origin.x, -origin.y);
            node
        })
        .collect();
    let count = shifted.len();
    let Some(nodes) = nodes::CopiedNodes::of(&reader, &shifted) else {
        editor.toast("What's selected doesn't copy");
        return;
    };
    editor.clipboard = Some(Clipboard::Scene(SceneClip {
        from: bundle,
        nodes,
        count,
        origin,
    }));
    editor.toast(format!("Copied {count} row{}", words::plural(count)));
}

/// Pastes what was copied into the folder row `folder`, where it was copied from.
pub fn paste_rows(editor: &mut Editor, snapshot: &AreaSnapshot, folder: &str) {
    let Some(clip) = editor
        .clipboard
        .as_ref()
        .and_then(Clipboard::scene)
        .cloned()
    else {
        editor.toast("Nothing copied");
        return;
    };
    let folder = paste_target(snapshot, folder);
    let into = match folder.as_str() {
        "" => String::new(),
        folder => format!("{folder}/children/0"),
    };
    let bundle = if folder.is_empty() || is_place_row(snapshot, &folder) {
        bundle_at(
            snapshot,
            if folder.is_empty() {
                0
            } else {
                SceneSource::level(&folder) + 1
            },
        )
    } else {
        bundle_of_row(snapshot, &into)
    };
    let Some((mut nodes_in, open)) = super::select::pasted(editor, &clip, &bundle) else {
        return;
    };
    for node in &mut nodes_in {
        super::blueprint::shift_tree(node, clip.origin.x, clip.origin.y);
    }
    let parts = match folder_pointer(snapshot, &folder).as_str() {
        "" => Vec::new(),
        pointer => vec![pointer.to_owned()],
    };
    nodes::select(editor, &bundle, parts);
    let count = nodes_in.len();
    if nodes::add(
        editor,
        &bundle,
        nodes_in,
        &format!("Paste {count} row{}", words::plural(count)),
    ) && open > 0
    {
        super::select::toast_open(editor, open);
    }
}

/// Copies each of the rows `ids` right after itself, a module a metre down and right of its
/// original, as one step.
pub fn duplicate_rows(editor: &mut Editor, snapshot: &AreaSnapshot, ids: &[String]) {
    let Some(first) = ids.first() else {
        return;
    };
    let bundle = bundle_of_row(snapshot, first);
    let pointers = topmost(ids);
    let reader = snapshot.reader.clone();
    let mut taken: HashSet<String> = snapshot.object_names();
    let mut made = Vec::new();
    let applied = nodes::change_scene(editor, &bundle, |scene| {
        let label = match pointers.as_slice() {
            [one] => format!(
                "Duplicate {}",
                shapes::node_label(nodes::node(scene, one)?, reader.as_ref())
            ),
            many => format!("Duplicate {} rows", many.len()),
        };
        made = nodes::SceneTree::new(scene).duplicate(&pointers, |node| {
            let mut node = node.clone();
            super::blueprint::shift_tree(&mut node, 1.0, 1.0);
            rename_objects(&mut node, &mut taken);
            node
        });
        Some(label)
    });
    if applied {
        let folder = made
            .first()
            .and_then(|pointer| nodes::place_of(pointer))
            .map(|(folder, _)| folder.to_owned())
            .unwrap_or_default();
        let into = match folder.as_str() {
            "" => "the top level".to_owned(),
            folder => nodes::node(&nodes::scene_of(editor, &bundle), folder)
                .map(|node| shapes::node_label(node, reader.as_ref()))
                .unwrap_or_default(),
        };
        nodes::select(editor, &bundle, made);
        editor.toast(format!("Duplicated into {into}"));
    }
}

/// Gives every object inside `node` a name no other object of the bundle has.
pub(super) fn rename_objects(node: &mut SceneNode, taken: &mut HashSet<String>) {
    match node {
        SceneNode::Folder(folder) => {
            for child in &mut folder.children {
                rename_objects(child, taken);
            }
        }
        SceneNode::Module(module) => {
            if let AreaModule::Object(object) = &mut module.module
                && let Some(name) = &mut object.name
            {
                let fresh = crate::rows::free_name(name, taken);
                taken.insert(fresh.clone());
                *name = fresh;
            }
        }
    }
}

/// Deletes the rows `ids`, a folder with all it holds, as one step that says what went.
pub fn delete_rows(editor: &mut Editor, snapshot: &AreaSnapshot, ids: &[String]) {
    let Some(first) = ids.first() else {
        return;
    };
    let bundle = bundle_of_row(snapshot, first);
    let pointers = topmost(ids);
    let reader = snapshot.reader.clone();
    let Some(edit) = nodes::scene_edit(editor, &bundle, |scene| {
        let mut modules = 0;
        let mut folders = Vec::new();
        for pointer in &pointers {
            let node = nodes::node(scene, pointer)?;
            nodes::walk(std::slice::from_ref(node), &mut |_, inner| {
                modules += usize::from(matches!(inner, SceneNode::Module(_)));
            });
            if nodes::is_folder(node) {
                folders.push(shapes::node_label(node, reader.as_ref()));
            }
        }
        let label = match (pointers.as_slice(), folders.as_slice()) {
            ([_], [folder]) => format!(
                "Delete folder {folder} and its {modules} module{}",
                words::plural(modules)
            ),
            ([one], []) => format!(
                "Delete {}",
                shapes::node_label(nodes::node(scene, one)?, reader.as_ref())
            ),
            (many, _) => format!("Delete {} rows", many.len()),
        };
        nodes::SceneTree::new(scene).remove(&pointers);
        Some(label)
    }) else {
        return;
    };
    if editor.apply(edit) {
        editor.selection.parts.clear();
    }
}

/// Moves each of the rows `ids` one place up or down among its siblings, as one step.
pub fn reorder_rows(editor: &mut Editor, snapshot: &AreaSnapshot, ids: &[String], step: i32) {
    let Some(first) = ids.first() else {
        return;
    };
    let level = SceneSource::level(first);
    let bundle = bundle_at(snapshot, level);
    let pointers: Vec<String> = ids
        .iter()
        .filter(|id| SceneSource::level(id) == level)
        .map(|id| local(id).to_owned())
        .collect();
    let mut remap = Remap::new();
    let reader = snapshot.reader.clone();
    let applied = nodes::change_scene(editor, &bundle, |scene| {
        let what = match pointers.as_slice() {
            [one] => shapes::node_label(nodes::node(scene, one)?, reader.as_ref()),
            many => format!("{} rows", many.len()),
        };
        let ((), moved) = nodes::restructure(scene, |tree| tree.reorder(&pointers, step));
        if moved.iter().all(|(was, now)| was == now) {
            return None;
        }
        remap = moved;
        Some(format!(
            "Move {what} {}",
            if step < 0 { "up" } else { "down" }
        ))
    });
    if applied {
        follow(editor, snapshot, level, &remap);
        let parts = pointers
            .iter()
            .filter_map(|pointer| nodes::remapped(&remap, pointer))
            .collect();
        nodes::select(editor, &bundle, parts);
    }
}

/// Adds a folder labelled "New folder" at the end of the folder row `folder`, selected, its name
/// ready to type; answers its row.
pub fn new_folder(editor: &mut Editor, snapshot: &AreaSnapshot, folder: &str) -> Option<String> {
    let level = match folder {
        "" => 0,
        folder if folder_is_place(snapshot, folder) => SceneSource::level(folder) + 1,
        folder => SceneSource::level(folder),
    };
    let bundle = bundle_at(snapshot, level);
    let into = if folder_is_place(snapshot, folder) {
        String::new()
    } else {
        local(folder).to_owned()
    };
    let mut made = Vec::new();
    nodes::change_scene(editor, &bundle, |scene| {
        let siblings = match into.as_str() {
            "" => scene.clone(),
            into => nodes::children_of(nodes::node(scene, into)?).to_vec(),
        };
        let label = nodes::free_label(NEW_FOLDER, &siblings);
        made = nodes::SceneTree::new(scene).append(&into, vec![nodes::folder(label.clone())]);
        Some(format!("New folder {label}"))
    });
    let pointer = made.first()?.clone();
    nodes::select(editor, &bundle, vec![pointer.clone()]);
    Some(format!("{}{pointer}", level_prefix(snapshot, level)))
}

/// Writes the label typed for the row `id`: a folder's or a module's label, as one step.
pub fn rename(editor: &mut Editor, snapshot: &AreaSnapshot, id: &str, typed: &str) {
    let typed = typed.trim();
    let Some(node) = snapshot.scene_node(id).cloned() else {
        return;
    };
    let reader = snapshot.reader.clone();
    if typed.is_empty() || shapes::node_label(&node, reader.as_ref()) == typed {
        return;
    }
    let level = SceneSource::level(id);
    let bundle = bundle_at(snapshot, level);
    let pointer = local(id).to_owned();
    let mut refused = None;
    nodes::change_scene(editor, &bundle, |scene| {
        let (folder, index) = nodes::place_of(&pointer)?;
        let siblings = nodes::children_mut(scene, folder)?;
        let taken = siblings.iter().enumerate().any(|(at, sibling)| {
            at != index
                && sibling
                    .label()
                    .is_some_and(|label| label.eq_ignore_ascii_case(typed))
        });
        if taken {
            refused = Some(format!(
                "{typed} is taken here: labels are unique among siblings"
            ));
            return None;
        }
        let node = siblings.get_mut(index)?;
        let was = shapes::node_label(node, reader.as_ref());
        nodes::set_label(node, typed.to_owned());
        Some(format!("Rename {was} to {typed}"))
    });
    if let Some(refused) = refused {
        editor.toast(refused);
    }
}

/// Every folder row of the scene at `level` a row could move into, by its labels from the top, the
/// top itself first.
pub fn folders_of(snapshot: &AreaSnapshot, level: usize) -> Vec<(String, String)> {
    let scene = scene_at(snapshot, level);
    let prefix = level_prefix(snapshot, level);
    let mut out = vec![(
        "(top level)".to_owned(),
        match level {
            0 => String::new(),
            depth => place_ids(&snapshot.in_place[..depth]),
        },
    )];
    let mut labels: HashMap<String, String> = HashMap::new();
    nodes::walk(scene, &mut |pointer, node| {
        if nodes::is_folder(node) {
            let (parent, _) = nodes::place_of(pointer).unwrap_or(("", 0));
            let above = labels.get(parent).cloned();
            let label = shapes::node_label(node, snapshot.reader.as_ref());
            let path = match above {
                Some(above) => format!("{above} › {label}"),
                None => label,
            };
            labels.insert(pointer.to_owned(), path.clone());
            out.push((path, format!("{prefix}{pointer}")));
        }
    });
    out
}

/// Writes `edit` and, when it lands, selects `parts` of `bundle`.
pub fn apply_selecting(editor: &mut Editor, edit: ContentEdit, bundle: &str, parts: Vec<String>) {
    if editor.apply(edit) {
        nodes::select(editor, bundle, parts);
    }
}
