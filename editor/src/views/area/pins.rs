use engine::core::content::{Content, SETTINGS_FILE, name_of};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::area::scene::SCENE_FILE;
use engine::systems::area::{self, AreaDef, AreaId, AreaModule, ObjectShape};
use engine::systems::script::Place;
use ui::egui;
use ui::egui::{Color32, Pos2, Stroke as EStroke, Vec2};

use super::{AreaFrame, nodes};
use crate::app::{Editor, PlacePick};
use crate::project::ContentEdit;
use crate::project::files::content_of;
use crate::scene_graph;
use crate::tabs::shown_path;

pub const INITIAL_SPAWN: &str = "/initial_spawn";

#[derive(Clone, Debug)]
pub struct ArrivalPin {
    pub file: String,
    pub pointer: String,
    pub at: Pos<Meters>,
    pub initial_spawn: bool,
}

#[derive(Clone, Debug)]
pub enum PinHit {
    InitialSpawn,
    Place { file: String, pointer: String },
}

pub fn arrivals(editor: &Editor, area_key: &str) -> Vec<ArrivalPin> {
    let Some(project) = &editor.project else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for usage in project.usages.of_content(area_key) {
        let Some(parent) = usage.pointer.strip_suffix("/area") else {
            continue;
        };
        let Some(place) = project.value_at::<Place>(&usage.file, parent) else {
            continue;
        };
        let initial_spawn = usage.file == SETTINGS_FILE && parent == INITIAL_SPAWN;
        out.push(ArrivalPin {
            file: usage.file.clone(),
            pointer: parent.to_owned(),
            at: place.at,
            initial_spawn,
        });
    }
    out
}

pub fn warp_objects(content: &Content, area_id: AreaId) -> Vec<ObjectShape> {
    area::warp_objects(content, area_id)
        .into_iter()
        .map(|(_, object)| object.shape)
        .collect()
}

impl ArrivalPin {
    /// Whose file the arrival is in, and the object leading to it when a scene's object does.
    pub fn names(&self, editor: &Editor) -> (String, Option<String>) {
        let owner = content_of(&self.file)
            .map(|(_, key)| name_of(key))
            .unwrap_or_else(|| shown_path(&self.file))
            .to_owned();
        let spot = self
            .file
            .strip_suffix(&format!("/{SCENE_FILE}"))
            .and_then(|bundle| nodes::bundle_of(editor, bundle))
            .and_then(|bundle| {
                let node = scene_graph::node_pointer(&self.pointer);
                match &nodes::module_node(bundle.nodes(), &node)?.module {
                    AreaModule::Object(object) => object.name.clone(),
                    _ => None,
                }
            });
        (owner, spot)
    }
}

/// Puts the place `at` in `area_key` where the place being picked goes, and ends the pick.
pub fn commit_pick(editor: &mut Editor, area_key: &str, at: Pos<Meters>) {
    let Some(PlacePick { file, pointer, .. }) = editor.pick.clone() else {
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(area) = project.reader().id_of::<AreaDef>(area_key) else {
        return;
    };
    let mut edit = ContentEdit::new(format!(
        "Pick place {area_key} {}, {} for {file}",
        at.x, at.y
    ));
    if let Err(error) = edit.update_at(project, &file, &pointer, |place: &mut Option<Place>| {
        *place = Some(Place { area, at });
    }) {
        editor.toast(error);
        editor.pick = None;
        return;
    }
    if editor.apply(edit) {
        editor.pick = None;
        editor.toast(format!("Picked {} {}, {}", name_of(area_key), at.x, at.y));
    }
}

pub fn draw_arrivals(editor: &Editor, painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    for arrival in arrivals(editor, &frame.bundle) {
        let at = canvas.to_screen(arrival.at);
        let color = if arrival.initial_spawn {
            Color32::from_rgb(120, 255, 160)
        } else {
            Color32::from_rgb(255, 190, 90)
        };
        pin(painter, at, color);
        if canvas.scale() >= 4.0 {
            let text = if arrival.initial_spawn {
                "initial spawn".to_owned()
            } else {
                chip_label(editor, &arrival)
            };
            frame
                .labels
                .chip(at + Vec2::new(8.0, -10.0), text, Color32::WHITE);
        }
    }
}

fn chip_label(editor: &Editor, arrival: &ArrivalPin) -> String {
    let (owner, spot) = arrival.names(editor);
    match spot {
        Some(spot) => format!("← {owner} · {spot}"),
        None => format!("← {owner}"),
    }
}

fn pin(painter: &egui::Painter, at: Pos2, color: Color32) {
    painter.circle_filled(at, 4.0, color);
    painter.circle_stroke(at, 4.0, EStroke::new(1.0, Color32::BLACK));
}

pub fn pin_at(editor: &Editor, frame: &AreaFrame, pointer: Pos2) -> Option<PinHit> {
    for arrival in arrivals(editor, &frame.bundle) {
        let at = frame.canvas.to_screen(arrival.at);
        let hit = egui::Rect::from_min_max(at + Vec2::new(-8.0, -20.0), at + Vec2::new(8.0, 2.0));
        if hit.contains(pointer) {
            return Some(if arrival.initial_spawn {
                PinHit::InitialSpawn
            } else {
                PinHit::Place {
                    file: arrival.file,
                    pointer: arrival.pointer,
                }
            });
        }
    }
    None
}

pub fn open_pin(editor: &mut Editor, pin: PinHit) {
    match pin {
        PinHit::InitialSpawn => {
            editor.toast("The game's spawn: set in game.config.json; drag the pin to move it")
        }
        PinHit::Place { file, pointer } => {
            if let Some((table, key)) = content_of(&file) {
                let (table, key) = (table.to_owned(), key.to_owned());
                editor.open_row(&table, &key);
                editor.toast(format!("{} {pointer}", shown_path(&key)));
            }
        }
    }
}

pub fn move_pin(
    editor: &mut Editor,
    frame: &AreaFrame,
    file: &str,
    pointer: &str,
    at: Pos<Meters>,
) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let mut edit = ContentEdit::new(format!(
        "Move arrival in {} to {}, {}",
        frame.bundle, at.x, at.y
    ));
    if let Err(error) = edit.update_at(project, file, pointer, |place: &mut Place| place.at = at) {
        editor.toast(error);
        return;
    }
    editor.apply(edit);
}

pub fn set_spawn(editor: &mut Editor, area_key: &str, at: Pos<Meters>) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let (Ok(settings), Some(area)) = (
        project.settings(),
        project.reader().id_of::<AreaDef>(area_key),
    ) else {
        return;
    };
    let mut settings = (*settings).clone();
    settings.initial_spawn = Place { area, at };
    let mut edit = ContentEdit::new(format!(
        "Set the initial spawn to {area_key} {}, {}",
        at.x, at.y
    ));
    if let Err(error) = edit.write_settings(project, &settings) {
        editor.toast(error);
        return;
    }
    editor.apply(edit);
}
