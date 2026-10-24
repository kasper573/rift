use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use ui::egui;
use ui::form;
use ui::theme;
use ui::words;

use engine::core::content::{
    Content, ContentFile, ContentScope, FolderReader, name_of, rescoped, table_of,
};
use engine::core::geometry::Degrees;
use engine::core::math::{Pos, Rect};
use engine::core::script::VariableType;
use engine::core::units::{Cell, Meters, TilePos};
use engine::systems::area::blueprint::{
    self, BlueprintDef, BlueprintId, Flattened, PlacedBlueprint, Placement,
};
use engine::systems::area::object::{ObjectDef, ObjectShape};
use engine::systems::area::scene::{ModuleNode, SceneFolder, SceneNode};
use engine::systems::area::{self, AreaDef, AreaId, AreaModule};
use engine::systems::script::{Place, VarValue, Variable, condition};
use serde_json::Value;
use ui::controls;
use ui::egui::{RichText, Stroke as EStroke, Ui, Vec2};
use ui::glyph::{self, Mark};

use super::inspect::Followups;
use super::stamp::object_names;
use super::{AreaFrame, AreaSnapshot, nodes, shapes};
use crate::app::{Editor, ModalDialog, PickRowDialog, PickRowThen};
use crate::forms::{self, FormCx};
use crate::keymap::EditorKey;
use crate::project::files::content_of;
use crate::project::{IdPaths, Project, RowSelection};
use crate::tabs::file_title;
use crate::{rows, schema};

/// What each placement of a scene covers on the map, by its node's pointer: the modules it brings,
/// placed.
pub fn placement_bounds(placed: &[Flattened]) -> HashMap<String, Rect<Meters>> {
    let mut out: HashMap<String, Rect<Meters>> = HashMap::new();
    for placed in placed {
        if placed.chain.is_empty() {
            continue;
        }
        let Some(bounds) = shapes::bounds(&placed.module, None) else {
            continue;
        };
        out.entry(placed.origin.clone())
            .and_modify(|known| *known = known.union(&bounds))
            .or_insert(bounds);
    }
    out
}

pub fn first_area_using(editor: &Editor, blueprint: &str) -> Option<String> {
    let project = editor.project.as_ref()?;
    project
        .usages
        .of_content(blueprint)
        .iter()
        .find_map(|usage| {
            content_of(&usage.file)
                .filter(|(table, _)| *table == AreaDef::TABLE)
                .map(|(_, key)| key.to_owned())
        })
}

/// Where a blueprint is placed: straight into areas, and how many more times inside other
/// blueprints.
pub struct BlueprintPlacements<'a> {
    pub direct: Vec<(AreaId, &'a PlacedBlueprint)>,
    pub nested: usize,
}

pub fn placements_of(content: &Content, blueprint: BlueprintId) -> BlueprintPlacements<'_> {
    let mut placements = BlueprintPlacements {
        direct: Vec::new(),
        nested: 0,
    };
    for area_id in content.ids::<AreaDef>() {
        for placement in area::placements(content, area_id) {
            if placement.blueprint != blueprint {
                continue;
            }
            if placement.chain.len() <= 1 {
                placements.direct.push((area_id, placement));
            } else {
                placements.nested += 1;
            }
        }
    }
    placements
}

pub fn placement_count(content: &Content, blueprint: &str) -> String {
    let Some(id) = content.by_path::<BlueprintDef>(blueprint) else {
        return String::new();
    };
    let BlueprintPlacements { direct, nested } = placements_of(content, id);
    let mut areas: Vec<AreaId> = direct.iter().map(|(area, _)| *area).collect();
    areas.dedup();
    let direct = direct.len();
    let mut text = match areas.as_slice() {
        [] => "Placed nowhere yet.".to_owned(),
        [only] => {
            let name = only.get(content).name.clone();
            format!(
                "Placed {direct} time{}, all in {name}.",
                words::plural(direct)
            )
        }
        many => format!("Placed {direct} times in {} areas.", many.len()),
    };
    if nested > 0 {
        text.push_str(&format!(" {nested} more sit inside other blueprints."));
    }
    text
}

