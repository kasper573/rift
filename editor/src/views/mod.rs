pub mod area;
pub mod character;
pub mod class;
pub mod model;
pub mod navigation;
pub mod script;
pub mod skill;
pub mod sound;
pub mod spreadsheet;
pub mod tileset;
pub mod vfx;
pub mod world;

use bevy::prelude::*;
use egui::Ui;
use engine::core::tiling::TilePos;
use engine::systems::area::AreaDef;
use engine::systems::script::Place;

use crate::app::{BottomTab, Editor};
use crate::project::{AreaCameraState, RowSelection};
use crate::tabs::EditorTab;
use crate::ui::browser::BrowserState;
use crate::ui::theme;
use crate::{play, ui};

#[derive(Component)]
pub struct EditorCamera;

#[derive(Default)]
pub struct EditorViews {
    pub browser: BrowserState,
    pub show_usages: bool,
    pub area: area::AreaViews,
    pub world: world::WorldView,
    pub script: script::ScriptViews,
    pub model: model::ModelViews,
    pub tileset: tileset::TilesetViews,
    pub spreadsheet: spreadsheet::SpreadsheetViews,
    pub sound: sound::SoundViews,
    pub character: character::CharacterView,
    pub class: class::ClassViews,
    pub vfx: vfx::VfxViews,
    pub assets: crate::ui::assets::AssetsState,
    pub preview: crate::ui::inspector::preview::InspectorPreviewState,
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

    pub fn content_changed(&mut self) {
        self.area.content_changed();
    }
}

pub fn sync(editor: &mut Editor, world: &mut World) {
    area::sync(editor, world);
    model::sounds::sync(editor, world);
    vfx::sync(editor, world);
}

pub fn bottom_tabs(editor: &Editor) -> Vec<(BottomTab, String)> {
    let mut tabs: Vec<BottomTab> = match ui::active_tab(editor) {
        Some(EditorTab::Area { .. }) | Some(EditorTab::Blueprint { .. }) => {
            let mut tabs = vec![BottomTab::Palette, BottomTab::Blueprints];
            if editor.views.sound.listening.is_some() {
                tabs.push(BottomTab::Listen);
            }
            tabs
        }
        Some(EditorTab::Script(_)) => vec![BottomTab::MayDo, BottomTab::UsedBy, BottomTab::Run],
        Some(EditorTab::Model { key }) => model::bottom_tabs(editor, &key),
        Some(EditorTab::Tileset { key }) => tileset::bottom_tabs(editor, &key),
        _ => Vec::new(),
    };
    tabs.push(BottomTab::History);
    tabs.push(BottomTab::Output);
    tabs.into_iter()
        .map(|tab| {
            let title = match (tab, ui::active_tab(editor)) {
                (BottomTab::UsedBy, Some(EditorTab::Script(owner))) => format!(
                    "{}   {}",
                    tab.title(),
                    script::panel::used_by_count(editor, &owner)
                ),
                _ => tab.title().to_owned(),
            };
            (tab, title)
        })
        .collect()
}

pub fn bottom_show(editor: &mut Editor, ui: &mut Ui, tab: BottomTab) {
    match (ui::active_tab(editor), tab) {
        (Some(EditorTab::Area { key }), BottomTab::Palette) => {
            area::bottom(editor, ui, &area::area_view_key(&key), tab)
        }
        (Some(EditorTab::Area { key }), BottomTab::Blueprints) => {
            area::bottom(editor, ui, &area::area_view_key(&key), tab)
        }
        (Some(EditorTab::Area { .. }) | Some(EditorTab::Blueprint { .. }), BottomTab::Listen) => {
            sound::panel(editor, ui)
        }
        (Some(EditorTab::Blueprint { key }), BottomTab::Palette | BottomTab::Blueprints) => {
            area::bottom(editor, ui, &area::blueprint_view_key(&key), tab)
        }
        (Some(EditorTab::Script(owner)), BottomTab::MayDo | BottomTab::UsedBy | BottomTab::Run) => {
            script::bottom(editor, ui, &owner, tab)
        }
        (
            Some(EditorTab::Model { key }),
            BottomTab::Timeline | BottomTab::Sheet | BottomTab::Sounds,
        ) => model::bottom(editor, ui, &key, tab),
        (
            Some(EditorTab::Tileset { key }),
            BottomTab::Surfaces | BottomTab::Mixes | BottomTab::Timeline,
        ) => tileset::bottom(editor, ui, &key, tab),
        _ => {
            ui.label(theme::muted("nothing to show here"));
        }
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
    let Some(area) = content.by_key::<AreaDef>(&hover.area) else {
        return;
    };
    play::start(
        editor,
        Some(Place {
            area,
            at: hover.tile.snap(),
        }),
    );
}

pub fn inspect_parts(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    if script::is_node_selection(selection) {
        script::inspect_parts(editor, ui, selection);
        return;
    }
    if selection
        .parts
        .first()
        .is_some_and(|part| part.starts_with("#sheet/"))
    {
        egui::ScrollArea::vertical()
            .id_salt(("sheet-inspect", &selection.table))
            .show(ui, |ui| spreadsheet::inspect(editor, ui, selection));
        return;
    }
    match selection.table.as_str() {
        "area" | "blueprint" => area::inspect_parts(editor, ui, selection),
        "model" => model::inspect_parts(editor, ui, selection),
        "tileset" => tileset::inspect_parts(editor, ui, selection),
        class::TABLE => class::inspect::show(editor, ui, selection),
        _ => {
            ui.label(format!(
                "{} parts of {}",
                selection.parts.len(),
                selection.table
            ));
        }
    }
}
