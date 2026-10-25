pub mod area;
pub mod character;
pub mod class;
pub mod item;
pub mod model;
pub mod navigation;
pub mod prop;
pub mod script;
pub mod skill;
pub mod sound;
pub mod spreadsheet;
pub mod stage;
pub mod vfx;
pub mod world;

use std::sync::Arc;
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use engine::core::content::ContentFile;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::script::Place;
use ui::egui::Ui;

use crate::app::{BottomTab, Editor};
use crate::play;
use crate::project::AreaCameraState;
use crate::shell::row_form::preview::RowPreviewState;
use crate::shell::row_form::usages;
use crate::shell::{self};
use crate::stage::Stages;
use crate::tabs::EditorTab;

/// Snaps a ground point to the half metre.
pub(crate) fn snap(at: Pos<Meters>) -> Pos<Meters> {
    area::select::snap_to(at, false)
}

#[derive(Component)]
pub struct EditorCamera;

#[derive(Default)]
pub struct EditorViews {
    pub area: area::AreaViews,
    pub world: world::WorldView,
    pub script: script::ScriptViews,
    pub model: model::ModelViews,
    pub prop: prop::PropViews,
    pub spreadsheet: spreadsheet::SpreadsheetViews,
    pub sound: sound::SoundViews,
    pub character: character::CharacterView,
    pub class: class::ClassViews,
    pub item: item::ItemViews,
    pub vfx: vfx::VfxViews,
    pub preview: RowPreviewState,
    pub stage: Stages,
    pub areas: crate::thumbs::areas::AreaThumbnails,
}

impl EditorViews {
    pub fn close(&mut self) {
        self.area.close();
        self.script.close();
    }

    pub fn camera_state(&self) -> Option<AreaCameraState> {
        self.area.camera_state()
    }

    pub fn restore_camera(&mut self, camera: Option<AreaCameraState>) {
        if let Some(camera) = camera {
            self.area.restore_camera(&camera);
        }
    }
}

/// The content file at `key` as the game reads it, or `None` once `ui` says why it doesn't read.
pub fn read_shown<Row: ContentFile>(editor: &Editor, ui: &mut Ui, key: &str) -> Option<Arc<Row>> {
    match editor.read::<Row>(key)? {
        Ok(row) => Some(row),
        Err(errors) => {
            shell::widgets::unreadable(ui, key, &errors);
            None
        }
    }
}

pub fn sync(editor: &mut Editor, world: &mut World) {
    sound::sync(editor, world);
    crate::stage::sync(editor, world);
    area::viewport::sync(editor, world);
    crate::thumbs::props::sync(editor, world);
    crate::thumbs::areas::sync(editor, world);
}

pub fn bottom_tabs(editor: &Editor) -> Vec<(BottomTab, String)> {
    let mut tabs: Vec<BottomTab> = match shell::active_tab(editor) {
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
            let title = match (tab, shell::active_tab(editor)) {
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

pub fn bottom_show(editor: &mut Editor, ui: &mut Ui, tab: BottomTab) {
    match (shell::active_tab(editor), tab) {
        (Some(EditorTab::Area { key }), BottomTab::Blueprints | BottomTab::Palette) => {
            area::bottom(editor, ui, &area::view_key(&key), tab)
        }
        (Some(EditorTab::Area { .. }) | Some(EditorTab::Blueprint { .. }), BottomTab::Listen) => {
            sound::panel(editor, ui)
        }
        (Some(EditorTab::Blueprint { key }), BottomTab::Blueprints | BottomTab::Palette) => {
            area::bottom(editor, ui, &area::view_key(&key), tab)
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
    match shell::active_tab(editor)? {
        EditorTab::World | EditorTab::Rows { .. } | EditorTab::Spreadsheet { .. } => None,
        EditorTab::Asset { path } => Some(UsedBySubject::Asset { path }),
        tab => tab.row().map(|(_, key)| UsedBySubject::Row { key }),
    }
}

pub fn play_here(editor: &mut Editor) {
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
            at: snap(hover.ground),
        }),
    );
}