/// The values a placement gives its blueprint's variables, each edited as its variable's type.
pub fn placement_card(
    ui: &mut Ui,
    cx: &mut FormCx,
    placement: &mut Placement,
    def: &BlueprintDef,
    path: &str,
) -> bool {
    if def.variables.is_empty() {
        form::kv_text(ui, "values", "none: no variables");
        return false;
    }
    let reader = cx.project.reader();
    let mut changed = false;
    for (name, variable) in &def.variables {
        form::kv(ui, name, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                let default = variable_default(&reader, variable);
                let value_path = format!("{path}/values/{name}");
                match placement.values.get(name).map(|own| reader.to_json(own)) {
                    Some(Ok(mut value)) => {
                        if variable_field(ui, cx, variable, &mut value, &value_path)
                            && let Ok(read) = reader.from_json::<VarValue>(&value)
                        {
                            placement.values.insert(name.clone(), read);
                            changed = true;
                        }
                        if default.is_some()
                            && glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                                .on_hover_text("Back to the blueprint's default")
                                .clicked()
                        {
                            placement.values.remove(name);
                            changed = true;
                        }
                    }
                    Some(Err(_)) => {
                        ui.label(theme::warn("does not write as the game reads it"));
                    }
                    None => {
                        let shown = match &variable.default {
                            Some(default) => short_value(default, cx.project),
                            None => "missing".to_owned(),
                        };
                        let response = ui
                            .add(egui::Label::new(theme::muted(shown)).sense(egui::Sense::click()))
                            .on_hover_text("Click to set this copy's own value");
                        let fresh = default
                            .clone()
                            .unwrap_or_else(|| variable_template(variable, cx.nearby()));
                        if response.clicked()
                            && let Ok(read) = reader.from_json::<VarValue>(&fresh)
                        {
                            placement.values.insert(name.clone(), read);
                            changed = true;
                        }
                    }
                }
            });
        });
    }
    changed
}

/// The value a placement that leaves `variable` unset takes, as the scene would write it.
pub fn variable_default(reader: &FolderReader, variable: &Variable) -> Option<Value> {
    reader.to_json(variable.default.as_ref()?).ok()
}

pub fn variable_template(variable: &Variable, nearby: schema::Nearby) -> Value {
    let schema = schema::variable_schema(variable.ty, variable.table.as_deref());
    match variable.ty {
        VariableType::Condition => Value::Bool(true),
        _ => schema::template(&schema, &schema, nearby),
    }
}

pub fn variable_field(
    ui: &mut Ui,
    cx: &mut FormCx,
    variable: &Variable,
    value: &mut Value,
    path: &str,
) -> bool {
    let schema = schema::variable_schema(variable.ty, variable.table.as_deref());
    forms::value(ui, cx, &schema, value, path).is_some()
}

/// A value in a line: text as it is, a place by its area and tile, a condition as it reads.
pub(super) fn short_value(value: &VarValue, project: &Project) -> String {
    let text = match value {
        VarValue::Str(text) => return text.clone(),
        VarValue::Place(place) => {
            return format!(
                "{} {}, {}",
                project.reader().path_of(place.area).unwrap_or_default(),
                place.at.x,
                place.at.y
            );
        }
        VarValue::Integer(number) => number.to_string(),
        VarValue::Number(number) => number.to_string(),
        VarValue::Boolean(on) => on.to_string(),
        VarValue::Condition(held) => condition::describe(project, held),
    };
    if text.chars().count() > 40 {
        format!("{}…", text.chars().take(40).collect::<String>())
    } else {
        text
    }
}

/// Every place the object's scripts lead to, by the path of its area and its tile.
pub fn warp_destinations(object: &ObjectDef, paths: &impl IdPaths) -> Vec<(String, Pos<Meters>)> {
    object
        .scripts
        .iter()
        .flat_map(|script| script.values.values())
        .filter_map(|value| match value {
            VarValue::Place(place) => Some((paths.path_of(place.area)?.to_owned(), place.at)),
            _ => None,
        })
        .collect()
}

/// Points every place the object's scripts name at `to`, so a copy of a warp leads there instead.
pub fn redirect_places(object: &mut ObjectDef, to: Place) {
    for value in object
        .scripts
        .iter_mut()
        .flat_map(|script| script.values.values_mut())
    {
        if let VarValue::Place(place) = value {
            *place = to;
        }
    }
}

