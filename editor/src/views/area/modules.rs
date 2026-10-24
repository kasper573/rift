use std::collections::BTreeMap;
use ui::egui;
use ui::paint;
use ui::theme;

use engine::core::audio::mix::AudioCategory;
use engine::core::audio::soundscape::SoundscapeShape;
use engine::core::content::{FolderReader, Nearby};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect, Rgb, Size};
use engine::core::units::{Cell, CellPos, Meters, TilePos};
use engine::systems::actor::Pose;
use engine::systems::area::blueprint::Placement;
use engine::systems::area::lighting::PointLightDef;
use engine::systems::area::object::{ObjectDef, ObjectShape};
use engine::systems::area::prop::{PropPlacement, ScaleRange, Scatter};
use engine::systems::area::scene::{ModuleNode, SceneNode};
use engine::systems::area::{
    AreaModule, AreaModuleKind, Population, Resident, SafeZone, SoundChannel, SoundscapeDef,
};
use engine::systems::script::{Place, ScriptModule};
use ui::egui::{Pos2, RichText, Ui, Vec2};

use super::select::start_paste;
use super::{AreaAction, AreaFrame, AreaTool, AreaViewState, nodes, pick, pins, placement, shapes};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::play;
use crate::shell::widgets;
use crate::tabs::file_title;

#[derive(Clone, Copy)]
pub struct AreaMenuAnchor {
    pub at: Pos2,
    pub tile: Pos<Meters>,
}

pub(super) fn context_menu_at(editor: &mut Editor, view_key: &str, frame: &AreaFrame) {
    let Some(tile) = frame.pointer_tile else {
        return;
    };
    let view = editor.views.area.view_mut(view_key);
    view.cursor_cell = Some(tile.cell());
    view.take_drag();
    let selecting = view.tool == AreaTool::Select;
    if selecting
        && let Some(hit) = frame.hit(tile)
        && !super::selected_parts(editor, &frame.bundle).contains(&hit)
    {
        nodes::select(editor, &frame.bundle, vec![hit]);
    }
    let view = editor.views.area.view_mut(view_key);
    open_menu(
        &frame.response.ctx,
        view,
        view_key,
        frame.pointer.unwrap_or(frame.canvas.screen.center()),
        tile,
    );
}

pub(super) fn open_add_menu(
    ui: &mut Ui,
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    tile: Pos<Meters>,
) {
    if let Some(view) = editor.views.area.views.get_mut(view_key) {
        let at = frame.pointer.unwrap_or(frame.canvas.screen.center());
        open_menu(ui.ctx(), view, view_key, at, tile);
    }
}

pub(super) fn open_menu(
    ctx: &egui::Context,
    view: &mut AreaViewState,
    view_key: &str,
    at: Pos2,
    tile: Pos<Meters>,
) {
    egui::Popup::open_id(ctx, menu_id(view_key));
    view.menu = Some(AreaMenuAnchor { at, tile });
}

fn menu_id(view_key: &str) -> egui::Id {
    egui::Id::new((view_key, "context"))
}

pub fn module_menu_row(ui: &mut Ui, kind: &str, description: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().min(260.0), 34.0),
        egui::Sense::click(),
    );
    if response.hovered() {
        paint::row_wash(ui.painter(), rect);
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(6.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    widgets::badge(&mut child, kind);
    child.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(kind).monospace().color(theme::TEXT));
        ui.label(RichText::new(description).size(11.5).color(theme::MUTED));
    });
    response.clicked()
}

