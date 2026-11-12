//! The prop view: the prop turning on a stage of the game's first terrain, the ground it takes up
//! shaped from above with handles or fitted to its model, and what the loader makes of its model.

pub mod footprint;
pub mod inspect;
pub mod turntable;

use std::path::PathBuf;
use std::sync::Arc;

use engine::core::content::Content;
use engine::core::content::ContentFile;
use engine::core::content::gltf::{self, Triangle};
use engine::core::content::media::GltfInfos;
use engine::core::units::{Meters, Radians};
use engine::systems::area::prop::{Footprint, ModelHash, PROP_TEXTURES, PropDef};
use engine::systems::area::{Picture, Pictured};
use engine::systems::player;
use ui::egui::{self, Ui};
use ui::{chip, controls};

use super::area::camera::OrbitCamera;
use super::stage::{self, StageState, StageToggle};
use crate::app::Editor;
use crate::project::files::{bundle_of, folder_entry};
use crate::project::{ContentEdit, FileStamp};
use crate::row_form;
use crate::tabs::shown_path;
use crate::views::ViewMap;

#[derive(Default)]
pub struct PropViews {
    pub views: ViewMap<PropView>,
    pub drops: Vec<PathBuf>,
}

pub struct PropView {
    pub stage: StageState,
    pub top: OrbitCamera,
    pub drag: Option<footprint::Drag>,
    geometry: Option<Arc<Geometry>>,
}

/// What the editor read of a prop's model, kept until the model changes.
pub struct Geometry {
    /// The version of the model's file it was read from.
    pub stamp: FileStamp,
    /// The hash of the model's bytes, as a footprint fitted to it records.
    pub hash: String,
    pub triangles: Vec<Triangle>,
    pub flat_shaded: bool,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<PropDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    take_drops(editor, key);
    ui::panel::left_scrolled(ui, "prop", |ui| {
        row_form::show_row(editor, ui, PropDef::TABLE, key)
    });
    ui::panel::inspector(ui, editor, "prop", |editor, ui| {
        inspect::show(editor, ui, key, &row);
    });
    egui::Frame::new()
        .inner_margin(crate::widgets::VIEW_PADDING)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            header(editor, ui, key, &row);
            ui.add_space(4.0);
            let height = ui.available_height();
            let turntable = egui::Vec2::new(ui.available_width(), (height * 0.55).max(120.0));
            turntable::show(editor, ui, key, &row, turntable);
            ui.add_space(6.0);
            footprint::show(editor, ui, key, &row, ui.available_size());
        });
    editor.status.left.push(format!(
        "{} · {}",
        shown_path(key),
        footprint::FootprintKind::of(&row.footprint).label()
    ));
    editor.status.left.push(
        "right drag turns · wheel zooms · drag the handles from above · drop a .glb to replace the \
         model"
            .to_owned(),
    );
}

/// A view for the prop at `key`, framed on its model, when none is open yet.
pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.prop.views.contains(key) {
        return;
    }
    let radius = model_bounds(editor, key).map_or(1.0, |(min, max)| (max - min).length() / 2.0);
    let stage = StageState::framing(radius);
    let mut top = OrbitCamera {
        yaw: Radians(0.0),
        ..stage.camera
    };
    top.top_down();
    editor.views.prop.views.insert(
        key,
        PropView {
            stage,
            top,
            drag: None,
            geometry: None,
        },
    );
}

/// Shapes the footprint of the prop at `key` as a polygon round its model's base, remembering the
/// model it was fitted to.
pub fn fit_to_mesh(editor: &mut Editor, key: &str) -> bool {
    let Some(Ok(row)) = editor.read::<PropDef>(key) else {
        return false;
    };
    let Some(geometry) = geometry(editor, key) else {
        editor.toast(format!("{} has no model to fit to", shown_path(key)));
        return false;
    };
    let Some(tall) = walker(editor) else {
        editor.toast("The game must load before a footprint is fitted to its player's height");
        return false;
    };
    let Some(footprint) =
        Footprint::fitted(&geometry.triangles, tall, ModelHash(geometry.hash.clone()))
    else {
        editor.toast(format!(
            "{}'s model has no ground to stand on",
            shown_path(key)
        ));
        return false;
    };
    let mut prop = (*row).clone();
    prop.footprint = footprint;
    editor.write_content(
        &format!("Fit {}'s footprint to its model", shown_path(key)),
        key,
        &prop,
    )
}

/// Gives the prop at `key` the footprint `footprint`, as one step.
pub fn shape(editor: &mut Editor, key: &str, footprint: Footprint) -> bool {
    editor.change(
        ContentEdit::new(format!("Shape {}'s footprint", shown_path(key))),
        key,
        |prop: &mut PropDef| {
            prop.footprint = footprint;
            Ok(())
        },
    )
}