/// The values every selected placement gives, edited for all of them at once when they place the
/// same blueprint.
pub fn shared_values(
    ui: &mut Ui,
    cx: &mut FormCx,
    nodes: &mut [SceneNode],
    pointers: &[String],
) -> bool {
    let reader = cx.project.reader();
    let modules: Vec<&AreaModule> = pointers
        .iter()
        .filter_map(|pointer| nodes::module_node(nodes, pointer).map(|node| &node.module))
        .collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for module in &modules {
        *counts.entry(module.kind().tag()).or_default() += 1;
    }
    for (kind, count) in &counts {
        ui.label(format!("{count} × {}", schema::title(kind)));
    }
    let blueprints: HashSet<BlueprintId> = modules
        .iter()
        .filter_map(|module| match module {
            AreaModule::Blueprint(placement) => Some(placement.blueprint),
            _ => None,
        })
        .collect();
    if blueprints.len() != 1 || modules.len() != pointers.len() || counts.len() != 1 {
        return false;
    }
    let Some(def) = blueprints
        .iter()
        .next()
        .and_then(|&id| reader.path_of(id))
        .and_then(|path| cx.project.read::<BlueprintDef>(path).ok())
    else {
        return false;
    };
    ui.label(RichText::new("Shared values").strong());
    let mut changed = false;
    let value_of = |nodes: &[SceneNode], pointer: &str, name: &str| -> Option<Value> {
        match &nodes::module_node(nodes, pointer)?.module {
            AreaModule::Blueprint(placement) => reader.to_json(placement.values.get(name)?).ok(),
            _ => None,
        }
    };
    let set_all = |nodes: &mut [SceneNode], name: &str, value: VarValue| {
        for pointer in pointers {
            if let Some(ModuleNode {
                module: AreaModule::Blueprint(placement),
                ..
            }) = nodes::module_node_mut(nodes, pointer)
            {
                placement.values.insert(name.to_owned(), value.clone());
            }
        }
    };
    for (name, variable) in &def.variables {
        let values: Vec<Option<Value>> = pointers
            .iter()
            .map(|pointer| value_of(nodes, pointer, name))
            .collect();
        let distinct: HashSet<String> = values
            .iter()
            .map(|value| value.as_ref().map_or_else(String::new, Value::to_string))
            .collect();
        ui.horizontal(|ui| {
            ui.label(name);
            if distinct.len() != 1 {
                ui.label(RichText::new(format!("{} values", distinct.len())).weak());
                return;
            }
            match values[0].clone() {
                None => {
                    ui.label(RichText::new("default").weak());
                    if ui.small_button("Set all").clicked() {
                        let template = variable_default(&reader, variable)
                            .unwrap_or_else(|| variable_template(variable, cx.nearby()));
                        if let Ok(read) = reader.from_json::<VarValue>(&template) {
                            set_all(nodes, name, read);
                            changed = true;
                        }
                    }
                }
                Some(mut value) => {
                    if variable_field(ui, cx, variable, &mut value, &format!("/shared/{name}"))
                        && let Ok(read) = reader.from_json::<VarValue>(&value)
                    {
                        set_all(nodes, name, read);
                        changed = true;
                    }
                }
            }
        });
    }
    changed
}

#[derive(Clone)]
pub struct AreaPlacement {
    /// The placement's node in the scene that places it: the area's, or the blueprint's of the
    /// placement before it.
    pub node: String,
    pub blueprint: String,
    pub at: Pos<Meters>,
    pub yaw: Degrees,
    pub values: BTreeMap<String, VarValue>,
    /// The blueprint as its files read now.
    pub def: Arc<BlueprintDef>,
}

/// The placements edited in place along `path`, each named by its node's pointer in the scene of
/// the one before; as many as still resolve.
pub fn resolve_in_place(
    editor: &Editor,
    scene: &[SceneNode],
    path: &[String],
) -> Option<Vec<AreaPlacement>> {
    let Some((top, nested)) = path.split_first() else {
        return Some(Vec::new());
    };
    let AreaModule::Blueprint(placement) = &nodes::module_node(scene, top)?.module else {
        return None;
    };
    let mut chain = vec![placement_of(editor, top, placement)?];
    for node in nested {
        let Some(next) = chain
            .last()
            .and_then(|current| nested_place(editor, current, node))
        else {
            break;
        };
        chain.push(next);
    }
    Some(chain)
}

/// The placement at `node` in the scene of the blueprint `current` places, placed as it lands.
fn nested_place(editor: &Editor, current: &AreaPlacement, node: &str) -> Option<AreaPlacement> {
    let reader = editor.reader()?;
    let part = nodes::module_node(&current.def.scene, node)?;
    let placed = placed_part(&reader, part, current)?;
    match &placed {
        AreaModule::Blueprint(placement) => placement_of(editor, node, placement),
        _ => None,
    }
}

