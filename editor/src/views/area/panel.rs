use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use ui::chip;
use ui::egui;
use ui::form;
use ui::panel;
use ui::theme;
use ui::words;

use engine::core::content::{ContentFile, name_of};
use engine::core::script::Variable;
use engine::field;
use engine::systems::area::blueprint::{BlueprintDef, Placement};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{AreaModule, AreaModuleKind, Pictured};
use engine::systems::script::RiftScripting;
use strum::VariantArray;
use ui::controls;
use ui::egui::{Align, Layout, Ui};
use ui::glyph::{self, Mark};

use super::nodes::SceneBundle;
use super::scene_tree::{self, SceneAction, SceneNarrowing, SceneSource, Toggle};
use super::{AreaSnapshot, nodes, placement, resize, shapes};
use crate::app::{Editor, ModalDialog, PickRowDialog, PickRowThen};
use crate::forms::{self, FormCx};
use crate::keymap::EditorKey;
use crate::project::files::content_of;
use crate::shell::tree::{self, DropAt, TreeSource, TreeState};
use crate::shell::widgets::{self, KindMark};
use crate::shell::{breadcrumb, content_tree, row_form};
use crate::tabs::{EditorTab, file_title};
use crate::views;
use ui::input::{self, InputField};

/// The left panel's own state between frames: the scene graph's tree, its filter, and what was
/// cut from it.
#[derive(Default)]
pub struct ScenePanel {
    pub tree: TreeState,
    pub filter: String,
    pub focus_filter: bool,
    /// Rows cut with Ctrl X, moved into the selected folder by the next paste.
    pub cut: Vec<String>,
    /// The rows the tree last showed selected, to tell a pick on the map from one in the tree.
    shown: Vec<String>,
    /// What the filter and Show last left of the scene.
    narrowed: Option<Arc<scene_tree::Narrowed>>,
    /// What another part of the view asked of the scene graph, done on its next frame.
    pub queued: Option<SceneAction>,
    /// A folder just made by grouping, with its step, so naming it joins that step.
    grouped: Option<(String, String)>,
    typed: String,
    typed_at: f64,
}

/// The view's left panel: where the bundle is, its boxes, then its scene graph.
pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let bundle = editor.views.area.view(view_key).bundle.clone();
    if let Some(folder) = breadcrumb::show(ui, &bundle) {
        content_tree::reveal(editor, &folder);
    }
    ui.add_space(4.0);
    if let Some(action) = editor.views.area.view_mut(view_key).scene.queued.take()
        && let Some(snapshot) = AreaSnapshot::for_view(editor, view_key)
    {
        act(editor, view_key, &snapshot, action);
    }
    let Some(snapshot) = AreaSnapshot::for_view(editor, view_key) else {
        ui.label(theme::muted("This bundle doesn't load; Output says why.").size(12.5));
        return;
    };
    boxes(editor, ui, &snapshot);
    ui.add_space(8.0);
    header(editor, ui, view_key, &snapshot);
    ui.add_space(4.0);
    filter_row(editor, ui, view_key);
    ui.add_space(4.0);
    scene_graph(editor, ui, view_key, &snapshot);
}