pub(super) fn show_add_menu(ui: &mut Ui, editor: &mut Editor, view_key: &str, frame: &AreaFrame) {
    let id = menu_id(view_key);
    if !egui::Popup::is_id_open(ui.ctx(), id) {
        return;
    }
    let Some(AreaMenuAnchor { at, tile }) = editor
        .views
        .area
        .views
        .get(view_key)
        .and_then(|view| view.menu)
    else {
        return;
    };
    let mut action: Option<MenuAction> = None;
    let selected = super::selected_parts(editor, &frame.bundle);
    egui::Popup::new(
        id,
        ui.ctx().clone(),
        egui::PopupAnchor::Position(at),
        ui.layer_id(),
    )
    .open_memory(None)
    .show(|ui| {
        ui.set_min_width(220.0);
        let cell = tile.cell();
        ui.label(RichText::new(format!("{}, {}", cell.x, cell.y)).weak());
        let character =
            play::current_character(editor).unwrap_or_else(|| "a test character".to_owned());
        let in_area = frame.area_id().is_some();
        if in_area
            && ui
                .add(widgets::shortcut_button(
                    format!("Play here as {character}"),
                    EditorKey::PlayHere,
                ))
                .clicked()
        {
            action = Some(MenuAction::PlayHere);
            ui.close();
        }
        ui.add_enabled_ui(in_area, |ui| {
            ui.menu_button("Play here as…", |ui| {
                for other in play::test_characters(editor) {
                    if ui
                        .button(format!(
                            "{} · level {} · {}",
                            other.key, other.level, other.area
                        ))
                        .clicked()
                    {
                        editor.play.character = Some(other.key.clone());
                        editor.settings.last_play = Some(other.key);
                        action = Some(MenuAction::PlayHere);
                        ui.close();
                    }
                }
            })
        });
        if ui
            .add(widgets::shortcut_button(
                "Play from its own start",
                EditorKey::PlayFromStart,
            ))
            .clicked()
        {
            action = Some(MenuAction::PlayFromStart);
            ui.close();
        }
        ui.separator();
        if in_area && ui.button("Set the game's spawn here").clicked() {
            action = Some(MenuAction::SetSpawn);
            ui.close();
        }
        if in_area
            && ui
                .button(format!("Listen here ({} click)", EditorKey::Listen.name()))
                .clicked()
        {
            action = Some(MenuAction::Listen);
            ui.close();
        }
        ui.separator();
        let folder = nodes::insertion_folder(editor, &frame.bundle);
        let into = frame
            .scene_node(&folder)
            .and_then(nodes::label_of)
            .map(|label| format!(" INTO {}", label.to_uppercase()))
            .unwrap_or_default();
        ui.label(
            RichText::new(format!("ADD MODULE{into}"))
                .size(11.0)
                .color(ui::theme::MUTED)
                .family(theme::medium()),
        );
        for (group, kinds) in ADD_MENU {
            ui.add_space(2.0);
            ui.label(RichText::new(*group).size(11.0).color(ui::theme::FAINT));
            for &(kind, description) in *kinds {
                if module_menu_row(ui, kind.tag(), description) {
                    action = Some(MenuAction::Add(kind));
                    ui.close();
                }
            }
        }
        if !selected.is_empty() {
            ui.separator();
            ui.label(RichText::new(format!("{} selected", selected.len())).strong());
            if ui
                .add(widgets::shortcut_button("Copy", EditorKey::Copy))
                .clicked()
            {
                action = Some(MenuAction::Area(AreaAction::Copy));
                ui.close();
            }
            if ui
                .add(widgets::shortcut_button("Duplicate", EditorKey::Duplicate))
                .clicked()
            {
                action = Some(MenuAction::Area(AreaAction::Duplicate));
                ui.close();
            }
            if ui
                .add(widgets::shortcut_button(
                    "Make blueprint",
                    EditorKey::MakeBlueprint,
                ))
                .clicked()
            {
                action = Some(MenuAction::Area(AreaAction::MakeBlueprint));
                ui.close();
            }
            if selected.len() == 1 {
                let node = selected.iter().next().cloned().unwrap_or_default();
                if let Some(module) = frame.module(&node) {
                    if let AreaModule::Blueprint(_) = module {
                        if ui
                            .add(widgets::shortcut_button(
                                "Edit blueprint in place",
                                EditorKey::Confirm,
                            ))
                            .clicked()
                        {
                            action = Some(MenuAction::EditInPlace(node.clone()));
                            ui.close();
                        }
                        if ui.button("Unpack").clicked() {
                            action = Some(MenuAction::Area(AreaAction::Unpack(node.clone())));
                            ui.close();
                        }
                        if ui.button("Replace with…").clicked() {
                            action = Some(MenuAction::Area(AreaAction::Replace(node.clone())));
                            ui.close();
                        }
                    }
                    if matches!(module, AreaModule::Object(_))
                        && ui.button("Return door…").clicked()
                    {
                        action = Some(MenuAction::Area(AreaAction::ReturnDoor(node.clone())));
                        ui.close();
                    }
                }
            }
            if ui
                .add(widgets::shortcut_button("Delete", EditorKey::Delete))
                .clicked()
            {
                action = Some(MenuAction::Area(AreaAction::Delete));
                ui.close();
            }
        }
        if editor.clipboard.is_some()
            && ui
                .add(widgets::shortcut_button("Paste here", EditorKey::Paste))
                .clicked()
        {
            action = Some(MenuAction::Paste);
            ui.close();
        }
    });
    let Some(action) = action else {
        return;
    };
    match action {
        MenuAction::PlayHere => {
            if let Some(area) = frame.area_id() {
                let place = Place {
                    area,
                    at: tile.snap(),
                };
                play::start(editor, Some(place));
            }
        }
        MenuAction::PlayFromStart => play::start(editor, None),
        MenuAction::SetSpawn => pins::set_spawn(editor, &frame.bundle, tile.snap()),
        MenuAction::Listen => {
            editor.views.area.view_mut(view_key).listener = Some(tile);
            pick::listen(editor, frame, tile);
        }
        MenuAction::Add(kind) => add_module(editor, view_key, frame, kind, tile),
        MenuAction::Paste => start_paste(editor, view_key),
        MenuAction::EditInPlace(node) => placement::edit_in_place(editor, view_key, frame, &node),
        MenuAction::Area(action) => super::run_action(editor, frame, action),
    }
}