fn placement_of(editor: &Editor, node: &str, placement: &Placement) -> Option<AreaPlacement> {
    AreaPlacement::of(editor.project.as_ref()?, node, placement)
}

impl AreaPlacement {
    /// The placement at `node`, `placement`, read among `project`'s files.
    pub fn of(project: &Project, node: &str, placement: &Placement) -> Option<AreaPlacement> {
        let blueprint = project.reader().path_of(placement.blueprint)?.to_owned();
        Some(AreaPlacement {
            node: node.to_owned(),
            def: project.read::<BlueprintDef>(&blueprint).ok()?,
            blueprint,
            at: placement.at,
            yaw: placement.yaw,
            values: placement.values.clone(),
        })
    }
}

/// The module of a part of the blueprint `place` places, its holes filled with the placement's
/// values and placed as the placement lands.
pub fn placed_part(
    reader: &FolderReader,
    part: &ModuleNode,
    place: &AreaPlacement,
) -> Option<AreaModule> {
    let mut module =
        blueprint::fill_module(reader.scope(), part, &place.values, &place.def.variables)?;
    blueprint::place(&mut module, place.at, place.yaw);
    Some(module)
}

pub struct PartView {
    pub part: String,
    pub module: AreaModule,
    pub bounds: Option<Rect<Meters>>,
    pub nested: Option<String>,
    pub locked: bool,
}

pub fn parts_in_place(frame: &AreaFrame, place: &AreaPlacement) -> Vec<PartView> {
    let reader = frame.reader.clone();
    let row = super::scene_tree::place_row(frame);
    let mut out = Vec::new();
    for part in nodes::modules(&place.def.scene) {
        let flags = frame.flags.row(&format!("{row}/in{}", part.pointer));
        if flags.hidden.on() {
            continue;
        }
        let Some(module) = placed_part(&reader, &part.node, place) else {
            continue;
        };
        let (nested, bounds) = match &module {
            AreaModule::Blueprint(placement) => (
                reader.path_of(placement.blueprint).map(str::to_owned),
                nested_bounds(&frame.content, &reader, placement),
            ),
            module => (None, shapes::bounds(module, None)),
        };
        out.push(PartView {
            part: part.pointer,
            module,
            bounds,
            nested,
            locked: flags.locked.on(),
        });
    }
    out
}

/// What a placement read among `reader`'s paths covers once `content` flattens what it places.
pub(super) fn nested_bounds(
    content: &Content,
    reader: &FolderReader,
    placement: &Placement,
) -> Option<Rect<Meters>> {
    let loaded = rescoped(placement, reader.scope(), &ContentScope::of(content)).ok()?;
    let node = SceneNode::Module(ModuleNode::new(AreaModule::Blueprint(loaded)));
    let mut errors = Vec::new();
    let flattening = blueprint::flatten(content, std::slice::from_ref(&node), &mut errors);
    let bounds = flattening
        .modules
        .iter()
        .filter_map(|placed| shapes::bounds(&placed.module, None))
        .reduce(|a, b| a.union(&b));
    Some(bounds.unwrap_or_else(|| shapes::cell_rect(placement.at)))
}

pub fn start_in_place(editor: &mut Editor, view_key: &str, area_key: &str, node: &str) {
    let scene = nodes::scene_of(editor, area_key);
    let Some(ModuleNode {
        module: AreaModule::Blueprint(placement),
        ..
    }) = nodes::module_node(&scene, node)
    else {
        return;
    };
    let Some(blueprint) = editor
        .reader()
        .and_then(|reader| reader.path_of(placement.blueprint).map(str::to_owned))
    else {
        return;
    };
    let view = editor.views.area.view_mut(view_key);
    view.in_place = vec![node.to_owned()];
    view.set_tool(super::AreaTool::Select);
    editor.selection = RowSelection::row(BlueprintDef::TABLE, &blueprint);
}

pub fn edit_in_place(editor: &mut Editor, view_key: &str, frame: &AreaFrame, node: &str) {
    start_in_place(editor, view_key, &frame.bundle, node);
}

pub fn enter_nested(editor: &mut Editor, view_key: &str, part: &str, blueprint: &str) {
    editor
        .views
        .area
        .view_mut(view_key)
        .in_place
        .push(part.to_owned());
    editor.selection = RowSelection::row(BlueprintDef::TABLE, blueprint);
}

