//! The model view: the skin on a stage in any stance and clip its skeleton plays, how its joints
//! fit that skeleton, the clips of its own that override the skeleton's with their cues, its
//! hitbox sized by hand, its busts, and a new glb imported over it once it fits.

pub mod busts;
pub mod fit;
pub mod hitbox;
pub mod import;
pub mod inspect;
pub mod overrides;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use engine::core::content::media::GltfInfos;
use engine::core::content::{Content, ContentFile};
use engine::core::geometry::Capsule;
use engine::core::render::camera::world_point;
use engine::core::render::world::mannequin::RigClip;
use engine::core::time::{Millis, Seconds};
use engine::core::units::Meters;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::{ClipName, Cue};
use engine::systems::actor::{Action, ModelDef, Motion, Pose};
use engine::systems::area::{Picture, Pictured};
use engine::systems::npc::NpcDef;
use engine::systems::player::PlayerDef;
use serde::Deserialize;
use ui::egui::{self, Rect, Response, Sense, Ui, Vec2};
use ui::{chip, form, theme};

use super::area::camera::{self, AreaCanvas, OrbitCamera};
use super::area::viewport::Standing;
use super::stage::{self, StagePicture};
use crate::app::Editor;
use crate::project::RowSelection;
use crate::project::files::content_of;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

#[derive(Default)]
pub struct ModelViews {
    pub views: HashMap<String, ModelView>,
    pub drops: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPage {
    #[default]
    Fit,
    Overrides,
    Hitbox,
    Busts,
}

impl ModelPage {
    const ALL: [ModelPage; 4] = [
        ModelPage::Fit,
        ModelPage::Overrides,
        ModelPage::Hitbox,
        ModelPage::Busts,
    ];

    fn label(self) -> &'static str {
        match self {
            ModelPage::Fit => "fit",
            ModelPage::Overrides => "overrides",
            ModelPage::Hitbox => "hitbox",
            ModelPage::Busts => "busts",
        }
    }
}

pub struct ModelView {
    pub page: ModelPage,
    pub face: Option<String>,
    pub removing: Option<FaceRemoval>,
    /// The stance the stage shows the model in, by its file's path.
    pub stance: Option<String>,
    pub motion: Motion,
    pub pose: Pose,
    /// The clip of its own the overrides page times.
    pub own_clip: Option<ClipName>,
    pub scrub: super::cues::Scrub,
    pub camera: OrbitCamera,
    pub spinning: bool,
    pub drag: Option<hitbox::Drag>,
    pub import: Option<import::ImportDraft>,
    picture: StagePicture,
}

impl ModelViews {
    pub fn view(&self, key: &str) -> &ModelView {
        self.views
            .get(key)
            .expect("ensure_view ran before the model view was read")
    }

    pub fn view_mut(&mut self, key: &str) -> &mut ModelView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the model view was changed")
    }
}

#[derive(Clone)]
pub struct FaceRemoval {
    pub face: busts::BustFace,
    pub replacement: String,
    pub uses: usize,
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ModelDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    take_drops(editor, key);
    let ctx = ui.ctx().clone();
    inspect::remove_face_dialog(editor, &ctx, key);
    import::dialog(editor, &ctx, key);
    let page = editor.views.model.view(key).page;
    sidebar::row(ui, "model", |ui| {
        row_form::show_row(editor, ui, ModelDef::TABLE, key)
    });
    let selection = editor.selection.clone();
    sidebar::inspector(editor, ui, "model", |editor, ui| {
        if selection.table == ModelDef::TABLE
            && selection.keys.first().is_some_and(|k| k == key)
            && !selection.parts.is_empty()
        {
            inspect::show(editor, ui, &selection);
        } else {
            inspect::rig(editor, ui, key, &row);
        }
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            page_bar(editor, ui, key, page);
            ui.add_space(4.0);
            match page {
                ModelPage::Fit => fit::show(editor, ui, key, &row),
                ModelPage::Overrides => overrides::show(editor, ui, key, &row),
                ModelPage::Hitbox => hitbox::show(editor, ui, key, &row),
                ModelPage::Busts => {
                    egui::ScrollArea::both()
                        .id_salt(("model-page", key))
                        .auto_shrink([false, false])
                        .show(ui, |ui| busts::show(editor, ui, key, &row));
                }
            }
        });
}