fn boxes(editor: &mut Editor, ui: &mut Ui, snapshot: &AreaSnapshot) {
    let bundle = snapshot.bundle.clone();
    let def = snapshot.def.clone();
    let folded = editor.settings.boxes_folded;
    let size = snapshot.size();
    let facts = format!("{} × {} m", size.width, size.height);
    let area = matches!(snapshot.pictured, Pictured::Area(_));
    let title = match &def {
        SceneBundle::Area(area) => area.name.clone(),
        SceneBundle::Blueprint(_) => name_of(&bundle).to_owned(),
    };
    let mut fold = false;
    let mut menu = None;
    let mut renamed = None;
    let mut variables_changed = None;
    let mut actions = Vec::new();
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let mark = if folded {
                Mark::CaretRight
            } else {
                Mark::CaretDown
            };
            if glyph::button(ui, mark, 11.0, theme::MUTED, "Fold the box").clicked() {
                fold = true;
            }
            ui.label(theme::strong(&title));
            if folded {
                ui.label(theme::muted(&facts).size(12.5));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
                    if ui.button("Resize…").clicked() {
                        menu = Some(BoxMenu::Resize);
                        ui.close();
                    }
                    if area
                        && ui
                            .add(widgets::shortcut_button("Play here", EditorKey::PlayHere))
                            .clicked()
                    {
                        menu = Some(BoxMenu::Play);
                        ui.close();
                    }
                    if ui.button("Open as file").clicked() {
                        menu = Some(BoxMenu::OpenRow);
                        ui.close();
                    }
                });
            });
        });
        if folded {
            return;
        }
        ui.add_space(4.0);
        if let SceneBundle::Area(held) = &def {
            form::kv(ui, "name", |ui| {
                let id = ui.id().with(("area-name", &bundle));
                renamed = input::drafted(ui, id, &held.name, |ui, draft| {
                    InputField::new(draft).show(ui)
                });
            });
            ui.add_space(2.0);
            ui.label(theme::faint("BLUEPRINT").size(10.5));
        }
        form::kv(ui, "size", |ui| {
            input::boxed(ui, Some("m"), |ui| {
                ui.label(format!("{} × {}", size.width, size.height))
            });
        });
        if let SceneBundle::Area(held) = &def {
            form::kv_text(
                ui,
                "terrains",
                held.terrains
                    .iter()
                    .map(|terrain| {
                        snapshot
                            .reader
                            .path_of(*terrain)
                            .map(name_of)
                            .unwrap_or("?")
                    })
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
            form::kv_text(ui, "slope", format!("walks up to {}°", held.max_slope.0));
        }
        if let SceneBundle::Blueprint(blueprint) = &def {
            variables_changed = variables(editor, ui, &bundle, blueprint, &mut actions);
        }
    });
    if fold {
        editor.settings.boxes_folded = !folded;
        editor.save_settings();
    }
    match (&def, renamed) {
        (SceneBundle::Area(area), Some(name)) => {
            let mut area = (**area).clone();
            area.name = name;
            let label = format!("Rename area {}", file_title(&bundle));
            editor.write_content(&label, &bundle, &area);
        }
        (SceneBundle::Blueprint(blueprint), _) if let Some(variables) = variables_changed => {
            let mut blueprint = (**blueprint).clone();
            blueprint.variables = variables;
            let label = format!("Edit the variables of {}", file_title(&bundle));
            editor.write_content(&label, &bundle, &blueprint);
        }
        _ => {}
    }
    for action in actions {
        row_form::act(
            editor,
            action,
            engine::systems::area::blueprint::BlueprintDef::TABLE,
            &bundle,
        );
    }
    match menu {
        Some(BoxMenu::Resize) => {
            editor
                .dialogs
                .push(ModalDialog::Resize(resize::ResizeDialog::for_bundle(
                    &bundle,
                    def.size(),
                )))
        }
        Some(BoxMenu::Play) => views::play_here(editor),
        Some(BoxMenu::OpenRow) => editor.open_tab(EditorTab::Row {
            table: engine::core::content::table_of(&bundle)
                .unwrap_or_default()
                .to_owned(),
            key: bundle.clone(),
        }),
        None => {}
    }
}

enum BoxMenu {
    Resize,
    Play,
    OpenRow,
}

/// A blueprint's variables, edited in its box, each with how many placements set it; the
/// variables as edited, when they changed.
fn variables(
    editor: &mut Editor,
    ui: &mut Ui,
    bundle: &str,
    blueprint: &BlueprintDef,
    actions: &mut Vec<crate::forms::FormAction>,
) -> Option<BTreeMap<String, Variable<RiftScripting>>> {
    let placements = set_by(editor, bundle);
    let shapes = editor.shapes.clone();
    let root = shapes.shape_of(BlueprintDef::TABLE);
    let Editor {
        project,
        form,
        shapes,
        ..
    } = editor;
    let project = project.as_ref()?;
    let mut cx = FormCx::new(project, root, shapes, form, actions)
        .editing(BlueprintDef::TABLE, Some(bundle));
    ui.add_space(2.0);
    form::kv(ui, "variables", |ui| {
        if blueprint.variables.is_empty() {
            ui.label(theme::muted("none").size(12.5));
        }
    });
    for (name, (set, of)) in &placements {
        if blueprint.variables.contains_key(name) {
            form::hint(ui, &format!("{name}: set by {set} of {of} placements"));
        }
    }
    let path = field!(BlueprintDef, variables);
    let mut variables = blueprint.variables.clone();
    forms::of_type(ui, &mut cx, &mut variables, path)
        .is_some()
        .then_some(variables)
}