/// The nodes of `blueprint` selected while it is edited in place.
pub fn selected_in_place(editor: &Editor, blueprint: &str) -> Vec<String> {
    if editor.selection.table != BlueprintDef::TABLE
        || !editor.selection.keys.iter().any(|k| k == blueprint)
    {
        return Vec::new();
    }
    editor.selection.parts.clone()
}

pub fn overlay_in_place(
    editor: &Editor,
    painter: &egui::Painter,
    frame: &AreaFrame,
    place: &AreaPlacement,
) {
    let canvas = frame.canvas;
    let parts = parts_in_place(frame, place);
    let extent = parts
        .iter()
        .filter_map(|part| part.bounds)
        .reduce(|a, b| a.union(&b))
        .or_else(|| frame.module_bounds(&place.node));
    if let Some(bounds) = extent {
        let shade = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150);
        let lit = canvas.rect(bounds);
        let full = canvas.screen;
        for dark in [
            egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, lit.min.y)),
            egui::Rect::from_min_max(egui::pos2(full.min.x, lit.max.y), full.max),
            egui::Rect::from_min_max(
                egui::pos2(full.min.x, lit.min.y),
                egui::pos2(lit.min.x, lit.max.y),
            ),
            egui::Rect::from_min_max(
                egui::pos2(lit.max.x, lit.min.y),
                egui::pos2(full.max.x, lit.max.y),
            ),
        ] {
            painter.rect_filled(dark, 0.0, shade);
        }
    }
    let selected = selected_in_place(editor, &place.blueprint);
    for part in parts {
        let Some(bounds) = part.bounds else {
            continue;
        };
        let is_selected = selected.contains(&part.part);
        let color = super::overlays::module_color(Some(part.module.kind()));
        let width = if is_selected { 2.0 } else { 1.0 };
        painter.rect_stroke(
            canvas.rect(bounds),
            0.0,
            EStroke::new(width, color),
            egui::StrokeKind::Inside,
        );
        let text = match &part.nested {
            Some(nested) => format!("{nested} (double-click to open)"),
            None => shapes::label(&part.module, frame.reader.as_ref()),
        };
        painter.text(
            canvas.rect(bounds).min + Vec2::new(2.0, 1.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(11.0),
            color,
        );
    }
}

pub fn move_parts_in_place(
    editor: &mut Editor,
    frame: &AreaFrame,
    place: &AreaPlacement,
    parts: &[String],
    dx: f32,
    dy: f32,
) {
    let local = Degrees(-place.yaw.0).rotate(engine::core::math::Offset::new(dx, dy));
    let (bx, by) = (local.x, local.y);
    let count = placement_count(&frame.content, &place.blueprint);
    nodes::change_scene(editor, &place.blueprint, |scene| {
        for part in parts {
            shapes::shift(&mut nodes::module_node_mut(scene, part)?.module, bx, by);
        }
        Some(format!(
            "Move {} parts of {} · {count}",
            parts.len(),
            place.blueprint
        ))
    });
}

/// The blueprint a placement places, as nodes of the scene it is placed in: its folders and
/// labels as they are, its modules with holes filled and positions placed.
pub fn unpacked(reader: &FolderReader, place: &AreaPlacement) -> Option<Vec<SceneNode>> {
    fn unpack(
        nodes: &[SceneNode],
        reader: &FolderReader,
        place: &AreaPlacement,
    ) -> Option<Vec<SceneNode>> {
        let mut out = Vec::new();
        for node in nodes {
            match node {
                SceneNode::Folder(folder) => out.push(SceneNode::Folder(SceneFolder {
                    children: unpack(&folder.children, reader, place)?,
                    ..folder.clone()
                })),
                SceneNode::Module(module) => out.push(SceneNode::Module(ModuleNode {
                    module: placed_part(reader, module, place)?,
                    holes: Default::default(),
                    ..module.clone()
                })),
            }
        }
        Some(out)
    }
    unpack(&place.def.scene, reader, place)
}

/// Points `placement` at the blueprint `to` declaring `variables`, dropping the values that don't
/// carry over to them; answers how many were dropped.
pub fn retarget(
    placement: &mut Placement,
    to: BlueprintId,
    variables: &BTreeMap<String, Variable>,
) -> usize {
    placement.blueprint = to;
    let before = placement.values.len();
    placement.values.retain(|name, value| {
        variables
            .get(name)
            .is_some_and(|variable| value_carries(variable, value))
    });
    before - placement.values.len()
}

