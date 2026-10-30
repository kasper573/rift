//! The prop view: the prop turning on a stage of the game's first terrain, the ground it takes up
//! shaped from above with handles or fitted to its model, and what the loader makes of its model.

pub mod footprint;
pub mod inspect;
pub mod turntable;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use engine::core::content::ContentFile;
use engine::core::content::gltf::{self, Triangle};
use engine::core::content::media::GltfInfos;
use engine::core::geometry::Radians;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::prop::{
    Footprint, MAX_POLYGON_POINTS, MIN_POLYGON_POINTS, ModelHash, PROP_TEXTURES, PropDef,
};
use engine::systems::area::{Picture, Pictured};
use ui::egui::{self, Ui};
use ui::{chip, controls, theme};

use super::area::camera::OrbitCamera;
use super::stage::{self, StagePicture};
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

/// How far above its lowest point a model's corners still count as standing on the ground; a
/// model shorter than twice this is measured to half its height instead.
pub const KNEE: Meters = Meters(0.5);

#[derive(Default)]
pub struct PropViews {
    pub views: HashMap<String, PropView>,
    pub drops: Vec<PathBuf>,
}

pub struct PropView {
    pub turntable: OrbitCamera,
    pub top: OrbitCamera,
    pub drag: Option<footprint::Drag>,
    picture: StagePicture,
    geometry: Option<Arc<Geometry>>,
}

/// What the editor read of a prop's model, kept until the model changes.
pub struct Geometry {
    pub hash: String,
    pub triangles: Vec<Triangle>,
    pub flat_shaded: bool,
}

impl PropViews {
    pub fn view(&self, key: &str) -> &PropView {
        self.views
            .get(key)
            .expect("ensure_view ran before the prop view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut PropView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the prop view was changed")
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<PropDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    take_drops(editor, key);
    sidebar::row(ui, "prop", |ui| {
        row_form::show_row(editor, ui, PropDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "prop", |editor, ui| {
        inspect::show(editor, ui, key, &row);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
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
        footprint::Kind::of(&row.footprint).label()
    ));
    editor.status.left.push(
        "right drag turns · wheel zooms · drag the handles from above · drop a .glb to replace the \
         model"
            .to_owned(),
    );
}

/// A view for the prop at `key`, framed on its model, when none is open yet.
pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.prop.views.contains_key(key) {
        return;
    }
    let radius = model_bounds(editor, key).map_or(1.0, |(min, max)| (max - min).length() / 2.0);
    let turntable = stage::framing(radius);
    let mut top = OrbitCamera {
        yaw: Radians(0.0),
        ..turntable
    };
    top.top_down();
    editor.views.prop.views.insert(
        key.to_owned(),
        PropView {
            turntable,
            top,
            drag: None,
            picture: StagePicture::default(),
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
    let points = gltf::footing(&geometry.triangles, knee_of(&geometry.triangles));
    if points.len() < MIN_POLYGON_POINTS {
        editor.toast(format!(
            "{}'s model has no ground to stand on",
            shown_path(key)
        ));
        return false;
    }
    let mut prop = (*row).clone();
    prop.footprint = Footprint::Polygon {
        points: thinned(points, MAX_POLYGON_POINTS),
        fitted_to: Some(ModelHash(geometry.hash.clone())),
    };
    editor.write_content(
        &format!("Fit {}'s footprint to its model", shown_path(key)),
        key,
        &prop,
    )
}

/// Gives the prop at `key` the footprint `footprint`, as one step.
pub fn shape(editor: &mut Editor, key: &str, footprint: Footprint) -> bool {
    let Some(Ok(row)) = editor.read::<PropDef>(key) else {
        return false;
    };
    if row.footprint == footprint {
        return false;
    }
    let mut prop = (*row).clone();
    prop.footprint = footprint;
    editor.write_content(
        &format!("Shape {}'s footprint", shown_path(key)),
        key,
        &prop,
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
        let points = gltf::triangles(&taken.glb)
            .ok()
            .map(|triangles| gltf::footing(&triangles, knee_of(&triangles)))
            .filter(|points| points.len() >= MIN_POLYGON_POINTS);
        let mut prop = (*row).clone();
        prop.footprint = match points {
            Some(points) => Footprint::Polygon {
                points: thinned(points, MAX_POLYGON_POINTS),
                fitted_to: Some(ModelHash(crate::project::files::hash(&taken.glb))),
            },
            None => {
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
            }
        };
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
    let bundle = format!(
        "{}/",
        model.rsplit_once('/').map_or("", |(bundle, _)| bundle)
    );
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
    let hash = project.files.hash_of(&path)?.to_owned();
    ensure_view(editor, key);
    if let Some(known) = &editor.views.prop.view(key).geometry
        && known.hash == hash
    {
        return Some(known.clone());
    }
    let bytes = editor.project.as_ref()?.files.get(&path)?.clone();
    let triangles = gltf::triangles(&bytes).ok()?;
    let geometry = Arc::new(Geometry {
        hash,
        flat_shaded: gltf::is_flat_shaded(&triangles),
        triangles,
    });
    editor.views.prop.view_mut(key).geometry = Some(geometry.clone());
    Some(geometry)
}

/// The box the model of the prop at `key` fits in, as the loader measured it.
pub fn model_bounds(editor: &Editor, key: &str) -> Option<(bevy::math::Vec3, bevy::math::Vec3)> {
    let project = editor.project.as_ref()?;
    let content = project.content.as_ref()?;
    let row = project.read::<PropDef>(key).ok()?;
    let info = content.built::<GltfInfos>().0.get(&row.model.path())?;
    Some((info.bounds.min, info.bounds.max))
}

/// The picture of the prop at `key` on its stage, made again when the project's content moves.
pub(super) fn picture(editor: &mut Editor, key: &str) -> Option<(u64, Arc<Picture>)> {
    let project = editor.project.as_ref()?;
    let content = project.content.clone()?;
    let generation = project.generation;
    let id = content.by_path::<PropDef>(key)?;
    ensure_view(editor, key);
    editor
        .views
        .prop
        .view_mut(key)
        .picture
        .of(&content, generation, Pictured::Prop(id))
}

fn header(editor: &mut Editor, ui: &mut Ui, key: &str, row: &PropDef) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        let current = footprint::Kind::of(&row.footprint);
        for kind in footprint::Kind::ALL {
            if chip::mode(ui, kind.label(), kind == current).clicked() && kind != current {
                let bounds = model_bounds(editor, key);
                shape(
                    editor,
                    key,
                    footprint::converted(&row.footprint, bounds, kind),
                );
            }
        }
        ui.add_space(8.0);
        if controls::button(ui, "Fit to mesh")
            .on_hover_text("A polygon round the model's base, remembering the model it fits")
            .clicked()
        {
            fit_to_mesh(editor, key);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            stage::toggle(ui, "turning", &mut editor.views.toggles.turning);
        });
    });
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

fn knee_of(triangles: &[Triangle]) -> Meters {
    let (lowest, highest) = triangles
        .iter()
        .flat_map(|triangle| triangle.corners)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), corner| {
            (low.min(corner.y), high.max(corner.y))
        });
    Meters(KNEE.0.min((highest - lowest) / 2.0).max(0.0))
}

/// At most `most` of `points`, evenly spread along them.
fn thinned(points: Vec<Pos<Meters>>, most: usize) -> Vec<Pos<Meters>> {
    if points.len() <= most {
        return points;
    }
    let step = points.len().div_ceil(most);
    points.into_iter().step_by(step).collect()
}