/// For each variable placements of `bundle` set, how many set it and how many placements there are.
fn set_by(editor: &Editor, bundle: &str) -> HashMap<String, (usize, usize)> {
    let mut out: HashMap<String, (usize, usize)> = HashMap::new();
    let Some(project) = &editor.project else {
        return out;
    };
    let reader = project.reader();
    let mut holders: Vec<&str> = project
        .usages
        .of_content(bundle)
        .iter()
        .filter_map(|usage| content_of(&usage.file).map(|(_, holder)| holder))
        .collect();
    holders.sort();
    holders.dedup();
    let mut placed: Vec<Placement> = Vec::new();
    for holder in holders {
        let Some(scene) = nodes::bundle_in(project, holder) else {
            continue;
        };
        nodes::walk(scene.nodes(), &mut |_, node| {
            if let SceneNode::Module(ModuleNode {
                module: AreaModule::Blueprint(placement),
                ..
            }) = node
                && reader.path_of(placement.blueprint) == Some(bundle)
            {
                placed.push(placement.clone());
            }
        });
    }
    let total = placed.len();
    for placement in placed {
        for name in placement.values.keys() {
            out.entry(name.clone()).or_insert((0, total)).0 += 1;
        }
    }
    out
}

fn header(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot) {
    let mut count = 0;
    nodes::walk(snapshot.nodes(), &mut |_, node| {
        count += usize::from(!nodes::is_folder(node));
    });
    let selected = scene_tree::selected_ids(snapshot, &editor.selection).len();
    let flagged = snapshot.flags.said();
    let (hidden, locked) = (snapshot.flags.hidden, snapshot.flags.locked);
    let mut action = None;
    let mut add = None;
    ui.horizontal(|ui| {
        ui.label(form::section_heading(&format!("Scene · {count}")));
        if let Some(said) = &flagged
            && chip::plain(ui, said)
                .on_hover_text("Opens the tree to the first one")
                .clicked()
        {
            action = Some(HeaderAction::FirstFlagged);
        }
        if selected > 1 {
            chip::plain(ui, format!("{selected} selected"));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), |ui| {
                let mut item = |ui: &mut Ui, text: String, key: Option<EditorKey>, picked| {
                    let clicked = match key {
                        Some(key) => ui.add(widgets::shortcut_button(text, key)).clicked(),
                        None => ui.button(text).clicked(),
                    };
                    if clicked {
                        action = Some(picked);
                        ui.close();
                    }
                };
                item(
                    ui,
                    format!("Show everything ({hidden})"),
                    Some(EditorKey::ShowEverything),
                    HeaderAction::Scene(SceneAction::ShowEverything),
                );
                item(
                    ui,
                    format!("Unlock everything ({locked})"),
                    Some(EditorKey::UnlockEverything),
                    HeaderAction::Scene(SceneAction::UnlockEverything),
                );
                ui.separator();
                item(
                    ui,
                    "Open all".to_owned(),
                    None,
                    HeaderAction::Scene(SceneAction::OpenAll),
                );
                item(
                    ui,
                    "Close all".to_owned(),
                    None,
                    HeaderAction::Scene(SceneAction::CloseAll),
                );
            });
            let plus = ui
                .button(Mark::Plus.text(14.0))
                .on_hover_text("Add into the insertion folder");
            egui::Popup::menu(&plus).show(|ui| {
                let folder = insertion_row(editor, snapshot);
                add = scene_tree::add_menu(ui, &folder, snapshot);
            });
        });
    });
    match action {
        Some(HeaderAction::FirstFlagged) => {
            let mut first = None;
            nodes::walk(snapshot.nodes(), &mut |pointer, node| {
                let flagged = node.hidden() || node.locked();
                if first.is_none() && flagged {
                    first = Some(pointer.to_owned());
                }
            });
            if let Some(first) = first {
                editor
                    .views
                    .area
                    .view_mut(view_key)
                    .scene
                    .tree
                    .reveal(&first);
            }
        }
        Some(HeaderAction::Scene(action)) => act(editor, view_key, snapshot, action),
        None => {}
    }
    if let Some(add) = add {
        act(editor, view_key, snapshot, add);
    }
}

enum HeaderAction {
    FirstFlagged,
    Scene(SceneAction),
}

/// The row new things land in: the selected folder, the folder of the selected row, the
/// placement edited in place, or the top level.
fn insertion_row(editor: &Editor, snapshot: &AreaSnapshot) -> String {
    let selected = scene_tree::selected_ids(snapshot, &editor.selection);
    let top = scene_tree::place_row(snapshot);
    match selected.as_slice() {
        [one] => match snapshot.scene_node(one) {
            Some(node) if nodes::is_folder(node) => one.clone(),
            Some(_) => {
                let parent = scene_tree::parent_id(one);
                if parent.is_empty() { top } else { parent }
            }
            None => top,
        },
        _ => top,
    }
}