fn value_carries(variable: &Variable, value: &VarValue) -> bool {
    matches!(
        (variable.ty, value),
        (VariableType::Integer, VarValue::Integer(_))
            | (
                VariableType::Number,
                VarValue::Integer(_) | VarValue::Number(_)
            )
            | (VariableType::Boolean, VarValue::Boolean(_))
            | (VariableType::Place, VarValue::Place(_))
            | (
                VariableType::Condition,
                VarValue::Condition(_) | VarValue::Boolean(_)
            )
            | (
                VariableType::Text | VariableType::Name | VariableType::Row | VariableType::Audio,
                VarValue::Str(_),
            )
    )
}

/// Replaces a placement with a folder of the same label holding what it places, as one step.
pub fn unpack(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let bundle = frame.bundle.clone();
    let Some(ModuleNode {
        module: AreaModule::Blueprint(placement),
        label,
        ..
    }) = frame.module_node(node).cloned()
    else {
        return;
    };
    let Some(place) = placement_of(editor, node, &placement) else {
        return;
    };
    let reader = frame.reader.clone();
    let label = label.unwrap_or_else(|| name_of(&place.blueprint).to_owned());
    let Some(children) = unpacked(&reader, &place) else {
        editor.toast(format!("{label} doesn't unpack: a hole of it has no value"));
        return;
    };
    let applied = nodes::change_scene(editor, &bundle, |scene| {
        *nodes::node_mut(scene, node)? = SceneNode::Folder(SceneFolder {
            label: label.clone(),
            hidden: false,
            locked: false,
            children,
        });
        Some(format!("Unpack {label} into a folder"))
    });
    if applied {
        nodes::select(editor, &bundle, vec![node.to_owned()]);
        editor.toast("Unpacked into a folder; Ctrl Z puts the placement back");
    }
}

pub fn replace_dialog(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let placed = match frame.module(node) {
        Some(AreaModule::Blueprint(placement)) => frame.reader.path_of(placement.blueprint),
        _ => None,
    };
    editor.dialogs.push(ModalDialog::PickRow(PickRowDialog {
        title: format!("Replace {} with", placed.unwrap_or("the blueprint")),
        table: BlueprintDef::TABLE.to_owned(),
        search: String::new(),
        picked: None,
        options: Vec::new(),
        then: PickRowThen::ReplaceBlueprint {
            area: frame.bundle.clone(),
            node: node.to_owned(),
        },
    }));
}

pub fn replace_with(editor: &mut Editor, bundle: &str, node: &str, replacement: &str) {
    let (Some(reader), Some(Ok(def))) = (editor.reader(), editor.read::<BlueprintDef>(replacement))
    else {
        return;
    };
    let Some(new_id) = reader.id_of::<BlueprintDef>(replacement) else {
        return;
    };
    let mut replaced = None;
    let applied = nodes::change_scene(editor, bundle, |scene| {
        let AreaModule::Blueprint(placement) = &mut nodes::module_node_mut(scene, node)?.module
        else {
            return None;
        };
        let old = reader.path_of(placement.blueprint)?.to_owned();
        let dropped = retarget(placement, new_id, &def.variables);
        let label = format!("Replace {old} with {replacement}");
        replaced = Some((old, dropped));
        Some(label)
    });
    if applied && let Some((old, dropped)) = replaced {
        editor.toast(format!(
            "Replaced {old} with {replacement}; {dropped} value{} didn't carry",
            words::plural(dropped)
        ));
    }
}

pub(super) fn placement_extras(
    ui: &mut Ui,
    cx: &mut FormCx,
    placement: &Placement,
    out: &mut Followups,
    node: &str,
) {
    let reader = cx.project.reader();
    let Some(key) = reader.path_of(placement.blueprint).map(str::to_owned) else {
        return;
    };
    ui.add_space(4.0);
    if let Some(content) = cx.content.clone() {
        form::hint(ui, &placement_count(&content, &key));
    }
    ui.horizontal_wrapped(|ui| {
        if controls::marked(ui, Mark::Grid, "Edit blueprint").clicked() {
            out.actions
                .push(super::AreaAction::OpenBlueprint(key.clone()));
        }
        if ui.button("Unpack into a folder").clicked() {
            out.actions.push(super::AreaAction::Unpack(node.to_owned()));
        }
        if ui.button("Replace with…").clicked() {
            out.actions
                .push(super::AreaAction::Replace(node.to_owned()));
        }
    });
    controls::key_hint_text(
        ui,
        &EditorKey::Confirm.name(),
        "edits the blueprint in place",
    );
}