fn page_bar(editor: &mut Editor, ui: &mut Ui, key: &str, page: ModelPage) {
    ui.horizontal(|ui| {
        ui.label(theme::mono(shown_path(key)).color(theme::CODE));
        ui.add_space(8.0);
        for other in ModelPage::ALL {
            if chip::mode(ui, other.label(), page == other).clicked() {
                editor.views.model.view_mut(key).page = other;
            }
        }
        if page != ModelPage::Busts {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let view = editor.views.model.view_mut(key);
                if chip::toggle(ui, "turning", view.spinning).clicked() {
                    view.spinning = !view.spinning;
                }
            });
        }
    });
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.model.views.contains_key(key) {
        return;
    }
    let model = editor.read::<ModelDef>(key).and_then(Result::ok);
    let height = model.as_ref().map_or(1.8, |model| model.hitbox.height.0);
    editor.views.model.views.insert(
        key.to_owned(),
        ModelView {
            page: ModelPage::default(),
            face: None,
            removing: None,
            stance: None,
            motion: Motion::Action(Action::Idle),
            pose: Pose::Idle,
            own_clip: None,
            scrub: super::cues::Scrub::default(),
            camera: stage::framing(height.max(1.0) * 0.7),
            spinning: false,
            drag: None,
            import: None,
            picture: StagePicture::default(),
        },
    );
    if let Some(model) = model
        && let Some(index) = busts::module_index(&model)
    {
        let path = format!("{}/modules/{index}", ModelDef::TABLE);
        if !editor.form.unfolded.contains(&path) {
            editor.form.folded.insert(path);
        }
    }
}

/// Gives the clip `clip` the model at `key` carries of its own the cues `cues`, in order of time:
/// none takes its entry away.
pub fn set_override_cues(
    editor: &mut Editor,
    key: &str,
    clip: &ClipName,
    mut cues: Vec<Cue>,
) -> bool {
    let Some(Ok(row)) = editor.read::<ModelDef>(key) else {
        return false;
    };
    cues.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    if row
        .overrides
        .get(clip)
        .map_or(cues.is_empty(), |held| *held == cues)
    {
        return false;
    }
    let mut model = (*row).clone();
    if cues.is_empty() {
        model.overrides.remove(clip);
    } else {
        model.overrides.insert(clip.clone(), cues);
    }
    editor.write_content(
        &format!("Mark {}'s {clip} cues", shown_path(key)),
        key,
        &model,
    )
}

/// Sizes the hitbox of the model at `key`.
pub fn set_hitbox(editor: &mut Editor, key: &str, hitbox: Capsule) -> bool {
    let Some(Ok(row)) = editor.read::<ModelDef>(key) else {
        return false;
    };
    if row.hitbox == hitbox {
        return false;
    }
    let mut model = (*row).clone();
    model.hitbox = hitbox;
    editor.write_content(&format!("Size {}'s hitbox", shown_path(key)), key, &model)
}

/// The model at `key` as the game built it from the files as they last loaded, when it loaded.
pub fn built<'c>(editor: &'c Editor, key: &str) -> Option<(&'c Content, &'c ActorModel)> {
    let content = editor.project.as_ref()?.content.as_ref()?;
    let id = content.by_path::<ModelDef>(key)?;
    Some((content, ActorModel::loaded(content, id)?))
}

/// The clips the model's own glTF carries, with how long each plays, as the loader measured them.
pub fn own_clips(editor: &Editor, row: &ModelDef) -> Vec<(ClipName, Millis)> {
    editor
        .project
        .as_ref()
        .and_then(|project| {
            let content = project.content.as_ref()?;
            let info = content.built::<GltfInfos>().0.get(&row.model.path())?;
            Some(
                info.clips
                    .iter()
                    .map(|clip| (ClipName::new(clip.name.clone()), clip.length.millis()))
                    .collect(),
            )
        })
        .unwrap_or_default()
}