fn filter_row(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let kinds = scene_kinds();
    let Some(project) = &editor.project else {
        return;
    };
    let key = scene_tree::tree_key(project, &editor.views.area.view(view_key).bundle);
    let panel = &mut editor.views.area.view_mut(view_key).scene;
    let view = editor.settings.scene_trees.entry(key).or_default();
    let ((), changed) = tree::filter_row(
        ui,
        |ui| {
            let response = input::search(
                ui,
                &mut panel.filter,
                "Filter the scene",
                Some(&EditorKey::FilterScene.name()),
            );
            if std::mem::take(&mut panel.focus_filter) {
                response.request_focus();
            }
            if response.has_focus() && ui.input(|i| EditorKey::Cancel.pressed(i)) {
                panel.filter.clear();
            }
        },
        |ui| tree::show_menu(ui, view, &kinds, &|kind| KindMark::Badge(kind), &mut []),
    );
    if changed {
        editor.save_settings();
    }
}

/// Every kind of node Show ▾ offers: every kind of module.
fn scene_kinds() -> Vec<String> {
    AreaModuleKind::VARIANTS
        .iter()
        .map(|kind| kind.tag().to_owned())
        .collect()
}

fn scene_graph(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot) {
    let Some(project) = &editor.project else {
        return;
    };
    let key = scene_tree::tree_key(project, &snapshot.bundle);
    let lit = lit_row(editor, snapshot);
    let selected = scene_tree::selected_ids(snapshot, &editor.selection);
    let placing = editor.views.area.view(view_key).placing;
    let Editor {
        project,
        settings,
        views,
        clipboard,
        selection,
        ..
    } = editor;
    let project = project.as_ref().expect("a project");
    let view = views.area.views.get_mut(view_key).expect("the view");
    let panel = &mut view.scene;
    if panel.shown != selected {
        match selected.as_slice() {
            [one] => panel.tree.reveal(one),
            many => panel.tree.select(many.to_vec()),
        }
        panel.shown = selected.clone();
    }
    let shown_kinds = settings.scene_trees.entry(key.clone()).or_default();
    let mut source = SceneSource::new(
        project,
        snapshot,
        selection,
        SceneNarrowing {
            filter: &panel.filter,
            view: shown_kinds,
            kept: &mut panel.narrowed,
        },
        clipboard.is_some() || !panel.cut.is_empty(),
    );
    source.lit = lit;
    source.placing = placing.map(|kind| (insertion_of(&selected, &source, snapshot), kind));
    if let Some((hits, total)) = source.hit_count() {
        ui.label(theme::faint(format!("{hits} of {total}")).size(12.0));
    }
    let mut open = shown_kinds.open.clone();
    let empty = snapshot.nodes().is_empty();
    let (response, changed) = egui::ScrollArea::vertical()
        .id_salt(("scene graph", view_key))
        .auto_shrink([false, false])
        .max_height(ui.available_height() - if empty { 90.0 } else { 0.0 })
        .show(ui, |ui| {
            tree::show(
                ui,
                egui::Id::new(("scene graph", view_key)),
                &mut panel.tree,
                &source,
                &mut open,
            )
        })
        .inner;
    let hovered = ui.ui_contains_pointer();
    drop(source);
    let shown_kinds = settings.scene_trees.entry(key).or_default();
    if changed {
        shown_kinds.open = open;
    }
    if let Some(ids) = &response.selected {
        panel.shown = ids.clone();
    }
    let double = ui.input(|i| {
        i.pointer
            .button_double_clicked(egui::PointerButton::Primary)
    });
    if changed {
        editor.save_settings();
    }
    if empty {
        empty_state(editor, ui, view_key, snapshot);
    }
    if let Some(ids) = response.selected {
        select_rows(editor, view_key, snapshot, &ids);
    }
    for id in response.activated {
        let placement = snapshot
            .scene_node(&id)
            .and_then(nodes::placement_of)
            .is_some();
        let action = if double && placement {
            SceneAction::EditInPlace(id)
        } else {
            SceneAction::ShowOnMap(id)
        };
        act(editor, view_key, snapshot, action);
    }
    if let Some((ids, drop)) = response.dropped
        && let Err(error) = scene_tree::move_rows(editor, snapshot, &ids, &drop.folder, &drop.at)
    {
        editor.toast(error);
    }
    if let Some((id, typed)) = response.renamed {
        renamed(editor, view_key, snapshot, &id, &typed);
    }
    if let Some(toggled) = response.toggled {
        if toggled.only {
            act(
                editor,
                view_key,
                snapshot,
                SceneAction::ShowOnly(toggled.id),
            );
        } else {
            let bundle = scene_tree::bundle_of_row(snapshot, &toggled.id);
            scene_tree::toggle(editor, &bundle, &[toggled.id], toggled.toggle);
        }
    }
    if let Some(action) = response.action {
        act(editor, view_key, snapshot, action);
    }
    if (response.focused || hovered) && !ui.ctx().text_edit_focused() {
        keys(editor, ui, view_key, snapshot, response.focused);
    }
}

