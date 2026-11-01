pub mod area;
pub mod bust;
pub mod character;
pub mod class;
pub mod cues;
pub mod item;
pub mod model;
pub mod model_part;
pub mod navigation;
pub mod prop;
pub mod script;
pub mod skeleton;
pub mod skill;
pub mod sound;
pub mod spreadsheet;
pub mod stage;
pub mod stance;
mod view_map;
pub use view_map::ViewMap;
pub mod vfx;
pub mod world;

use std::sync::Arc;
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use engine::core::content::ContentFile;
use engine::core::math::Pos;
use engine::core::render::Look;
use engine::core::units::Meters;
use engine::systems::GameSettings;
use engine::systems::area::{AreaDef, sixteenth};
use engine::systems::script::Place;
use ui::egui::Ui;

use crate::app::{BottomTab, Editor};
use crate::play;
use crate::project::AreaCameraState;
use crate::row_form::preview::RowPreviewState;
use crate::row_form::usages;

use crate::stage::Stages;
use crate::tabs::EditorTab;

/// Snaps a ground point to the half metre, or the sixteenth when `fine`.
pub(crate) fn snap(at: Pos<Meters>, fine: bool) -> Pos<Meters> {
    if fine {
        Pos::new(sixteenth(at.x), sixteenth(at.y))
    } else {
        Pos::new((at.x * 2.0).round() / 2.0, (at.y * 2.0).round() / 2.0)
    }
}

#[derive(Component)]
pub(crate) struct EditorCamera;

#[derive(Default)]
pub struct EditorViews {
    pub gltfs: crate::project::GltfCache,
    shared_generation: Option<u64>,
    pub area: area::AreaViews,
    pub world: world::WorldView,
    pub script: script::ScriptViews,
    pub model: ViewMap<model::ModelView>,
    pub model_part: ViewMap<model_part::ModelPartView>,
    pub bust: bust::BustViews,
    pub skeleton: skeleton::SkeletonViews,
    pub prop: prop::PropViews,
    pub spreadsheet: spreadsheet::SpreadsheetViews,
    pub sound: sound::SoundViews,
    pub character: character::CharacterView,
    pub class: class::ClassViews,
    pub item: ViewMap<item::ItemView>,
    pub vfx: vfx::VfxViews,
    pub preview: RowPreviewState,
    pub stage: Stages,
    pub toggles: stage::StageToggles,
    pub areas: crate::thumbs::areas::AreaThumbnails,
}

impl EditorViews {
    pub fn close(&mut self) {
        self.area.close();
        self.script.close();
    }

    pub(crate) fn camera_state(&self) -> Option<AreaCameraState> {
        self.area.camera_state()
    }

    pub(crate) fn restore_camera(&mut self, camera: Option<AreaCameraState>) {
        if let Some(camera) = camera {
            self.area.restore_camera(&camera);
        }
    }
}

/// The content file at `key` as the game reads it, or `None` once `ui` says why it doesn't read.
pub(crate) fn read_shown<Row: ContentFile>(
    editor: &Editor,
    ui: &mut Ui,
    key: &str,
) -> Option<Arc<Row>> {
    match editor.read::<Row>(key)? {
        Ok(row) => Some(row),
        Err(errors) => {
            crate::widgets::unreadable(ui, key, &errors);
            None
        }
    }
}

pub fn sync(editor: &mut Editor, world: &mut World) {
    share_content(editor, world);
    take_look(editor, world);
    sound::sync(editor, world);
    crate::stage::sync(editor, world);
    area::viewport::sync(editor, world);
    crate::thumbs::props::sync(editor, world);
    crate::thumbs::areas::sync(editor, world);
}

fn share_content(editor: &mut Editor, world: &mut World) {
    let content = editor
        .project
        .as_ref()
        .and_then(|project| Some((project.content.clone()?, project.generation)));
    let Some((content, generation)) = content else {
        editor.views.shared_generation = None;
        return;
    };
    if editor.views.shared_generation != Some(generation) {
        world.insert_resource(content);
        editor.views.shared_generation = Some(generation);
    }
}

/// The world draws as the open game looks.
fn take_look(editor: &Editor, world: &mut World) {
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.as_ref())
    else {
        return;
    };
    let look = &content.built::<GameSettings>().look;
    if world.get_resource::<Look>() != Some(look) {
        world.insert_resource(look.clone());
    }
}