/// Why placing the blueprint `blueprint` into `bundle` is refused, when it would end up inside
/// itself: the chain of what holds what.
pub fn holds_itself(project: &Project, bundle: &str, blueprint: &str) -> Option<String> {
    if table_of(bundle) != Some(BlueprintDef::TABLE) {
        return None;
    }
    let chain = chain_to(project, blueprint, bundle, &mut HashSet::new())?;
    let names: Vec<&str> = std::iter::once(bundle)
        .chain(chain.iter().map(String::as_str))
        .map(file_title)
        .collect();
    Some(format!("would hold itself: {}", names.join(" › ")))
}

/// The blueprints from `from` down to `target` through placements, both ends included.
fn chain_to(
    project: &Project,
    from: &str,
    target: &str,
    seen: &mut HashSet<String>,
) -> Option<Vec<String>> {
    if from == target {
        return Some(vec![from.to_owned()]);
    }
    if !seen.insert(from.to_owned()) {
        return None;
    }
    let reader = project.reader();
    let def = project.read::<BlueprintDef>(from).ok()?;
    nodes::modules(&def.scene)
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Blueprint(placement) => reader.path_of(placement.blueprint),
            _ => None,
        })
        .find_map(|placed| {
            let mut chain = chain_to(project, placed, target, seen)?;
            chain.insert(0, from.to_owned());
            Some(chain)
        })
}

/// Adds an object to the area the warp object at `node` leads to, on the nearest walkable cell
/// beside its arrival, with the object's scripts pointed back here, so the player can come back.
pub fn return_door(editor: &mut Editor, frame: &AreaSnapshot, node: &str) {
    let area_key = &frame.bundle;
    let Some(AreaModule::Object(object)) = frame.module(node) else {
        return;
    };
    let Some((target_area, target_at)) = warp_destinations(object, frame.reader.as_ref())
        .into_iter()
        .next()
    else {
        editor.toast("The object has no warp to return from");
        return;
    };
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return;
    };
    let Some(here_id) = content.by_path::<AreaDef>(area_key) else {
        return;
    };
    let Some(there_id) = content.by_path::<AreaDef>(&target_area) else {
        return;
    };
    let here = area::load(&content, here_id);
    let there = area::load(&content, there_id);
    let door_bounds = object.shape.bounds();
    let back_to = here
        .grid
        .nodes()
        .iter()
        .filter(|cell| !door_bounds.contains(**cell))
        .min_by(|a, b| {
            a.distance(door_bounds.center())
                .0
                .total_cmp(&b.distance(door_bounds.center()).0)
        })
        .copied();
    let Some(back_to) = back_to else {
        editor.toast("No walkable cell outside the door to come back to");
        return;
    };
    let arrival = target_at.cell();
    let Some(beside_arrival) = there
        .grid
        .nearest_walkable_where(target_at, |cell| cell != arrival)
    else {
        editor.toast("The destination has no walkable cell beside the arrival for a door back");
        return;
    };
    let mut names = object_names(area::placed(&content, there_id));
    let Some(here_in_files) = frame.reader.id_of::<AreaDef>(area_key) else {
        return;
    };
    let name = rows::free_name(
        &format!("{}_back", object.name.as_deref().unwrap_or("door")),
        &names,
    );
    names.insert(name.clone());
    let cell = beside_arrival.cell();
    let mut door = ObjectDef {
        name: Some(name),
        title: None,
        shape: ObjectShape::Point(cell.center()),
        look: None,
        shown: None,
        scripts: object.scripts.clone(),
    };
    redirect_places(
        &mut door,
        Place {
            area: here_in_files,
            at: back_to,
        },
    );
    let door = SceneNode::Module(ModuleNode::new(AreaModule::Object(door)));
    let applied = nodes::change_scene(editor, &target_area, |scene| {
        nodes::SceneTree::new(scene).append("", vec![door]);
        Some(format!(
            "Return door from {} to {}",
            file_title(&target_area),
            file_title(area_key)
        ))
    });
    if applied {
        editor.toast(format!(
            "Added a door back in {target_area} at {}, {}",
            cell.x, cell.y
        ));
    }
}