fn insertion_of(selected: &[String], source: &SceneSource, snapshot: &AreaSnapshot) -> String {
    match selected {
        [one] if source.is_folder(one) => one.clone(),
        [one] => {
            let parent = scene_tree::parent_id(one);
            if parent.is_empty() {
                scene_tree::place_row(snapshot)
            } else {
                parent
            }
        }
        _ => scene_tree::place_row(snapshot),
    }
}

/// Below a scene of nothing but its ground: what goes here, and the two ways to start.
fn empty_state(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot) {
    let mut action = None;
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(
            theme::muted("Nothing placed yet. Modules, placements and folders go here.").size(12.5),
        );
        ui.horizontal(|ui| {
            let add = controls::dropdown(ui, "+ Add");
            egui::Popup::menu(&add).show(|ui| {
                action = scene_tree::add_menu(ui, "", snapshot);
            });
            if ui
                .add(widgets::shortcut_button("New folder", EditorKey::NewFolder))
                .clicked()
            {
                action = Some(SceneAction::NewFolder(String::new()));
            }
        });
    });
    if let Some(action) = action {
        act(editor, view_key, snapshot, action);
    }
}

/// The row of the locked module the pointer is over on the map, lit so it can be found.
fn lit_row(editor: &Editor, snapshot: &AreaSnapshot) -> Option<String> {
    let hover = editor.views.area.hover.as_ref()?;
    if hover.area != snapshot.bundle {
        return None;
    }
    snapshot.locked_hit_at(hover.tile)
}

/// Follows a selection made in the tree: the map and inspector take it, and selecting a row
/// outside the placement edited in place leaves it.
fn select_rows(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, ids: &[String]) {
    let selection = scene_tree::selection_of(snapshot, ids);
    let level = ids.first().map_or(0, |id| id.matches("/in/").count());
    let view = editor.views.area.view_mut(view_key);
    if level < view.in_place.len() && !ids.is_empty() {
        view.in_place.truncate(level);
    }
    editor.selection = selection;
}

fn renamed(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, id: &str, typed: &str) {
    let grouped = editor
        .views
        .area
        .view_mut(view_key)
        .scene
        .grouped
        .take()
        .filter(|(folder, _)| folder == id);
    match grouped {
        Some((_, step)) => scene_tree::rename_grouped(editor, snapshot, id, typed, &step),
        None => scene_tree::rename(editor, snapshot, id, typed),
    }
}