pub(super) enum MenuAction {
    PlayHere,
    PlayFromStart,
    SetSpawn,
    Listen,
    Add(AreaModuleKind),
    Paste,
    EditInPlace(String),
    Area(AreaAction),
}

pub const ADD_MENU: &[(&str, &[(AreaModuleKind, &str)])] = &[
    (
        "Shapes",
        &[
            (AreaModuleKind::Object, "a point or rect with scripts"),
            (AreaModuleKind::SafeZone, "a rect where no one fights"),
            (AreaModuleKind::Soundscape, "a sound zone"),
        ],
    ),
    (
        "People",
        &[
            (AreaModuleKind::Population, "npcs that roam a rect"),
            (AreaModuleKind::Resident, "one npc at one spot"),
        ],
    ),
    (
        "Things",
        &[
            (AreaModuleKind::Prop, "one model standing here"),
            (AreaModuleKind::Scatter, "models strewn over a rect"),
            (AreaModuleKind::Light, "a point light"),
        ],
    ),
    (
        "Copies",
        &[(AreaModuleKind::Blueprint, "a blueprint, placed")],
    ),
    (
        "Area-wide",
        &[(AreaModuleKind::Script, "names a script row")],
    ),
];

/// What a module of `kind` first covers when it is placed at `cell`.
pub fn template_bounds(kind: AreaModuleKind, cell: CellPos) -> Rect<Meters> {
    match kind {
        AreaModuleKind::SafeZone
        | AreaModuleKind::Soundscape
        | AreaModuleKind::Population
        | AreaModuleKind::Scatter => spot_rect(cell),
        _ => cell.bounds(),
    }
}