pub fn select_face(editor: &mut Editor, key: &str, face: &busts::BustFace) {
    editor.select(RowSelection::parts(ModelDef::TABLE, key, vec![face.part()]));
}

pub fn users(editor: &Editor, key: &str) -> Vec<(String, String)> {
    let Some(project) = editor.project.as_ref() else {
        return Vec::new();
    };
    project
        .usages
        .of_content(key)
        .iter()
        .filter_map(|usage| content_of(&usage.file))
        .filter(|(table, _)| matches!(*table, NpcDef::TABLE | PlayerDef::TABLE))
        .map(|(table, key)| (table.to_owned(), key.to_owned()))
        .collect()
}

/// The stage with the camera moved as the pointer asks, for a page to draw over and then stand
/// its rig on.
pub(super) struct Turntable {
    pub rect: Rect,
    pub response: Response,
    pub canvas: AreaCanvas,
    view_key: String,
    picture: (u64, Arc<Picture>),
}

pub(super) fn turntable(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    size: Vec2,
) -> Option<Turntable> {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let Some(picture) = picture(editor, key) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no terrain to stand the rig on",
            egui::FontId::proportional(13.0),
            theme::MUTED,
        );
        return None;
    };
    let view_key = format!("model:{key}");
    stage::show_picture(editor, ui, &painter, rect, &view_key);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let view = editor.views.model.view_mut(key);
    let busy = camera::navigate(ui, &response, &mut view.camera, rect.height());
    if busy && response.dragged_by(egui::PointerButton::Secondary) {
        view.spinning = false;
    }
    if response.double_clicked() {
        view.camera.target = stage::center();
    }
    if view.spinning {
        stage::spin(&mut view.camera, dt);
        ui.ctx().request_repaint();
    }
    let canvas = AreaCanvas::new(rect, view.camera, picture.1.area.clone());
    Some(Turntable {
        rect,
        response,
        canvas,
        view_key,
        picture,
    })
}

impl Turntable {
    /// Stands the model in the middle of the stage, in `clip` held at `held`, and asks for the
    /// picture.
    pub fn stand(
        self,
        editor: &mut Editor,
        ui: &Ui,
        row: &ModelDef,
        clip: Option<RigClip>,
        held: Option<Seconds>,
    ) {
        let cast = vec![Standing {
            model: row.model.path(),
            clip,
            transform: bevy::prelude::Transform::from_translation(world_point(
                stage::center(),
                Meters(0.0),
            )),
            held,
        }];
        stage::request(
            editor,
            ui,
            &self.view_key,
            self.rect,
            self.canvas.camera,
            self.picture,
            cast,
        );
    }
}

fn picture(editor: &mut Editor, key: &str) -> Option<(u64, Arc<Picture>)> {
    let project = editor.project.as_ref()?;
    let content = project.content.clone()?;
    let generation = project.generation;
    ensure_view(editor, key);
    editor
        .views
        .model
        .view_mut(key)
        .picture
        .of(&content, generation, Pictured::Stage)
}

/// Takes the files dropped on the view: a glTF binary is offered as a new rig to import, pictures
/// go to the busts page.
fn take_drops(editor: &mut Editor, key: &str) {
    let drops = std::mem::take(&mut editor.views.model.drops);
    let (rigs, pictures): (Vec<PathBuf>, Vec<PathBuf>) = drops.into_iter().partition(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    });
    if let Some(path) = rigs.last() {
        match std::fs::read(path) {
            Ok(bytes) => match import::draft(editor, key, bytes, path.display().to_string()) {
                Ok(draft) => editor.views.model.view_mut(key).import = Some(draft),
                Err(error) => editor.toast(error),
            },
            Err(error) => editor.toast(format!("{}: {error}", path.display())),
        }
    }
    if !pictures.is_empty() {
        editor.views.model.view_mut(key).page = ModelPage::Busts;
        editor.views.model.drops = pictures;
    }
}

pub(super) fn nothing_to_stand(ui: &mut Ui) {
    form::hint(
        ui,
        "The model's glTF does not read, so nothing stands on the stage.",
    );
}