fn act(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, action: SceneAction) {
    if let Some(reason) = editor.project.as_ref().and_then(|p| p.read_only.clone())
        && action.writes()
    {
        editor.toast(reason);
        return;
    }
    let bundle_of = |id: &str| scene_tree::bundle_of_row(snapshot, id);
    let begin_rename = |editor: &mut Editor, id: &str| {
        let label = AreaSnapshot::for_view(editor, view_key)
            .and_then(|fresh| {
                fresh
                    .scene_node(id)
                    .map(|node| shapes::node_label(node, fresh.reader.as_ref()))
            })
            .unwrap_or_default();
        editor
            .views
            .area
            .view_mut(view_key)
            .scene
            .tree
            .begin_rename(id, &label);
    };
    match action {
        SceneAction::Add { folder, kind } => scene_tree::add(editor, snapshot, &folder, kind),
        SceneAction::NewFolder(folder) => {
            if let Some(id) = scene_tree::new_folder(editor, snapshot, &folder) {
                begin_rename(editor, &id);
            }
        }
        SceneAction::Rename(id) => begin_rename(editor, &id),
        SceneAction::ShowOnMap(id) => show_on_map(editor, view_key, snapshot, &id),
        SceneAction::OpenBlueprint(id) => {
            if let Some(blueprint) = snapshot
                .scene_node(&id)
                .and_then(nodes::placement_of)
                .and_then(|placement| snapshot.reader.path_of(placement.blueprint))
            {
                super::run_action(
                    editor,
                    snapshot,
                    super::AreaAction::OpenBlueprint(blueprint.to_owned()),
                );
            }
        }
        SceneAction::EditInPlace(id) => edit_in_place(editor, view_key, snapshot, &id),
        SceneAction::DoneEditing => {
            editor.views.area.view_mut(view_key).in_place.pop();
            editor.selection.parts.clear();
        }
        SceneAction::Unpack(id) => {
            let bundle = bundle_of(&id);
            if let Some(fresh) = AreaSnapshot::for_bundle(editor, &bundle) {
                placement::unpack(editor, &fresh, scene_tree::local(&id));
            }
        }
        SceneAction::Replace(id) => {
            let bundle = bundle_of(&id);
            if let Some(fresh) = AreaSnapshot::for_bundle(editor, &bundle) {
                placement::replace_dialog(editor, &fresh, scene_tree::local(&id));
            }
        }
        SceneAction::ReturnDoor(id) => super::run_action(
            editor,
            snapshot,
            super::AreaAction::ReturnDoor(scene_tree::local(&id).to_owned()),
        ),
        SceneAction::MoveTo(ids) => move_to_dialog(editor, snapshot, ids),
        SceneAction::Group(ids) => {
            if let Some((id, step)) = scene_tree::group(editor, snapshot, &ids) {
                begin_rename(editor, &id);
                editor.views.area.view_mut(view_key).scene.grouped = Some((id, step));
            }
        }
        SceneAction::Ungroup(ids) => {
            let bundle = ids.first().map(|id| bundle_of(id)).unwrap_or_default();
            scene_tree::ungroup(editor, &bundle, &ids);
        }
        SceneAction::Toggle(ids, toggle) => {
            let bundle = ids.first().map(|id| bundle_of(id)).unwrap_or_default();
            scene_tree::toggle(editor, &bundle, &ids, toggle);
        }
        SceneAction::ShowOnly(id) => {
            let view = editor.views.area.view_mut(view_key);
            view.show_only = match &view.show_only {
                Some(only) if *only == id => None,
                _ => Some(id),
            };
        }
        SceneAction::Copy(ids) => scene_tree::copy_rows(editor, snapshot, &ids),
        SceneAction::Cut(ids) => {
            let count = ids.len();
            editor.views.area.view_mut(view_key).scene.cut = ids;
            editor.toast(format!(
                "Cut {count} row{}; {} moves {} into the selected folder",
                words::plural(count),
                EditorKey::Paste.name(),
                if count == 1 { "it" } else { "them" }
            ));
        }
        SceneAction::Paste(folder) => {
            let cut = std::mem::take(&mut editor.views.area.view_mut(view_key).scene.cut);
            if cut.is_empty() {
                scene_tree::paste_rows(editor, snapshot, &folder);
            } else {
                let folder = scene_tree::paste_target(snapshot, &folder);
                if let Err(error) =
                    scene_tree::move_rows(editor, snapshot, &cut, &folder, &DropAt::End)
                {
                    editor.toast(error);
                }
            }
        }
        SceneAction::Duplicate(ids) => scene_tree::duplicate_rows(editor, snapshot, &ids),
        SceneAction::MakeBlueprint(ids) => {
            editor.selection = scene_tree::selection_of(snapshot, &ids);
            if let Some(fresh) = AreaSnapshot::for_view(editor, view_key) {
                super::run_action(editor, &fresh, super::AreaAction::MakeBlueprint);
            }
        }
        SceneAction::Delete(ids) => scene_tree::delete_rows(editor, snapshot, &ids),
        SceneAction::OpenAll | SceneAction::CloseAll => {
            let open = matches!(action, SceneAction::OpenAll);
            let mut folders = Vec::new();
            nodes::walk(snapshot.nodes(), &mut |pointer, node| {
                if nodes::is_folder(node) {
                    folders.push(pointer.to_owned());
                }
            });
            if let Some(project) = &editor.project {
                let key = scene_tree::tree_key(project, &snapshot.bundle);
                let view = editor.settings.scene_trees.entry(key).or_default();
                view.open = if open {
                    folders.iter().cloned().collect()
                } else {
                    BTreeSet::new()
                };
                editor.save_settings();
            }
            editor
                .views
                .area
                .view_mut(view_key)
                .scene
                .tree
                .set_open(folders, open);
        }
        SceneAction::ShowEverything => {
            scene_tree::clear_all(editor, &snapshot.bundle, Toggle::Hide)
        }
        SceneAction::UnlockEverything => {
            scene_tree::clear_all(editor, &snapshot.bundle, Toggle::Lock)
        }
    }
}