/// Replaces the model of the prop at `key` with the glTF binary `bytes`, when it reads as one. A
/// footprint fitted to the old model is fitted to the new one in the same step, so the prop never
/// stands on a footprint the loader calls stale.
pub fn replace_model(editor: &mut Editor, key: &str, bytes: Vec<u8>) -> bool {
    let Some(Ok(row)) = editor.read::<PropDef>(key) else {
        return false;
    };
    let taken = match gltf::externalize(&bytes, PROP_TEXTURES) {
        Ok(taken) => taken,
        Err(error) => {
            editor.toast(format!("Not a glTF binary: {error}"));
            return false;
        }
    };
    let mut edit = ContentEdit::new(format!("Replace {}'s model", shown_path(key)));
    let mut refitted = None;
    if let Footprint::Polygon {
        fitted_to: Some(_), ..
    } = &row.footprint
    {
        let fitted = gltf::triangles(&taken.glb)
            .ok()
            .zip(walker(editor))
            .and_then(|(triangles, tall)| {
                let model = ModelHash(crate::project::files::hash(&taken.glb));
                Footprint::fitted(&triangles, tall, model)
            });
        let mut prop = (*row).clone();
        prop.footprint = fitted.unwrap_or_else(|| {
            editor.toast(format!(
                "{}'s new model has no ground to stand on: its footprint is now shaped by hand",
                shown_path(key)
            ));
            let Footprint::Polygon { points, .. } = &row.footprint else {
                unreachable!("matched a polygon above")
            };
            Footprint::Polygon {
                points: points.clone(),
                fitted_to: None,
            }
        });
        refitted = Some(prop);
    }
    let Some(project) = editor.project.as_ref() else {
        return false;
    };
    if let Some(prop) = &refitted
        && let Err(reason) = edit.write_content(project, key, prop)
    {
        editor.toast(reason);
        return false;
    }
    let model = row.model.path();
    let Some((bundle, _)) = bundle_of(&model) else {
        editor.toast(format!("{model} is not a part of a prop's folder"));
        return false;
    };
    let bundle = folder_entry(bundle);
    let old_textures: Vec<String> = project
        .files
        .paths_under(&bundle)
        .filter(|path| {
            path.strip_prefix(&bundle)
                .is_some_and(|part| PROP_TEXTURES.matches(part))
        })
        .cloned()
        .collect();
    for path in old_textures {
        edit.remove(path);
    }
    for (part, image) in taken.images {
        edit.write(format!("{bundle}{part}"), image.into());
    }
    edit.write(model, taken.glb.into());
    editor.apply(edit)
}

/// The triangles of the model of the prop at `key`, read again when the model changes.
pub fn geometry(editor: &mut Editor, key: &str) -> Option<Arc<Geometry>> {
    let project = editor.project.as_ref()?;
    let row = project.read::<PropDef>(key).ok()?;
    let path = row.model.path();
    let stamp = project.files.stamp_of(&path)?;
    ensure_view(editor, key);
    if let Some(known) = &editor.views.prop.views.view(key).geometry
        && known.stamp == stamp
    {
        return Some(known.clone());
    }
    let files = &editor.project.as_ref()?.files;
    let bytes = files.get(&path)?;
    let triangles = gltf::triangles(&bytes).ok()?;
    let geometry = Arc::new(Geometry {
        stamp,
        hash: files.hash_of(&path)?,
        flat_shaded: gltf::is_flat_shaded(&triangles),
        triangles,
    });
    editor.views.prop.views.view_mut(key).geometry = Some(geometry.clone());
    Some(geometry)
}

/// How tall the game's player stands, once the game has loaded: what of a model a footprint is
/// fitted round.
fn walker(editor: &Editor) -> Option<Meters> {
    let content = editor.project.as_ref()?.content.as_ref()?;
    Some(player::stature(content))
}

/// The box the model of the prop at `key` fits in, as the loader measured it.
pub(crate) fn model_bounds(
    editor: &Editor,
    key: &str,
) -> Option<(bevy::math::Vec3, bevy::math::Vec3)> {
    let project = editor.project.as_ref()?;
    let content = project.content.as_ref()?;
    let row = project.read::<PropDef>(key).ok()?;
    let info = content.built::<GltfInfos>().0.get(&row.model.path())?;
    Some((info.bounds.min, info.bounds.max))
}

/// The picture of the prop at `key` on its stage, made again when the project's content moves.
pub(super) fn picture(editor: &mut Editor, key: &str) -> Option<Arc<Picture>> {
    let project = editor.project.as_ref()?;
    let content = project.content.clone()?;
    let generation = project.generation;
    let pictured = pictured(&content, key)?;
    ensure_view(editor, key);
    editor
        .views
        .prop
        .views
        .view_mut(key)
        .stage
        .picture(&content, generation, pictured)
}

/// The prop at `key` as its stage pictures it.
pub(super) fn pictured(content: &Content, key: &str) -> Option<Pictured> {
    content.by_path::<PropDef>(key).map(Pictured::Prop)
}

enum HeaderAction {
    Convert(footprint::FootprintKind),
    Fit,
}

fn header(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef) {
    let current = footprint::FootprintKind::of(&row.footprint);
    let action = stage::bar(
        ui,
        key,
        &mut editor.views.toggles,
        &[StageToggle::Turning],
        |ui| {
            let converted = footprint::FootprintKind::ALL
                .into_iter()
                .filter(|kind| chip::mode(ui, kind.label(), *kind == current).clicked())
                .last()
                .filter(|kind| *kind != current)
                .map(HeaderAction::Convert);
            ui.add_space(8.0);
            let fit = controls::button(ui, "Fit to mesh")
                .on_hover_text("A polygon round the model's base, remembering the model it fits")
                .clicked();
            converted.or(fit.then_some(HeaderAction::Fit))
        },
    );
    match action {
        Some(HeaderAction::Convert(kind)) => {
            let bounds = model_bounds(editor, key);
            shape(
                editor,
                key,
                footprint::converted(&row.footprint, bounds, kind),
            );
        }
        Some(HeaderAction::Fit) => {
            fit_to_mesh(editor, key);
        }
        None => {}
    }
}

/// Takes the files dropped on the view: a glTF binary replaces the model.
fn take_drops(editor: &mut Editor, key: &str) {
    let drops = std::mem::take(&mut editor.views.prop.drops);
    for path in drops {
        let glb = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"));
        if !glb {
            editor.toast("Drop a .glb to replace the model");
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                replace_model(editor, key, bytes);
            }
            Err(error) => editor.toast(format!("{}: {error}", path.display())),
        }
    }
}