/// A module of `kind` as it is first placed in `bundle` at `cell`, or area-wide when there is no
/// cell, naming the file at `named` when its kind names one, and the media nearest `bundle` when it
/// plays some. Refused, saying why, when its kind needs a cell, a file or media it hasn't.
pub fn module_template(
    reader: &FolderReader,
    bundle: &str,
    kind: AreaModuleKind,
    cell: Option<CellPos>,
    named: Option<&str>,
) -> Result<AreaModule, String> {
    let at = cell.map(|cell| Pos::new(cell.x as f32, cell.y as f32));
    let placed = || format!("a {} is placed on the map", kind.tag().replace('_', " "));
    let named = named.unwrap_or_default();
    let unnamed = || format!("{named} can't be named by a {}", kind.tag());
    Ok(match kind {
        AreaModuleKind::Object => AreaModule::Object(ObjectDef {
            name: None,
            title: None,
            shape: ObjectShape::Point(at.ok_or_else(placed)?),
            look: None,
            shown: None,
            scripts: Vec::new(),
        }),
        AreaModuleKind::SafeZone => AreaModule::SafeZone(SafeZone {
            rect: spot_rect(cell.ok_or_else(placed)?),
        }),
        AreaModuleKind::Soundscape => AreaModule::Soundscape(SoundscapeDef {
            shape: SoundscapeShape::Rect(spot_rect(cell.ok_or_else(placed)?)),
            emit: 4.0,
            channels: vec![SoundChannel {
                channel: 1,
                file: Nearby::new(reader, bundle).audio()?,
                volume: 1.0,
                pan: 0.0,
                category: AudioCategory::default(),
            }],
        }),
        AreaModuleKind::Population => AreaModule::Population(Population {
            npc: reader.id_of(named).ok_or_else(unnamed)?,
            count: 1,
            roams: cell.map(spot_rect),
        }),
        AreaModuleKind::Resident => AreaModule::Resident(Resident {
            npc: reader.id_of(named).ok_or_else(unnamed)?,
            at: at.ok_or_else(placed)?,
            facing: Degrees::default(),
            pose: Pose::Idle,
            shown: None,
        }),
        AreaModuleKind::Blueprint => AreaModule::Blueprint(Placement {
            blueprint: reader.id_of(named).ok_or_else(unnamed)?,
            at: at.ok_or_else(placed)?,
            yaw: Degrees::default(),
            values: BTreeMap::new(),
        }),
        AreaModuleKind::Prop => AreaModule::Prop(PropPlacement {
            prop: reader.id_of(named).ok_or_else(unnamed)?,
            at: at.ok_or_else(placed)?,
            yaw: Degrees::default(),
            scale: 1.0,
        }),
        AreaModuleKind::Scatter => AreaModule::Scatter(Scatter {
            props: vec![reader.id_of(named).ok_or_else(unnamed)?],
            region: spot_rect(cell.ok_or_else(placed)?),
            per_square_meter: 0.25,
            seed: 1,
            scale: ScaleRange::default(),
        }),
        AreaModuleKind::Light => AreaModule::Light(PointLightDef {
            at: at.ok_or_else(placed)?,
            color: Rgb::WHITE,
            intensity: 20.0,
            radius: Meters(6.0),
            height: Meters(2.0),
            flicker: 0.0,
        }),
        AreaModuleKind::Script => AreaModule::Script(ScriptModule {
            script: reader.id_of(named).ok_or_else(unnamed)?,
            values: BTreeMap::new(),
        }),
    })
}