fn edit_in_place(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, id: &str) {
    let level = id.matches("/in/").count();
    let Some(blueprint) = snapshot
        .scene_node(id)
        .and_then(nodes::placement_of)
        .and_then(|placement| snapshot.reader.path_of(placement.blueprint))
        .map(str::to_owned)
    else {
        return;
    };
    let view = editor.views.area.view_mut(view_key);
    view.in_place.truncate(level);
    view.in_place.push(scene_tree::local(id).to_owned());
    view.set_tool(super::AreaTool::Select);
    let mut open: Vec<String> = vec![id.to_owned()];
    let mut parent = scene_tree::parent_id(id);
    while !parent.is_empty() {
        open.push(parent.clone());
        parent = scene_tree::parent_id(&parent);
    }
    view.scene.tree.set_open(open, true);
    editor.selection = crate::project::RowSelection::row(
        engine::systems::area::blueprint::BlueprintDef::TABLE,
        &blueprint,
    );
}

/// Moves the camera to what the row `id` covers, and selects it.
fn show_on_map(editor: &mut Editor, view_key: &str, snapshot: &AreaSnapshot, id: &str) {
    let Some(node) = snapshot.scene_node(id) else {
        return;
    };
    let level = id.matches("/in/").count();
    let bounds = if level > 0 {
        snapshot
            .in_place
            .last()
            .map(|place| shapes::cell_rect(place.at))
    } else if nodes::is_folder(node) {
        let pointer = scene_tree::local(id);
        snapshot
            .modules
            .iter()
            .filter(|module| module.pointer.starts_with(&format!("{pointer}/")))
            .filter_map(|module| snapshot.bounds_of(module))
            .reduce(|a, b| a.union(&b))
    } else {
        snapshot.module_bounds(scene_tree::local(id))
    };
    if let Some(bounds) = bounds {
        editor.views.area.view_mut(view_key).camera.frame(bounds);
    }
    editor.selection = scene_tree::selection_of(snapshot, &[id.to_owned()]);
}

fn move_to_dialog(editor: &mut Editor, snapshot: &AreaSnapshot, ids: Vec<String>) {
    let level = ids.first().map_or(0, |id| id.matches("/in/").count());
    let folders: Vec<(String, String)> = scene_tree::folders_of(snapshot, level)
        .into_iter()
        .filter(|(_, row)| {
            !ids.iter()
                .any(|id| row == id || row.starts_with(&format!("{id}/")))
        })
        .collect();
    let title = match ids.as_slice() {
        [one] => format!(
            "Move {} to…",
            snapshot
                .scene_node(one)
                .map(|node| shapes::node_label(node, snapshot.reader.as_ref()))
                .unwrap_or_default()
        ),
        many => format!("Move {} rows to…", many.len()),
    };
    editor.dialogs.push(ModalDialog::PickRow(PickRowDialog {
        title,
        table: String::new(),
        search: String::new(),
        picked: None,
        options: folders.iter().map(|(label, _)| label.clone()).collect(),
        then: PickRowThen::MoveNodes {
            bundle: snapshot.bundle.clone(),
            ids,
            folders,
        },
    }));
}

/// Moves the rows `ids` of the view of `bundle` into the folder labelled `picked` among `folders`.
pub fn move_into_picked(
    editor: &mut Editor,
    bundle: &str,
    ids: &[String],
    folders: &[(String, String)],
    picked: &str,
) {
    let Some((_, folder)) = folders.iter().find(|(label, _)| label == picked) else {
        return;
    };
    let Some(snapshot) = AreaSnapshot::for_bundle(editor, bundle) else {
        return;
    };
    if let Err(error) = scene_tree::move_rows(editor, &snapshot, ids, folder, &DropAt::End) {
        editor.toast(error);
    }
}