pub(crate) fn bottom_tabs(editor: &Editor) -> Vec<(BottomTab, String)> {
    let mut tabs: Vec<BottomTab> = match crate::tabs::active_tab(editor) {
        Some(EditorTab::Area { .. }) | Some(EditorTab::Blueprint { .. }) => {
            let mut tabs = vec![BottomTab::Palette, BottomTab::Blueprints];
            if editor.views.sound.listening.is_some() {
                tabs.push(BottomTab::Listen);
            }
            tabs
        }
        Some(EditorTab::Script(_)) => vec![BottomTab::MayDo, BottomTab::UsedBy, BottomTab::Run],
        _ => Vec::new(),
    };
    if used_by_subject(editor).is_some() && !tabs.contains(&BottomTab::UsedBy) {
        tabs.push(BottomTab::UsedBy);
    }
    tabs.push(BottomTab::History);
    tabs.push(BottomTab::Output);
    tabs.into_iter()
        .map(|tab| {
            let title = match (tab, crate::tabs::active_tab(editor)) {
                (BottomTab::UsedBy, Some(EditorTab::Script(owner))) => format!(
                    "{}   {}",
                    tab.title(),
                    script::panel::used_by_count(editor, &owner)
                ),
                (BottomTab::UsedBy, _) => format!(
                    "{}   {}",
                    tab.title(),
                    used_by_subject(editor).map_or(0, |subject| subject.count(editor))
                ),
                _ => tab.title().to_owned(),
            };
            (tab, title)
        })
        .collect()
}

pub(crate) fn bottom_show(editor: &mut Editor, ui: &mut Ui, tab: BottomTab) {
    match (crate::tabs::active_tab(editor), tab) {
        (
            Some(EditorTab::Area { key } | EditorTab::Blueprint { key }),
            BottomTab::Blueprints | BottomTab::Palette,
        ) => area::bottom(editor, ui, &area::view_key(&key), tab),
        (Some(EditorTab::Area { .. } | EditorTab::Blueprint { .. }), BottomTab::Listen) => {
            sound::panel(editor, ui)
        }
        (Some(EditorTab::Script(owner)), BottomTab::MayDo | BottomTab::UsedBy | BottomTab::Run) => {
            script::bottom(editor, ui, &owner, tab)
        }
        (_, BottomTab::UsedBy) => {
            let Some(subject) = used_by_subject(editor) else {
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("used-by")
                .show(ui, |ui| match subject {
                    UsedBySubject::Row { key } => {
                        usages::show_usages_grouped(editor, ui, &key, false);
                    }
                    UsedBySubject::Asset { path } => usages::show_asset_usages(editor, ui, &path),
                });
        }
        _ => {
            ui.label(theme::muted("nothing to show here"));
        }
    }
}

/// What the active window's Used by panel lists the users of: the row or file it shows.
enum UsedBySubject {
    Row { key: String },
    Asset { path: String },
}

impl UsedBySubject {
    fn count(&self, editor: &Editor) -> usize {
        let Some(project) = &editor.project else {
            return 0;
        };
        match self {
            UsedBySubject::Row { key } => project.usages.of_content(key).len(),
            UsedBySubject::Asset { path } => project.usages.of_media(path).len(),
        }
    }
}

fn used_by_subject(editor: &Editor) -> Option<UsedBySubject> {
    match crate::tabs::active_tab(editor)? {
        EditorTab::World | EditorTab::Rows { .. } | EditorTab::Spreadsheet { .. } => None,
        EditorTab::Asset { path } => Some(UsedBySubject::Asset { path }),
        tab => tab.row().map(|(_, key)| UsedBySubject::Row { key }),
    }
}

pub(crate) fn play_here(editor: &mut Editor) {
    let Some(hover) = editor.views.area.hover.clone() else {
        editor.toast("Play here needs an area view under the pointer");
        return;
    };
    let Some(content) = editor.project.as_ref().and_then(|p| p.content.clone()) else {
        return;
    };
    let Some(area) = content.by_path::<AreaDef>(&hover.area) else {
        return;
    };
    play::start(
        editor,
        Some(Place {
            area,
            at: snap(hover.ground, false),
        }),
    );
}