/// Adds a module of `kind` where `tile` is: into the blueprint being edited in place when it is
/// selected, at the tile's spot within it. Asks first which file it names, unless the view's stamp
/// names the blueprint to place.
pub fn add_module(
    editor: &mut Editor,
    view_key: &str,
    frame: &AreaFrame,
    kind: AreaModuleKind,
    tile: Pos<Meters>,
) {
    let (bundle, tile) = match frame.in_place.last() {
        Some(place) if editor.selection.keys.first() == Some(&place.blueprint) => (
            place.blueprint.clone(),
            Pos::new(0.0, 0.0) + Degrees(-place.yaw.0).rotate(tile - place.at),
        ),
        _ => (frame.bundle.clone(), tile),
    };
    let stamp = editor.views.area.view(view_key).stamp.clone();
    match (kind, stamp) {
        (AreaModuleKind::Blueprint, Some(stamp)) => {
            pick::add_picked(editor, &bundle, kind, Some(tile.cell()), &stamp)
        }
        (kind, _) if pick::named_table(kind).is_some() => {
            pick::pick_then_add(editor, &bundle, kind, Some(tile.cell()))
        }
        (kind, _) => {
            let Some(reader) = editor.reader() else {
                return;
            };
            match module_template(&reader, &bundle, kind, Some(tile.cell()), None) {
                Ok(module) => {
                    let label = format!(
                        "Add {} to {}",
                        shapes::label(&module, reader.as_ref()),
                        file_title(&bundle)
                    );
                    nodes::add(
                        editor,
                        &bundle,
                        vec![SceneNode::Module(ModuleNode::new(module))],
                        &label,
                    );
                }
                Err(why) => editor.toast(format!("No {} here: {why}", kind.tag())),
            }
        }
    }
}

pub enum PlaceOutcome {
    Whole,
    Partly { skipped: usize, reason: String },
}

/// Places `new` in the insertion folder of `bundle` as one step: all of them, or as many as the
/// folder's game takes, in order, when some are refused. Selects what it placed.
pub fn place_modules(
    editor: &mut Editor,
    bundle: &str,
    mut new: Vec<SceneNode>,
    label: &str,
) -> PlaceOutcome {
    let count = new.len();
    let folder = nodes::insertion_folder(editor, bundle);
    let Some(reader) = editor.reader() else {
        return PlaceOutcome::Partly {
            skipped: count,
            reason: "no folder is open".to_owned(),
        };
    };
    let scene = nodes::scene_of(editor, bundle);
    nodes::label_placements(&scene, &folder, &mut new, &reader);
    let with = |editor: &Editor, placed: &[SceneNode], label: String| {
        let mut added = Vec::new();
        let edit = nodes::scene_edit(editor, bundle, |nodes| {
            added = nodes::SceneTree::new(nodes).append(&folder, placed.to_vec());
            Some(label)
        });
        edit.map(|edit| (edit, added))
    };
    let check = |editor: &Editor, placed: &[SceneNode]| -> Result<(), String> {
        let (edit, _) = with(editor, placed, String::new())
            .ok_or_else(|| format!("{} doesn't read", file_title(bundle)))?;
        let project = editor.project.as_ref().ok_or("no folder is open")?;
        project.check(&edit).map_err(|refusal| refusal.sentence())
    };
    let mut reason = match check(editor, &new) {
        Ok(()) => {
            let Some((edit, added)) = with(editor, &new, label.to_owned()) else {
                return PlaceOutcome::Partly {
                    skipped: count,
                    reason: format!("{} doesn't read", file_title(bundle)),
                };
            };
            return match editor.try_apply(edit) {
                Ok(()) => {
                    nodes::select(editor, bundle, added);
                    PlaceOutcome::Whole
                }
                Err(reason) => PlaceOutcome::Partly {
                    skipped: count,
                    reason,
                },
            };
        }
        Err(reason) => reason,
    };
    let mut kept: Vec<SceneNode> = Vec::new();
    let mut skipped = 0;
    for node in new {
        let mut candidate = kept.clone();
        candidate.push(node);
        match check(editor, &candidate) {
            Ok(()) => kept = candidate,
            Err(refused) => {
                skipped += 1;
                if kept.is_empty() {
                    reason = refused;
                }
            }
        }
    }
    if !kept.is_empty()
        && let Some((edit, added)) = with(editor, &kept, format!("{label} ({skipped} skipped)"))
        && editor.try_apply(edit).is_ok()
    {
        nodes::select(editor, bundle, added);
    }
    PlaceOutcome::Partly { skipped, reason }
}

fn spot_rect(cell: CellPos) -> Rect<Meters> {
    Rect::new(
        Pos::new(cell.x as f32 - 0.5, cell.y as f32 - 0.5),
        Size::splat(2.0),
    )
}