/// The scene graph's keys, while the tree has the keys or the pointer.
fn keys(editor: &mut Editor, ui: &mut Ui, view_key: &str, snapshot: &AreaSnapshot, focused: bool) {
    let input = ui.input(|i| i.clone());
    let pressed = |key: EditorKey| key.pressed(&input);
    let selected = scene_tree::selected_ids(snapshot, &editor.selection);
    let folder = insertion_row(editor, snapshot);
    let action = if pressed(EditorKey::FilterScene) {
        editor.views.area.view_mut(view_key).scene.focus_filter = true;
        None
    } else if pressed(EditorKey::Group) && !selected.is_empty() {
        Some(SceneAction::Group(selected.clone()))
    } else if pressed(EditorKey::Ungroup) && !selected.is_empty() {
        Some(SceneAction::Ungroup(selected.clone()))
    } else if pressed(EditorKey::NewFolder) {
        Some(SceneAction::NewFolder(folder.clone()))
    } else if pressed(EditorKey::Hide) && !selected.is_empty() {
        Some(SceneAction::Toggle(selected.clone(), Toggle::Hide))
    } else if pressed(EditorKey::Lock) && !selected.is_empty() {
        Some(SceneAction::Toggle(selected.clone(), Toggle::Lock))
    } else if pressed(EditorKey::ShowEverything) {
        Some(SceneAction::ShowEverything)
    } else if pressed(EditorKey::UnlockEverything) {
        Some(SceneAction::UnlockEverything)
    } else if pressed(EditorKey::OpenBlueprint) && selected.len() == 1 {
        Some(SceneAction::OpenBlueprint(selected[0].clone()))
    } else if pressed(EditorKey::Rename) && selected.len() == 1 {
        Some(SceneAction::Rename(selected[0].clone()))
    } else if pressed(EditorKey::Cut) && !selected.is_empty() {
        Some(SceneAction::Cut(selected.clone()))
    } else if !focused {
        None
    } else if pressed(EditorKey::Copy) && !selected.is_empty() {
        Some(SceneAction::Copy(selected.clone()))
    } else if pressed(EditorKey::Paste) {
        Some(SceneAction::Paste(folder.clone()))
    } else if pressed(EditorKey::Duplicate) && !selected.is_empty() {
        Some(SceneAction::Duplicate(selected.clone()))
    } else if pressed(EditorKey::Delete) && !selected.is_empty() {
        Some(SceneAction::Delete(selected.clone()))
    } else if pressed(EditorKey::MakeBlueprint) && !selected.is_empty() {
        Some(SceneAction::MakeBlueprint(selected.clone()))
    } else if pressed(EditorKey::Cancel) {
        let view = editor.views.area.view_mut(view_key);
        if view.show_only.take().is_none() && !view.in_place.is_empty() {
            Some(SceneAction::DoneEditing)
        } else {
            None
        }
    } else {
        None
    };
    if let Some([x, y]) = EditorKey::Reorder.arrow(&input)
        && x == 0
        && !selected.is_empty()
    {
        scene_tree::reorder_rows(editor, snapshot, &selected, y);
        return;
    }
    if focused {
        typeahead(editor, ui, view_key, snapshot, &selected, &input);
    }
    if let Some(action) = action {
        act(editor, view_key, snapshot, action);
    }
}

/// Typing a name jumps to the next row starting with it; `*` opens every folder beside the
/// selected row.
fn typeahead(
    editor: &mut Editor,
    ui: &Ui,
    view_key: &str,
    snapshot: &AreaSnapshot,
    selected: &[String],
    input: &egui::InputState,
) {
    let now = input.time;
    let typed: String = input
        .events
        .iter()
        .filter_map(|event| match event {
            egui::Event::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if typed.is_empty() || input.modifiers.command || input.modifiers.alt {
        return;
    }
    if typed == "*" {
        let parent = selected
            .first()
            .map(|id| scene_tree::parent_id(id))
            .unwrap_or_default();
        let siblings = scene_tree::rows_in(snapshot, &parent);
        let folders: Vec<String> = siblings
            .into_iter()
            .filter(|id| snapshot.scene_node(id).is_some_and(nodes::is_folder))
            .collect();
        editor
            .views
            .area
            .view_mut(view_key)
            .scene
            .tree
            .set_open(folders, true);
        return;
    }
    let panel = &mut editor.views.area.view_mut(view_key).scene;
    if now - panel.typed_at > 1.0 {
        panel.typed.clear();
    }
    panel.typed.push_str(&typed.to_lowercase());
    panel.typed_at = now;
    let wanted = panel.typed.clone();
    let rows = scene_tree::rows_in_order(snapshot);
    let start = selected
        .first()
        .and_then(|id| rows.iter().position(|row| row == id))
        .map_or(0, |at| at + usize::from(wanted.chars().count() == 1));
    let found = rows
        .iter()
        .cycle()
        .skip(start)
        .take(rows.len())
        .find(|id| {
            snapshot
                .scene_node(id)
                .map(|node| shapes::node_label(node, snapshot.reader.as_ref()))
                .is_some_and(|label| label.to_lowercase().starts_with(&wanted))
        })
        .cloned();
    if let Some(found) = found {
        editor
            .views
            .area
            .view_mut(view_key)
            .scene
            .tree
            .reveal(&found);
        editor.selection = scene_tree::selection_of(snapshot, &[found]);
    }
    ui.ctx().request_repaint();
}
