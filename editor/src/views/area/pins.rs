use engine::core::content::Content;
use engine::core::content::{SETTINGS_FILE, name_of};
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::field;
use engine::systems::GameSettings;
use engine::systems::area;
use engine::systems::area::scene::SCENE_FILE;
use engine::systems::area::{AreaDef, AreaId, AreaModule, check_place};
use engine::systems::script::Place;
use ui::egui;
use ui::egui::{Color32, Pos2, Sense, Stroke as EStroke, Ui, Vec2};
use ui::theme;
use ui::{controls, face, form, sheet};

use super::{AreaFrame, nodes};
use crate::app::{Editor, PlacePick};
use crate::forms;
use crate::keymap::EditorKey;
use crate::project::files::content_of;
use crate::project::{ContentEdit, Project};
use crate::scene_graph;
use crate::tabs::shown_path;
use crate::views::navigation::{self, PX_PER_METER};

const INITIAL_SPAWN: &str = field!(GameSettings, initial_spawn);

#[derive(Clone, Debug)]
pub struct ArrivalPin {
    pub file: String,
    pub pointer: String,
    pub at: Pos<Meters>,
    pub initial_spawn: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum PinHit {
    InitialSpawn,
    Place { file: String, pointer: String },
}

/// Every arrival into `area_key`: each place in the files that leads there.
pub fn arrivals(editor: &Editor, area_key: &str) -> Vec<ArrivalPin> {
    arrivals_where(editor, area_key, |_, _| true)
}

/// The arrivals into `area_key` whose files are at hand, so a frame drawing them parses nothing:
/// one in a file not yet parsed shows once the background has parsed it.
fn arrivals_at_hand(editor: &Editor, area_key: &str) -> Vec<ArrivalPin> {
    arrivals_where(editor, area_key, |project, file| {
        project.files.json_at_hand(file)
    })
}

fn arrivals_where(
    editor: &Editor,
    area_key: &str,
    at_hand: impl Fn(&Project, &str) -> bool,
) -> Vec<ArrivalPin> {
    let Some(project) = &editor.project else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for usage in project.usages.of_content(area_key) {
        let Some(parent) = usage.pointer.strip_suffix(field!(Place, area)) else {
            continue;
        };
        if !at_hand(project, &usage.file) {
            continue;
        }
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

/// The ground under a pointer taken as a place in an area, with whether the game accepts it.
pub(crate) struct PickedPlace {
    pub at: Pos<Meters>,
    pub verdict: Result<(), engine::core::content::ContentProblem>,
}

impl PickedPlace {
    pub fn at(content: &Content, area: AreaId, ground: Pos<Meters>, fine: bool) -> PickedPlace {
        let at = crate::views::snap(ground, fine);
        let verdict = check_place(content, Place { area, at });
        PickedPlace { at, verdict }
    }

    pub fn color(&self) -> Color32 {
        if self.verdict.is_ok() {
            Color32::from_rgb(255, 230, 120)
        } else {
            theme::DANGER
        }
    }

    pub fn commit(self, editor: &mut Editor, area_key: &str) {
        match self.verdict {
            Ok(()) => commit_pick(editor, area_key, self.at),
            Err(problem) => editor.toast(format!("{}: {}", problem.rule, problem.expected)),
        }
    }
}

/// Puts the place `at` in `area_key` where the place being picked goes, and ends the pick.
fn commit_pick(editor: &mut Editor, area_key: &str, at: Pos<Meters>) {
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

pub(crate) fn draw_arrivals(editor: &Editor, painter: &egui::Painter, frame: &AreaFrame) {
    let canvas = &frame.canvas;
    for arrival in arrivals_at_hand(editor, &frame.bundle) {
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

pub(crate) fn pin_at(editor: &Editor, frame: &AreaFrame, pointer: Pos2) -> Option<PinHit> {
    for arrival in arrivals_at_hand(editor, &frame.bundle) {
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

pub(crate) fn open_pin(editor: &mut Editor, pin: PinHit) {
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

pub(crate) fn move_pin(
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

pub(crate) fn set_spawn(editor: &mut Editor, area_key: &str, at: Pos<Meters>) {
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

/// The panel a place is picked in for a file's field: an area chosen, then a spot on its map.
pub(crate) fn place_pick_panel(editor: &mut Editor, ui: &mut Ui) {
    let Some(pick) = editor.pick.clone() else {
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    let mut area_key = pick.area.clone().or_else(|| {
        let place = project.value_at::<Place>(&pick.file, &pick.pointer)?;
        project.reader().path_of(place.area).map(str::to_owned)
    });
    let mut areas: Vec<String> = content
        .ids::<area::AreaDef>()
        .map(|id| id.path(&content).to_owned())
        .collect();
    areas.sort();
    sheet::section(ui, "Pick a place");
    controls::combo("pick-area")
        .width(ui.available_width())
        .selected_text(face::mono(
            area_key
                .clone()
                .unwrap_or_else(|| "choose an area".to_owned()),
        ))
        .show_ui(ui, |ui| {
            for key in &areas {
                if ui
                    .selectable_label(area_key.as_deref() == Some(key.as_str()), key)
                    .clicked()
                {
                    area_key = Some(key.clone());
                }
            }
        });
    if area_key != pick.area
        && let Some(pick) = editor.pick.as_mut()
    {
        pick.area = area_key.clone();
    }
    let Some(area_key) = area_key else {
        return;
    };
    let Some(area_id) = content.by_path::<area::AreaDef>(&area_key) else {
        return;
    };
    let area = area::load(&content, area_id);
    let width = ui.available_width();
    let height = (width * area.size.height / area.size.width.max(1.0)).clamp(80.0, 360.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let zoom = navigation::fit_zoom(rect.size(), Vec2::new(area.size.width, area.size.height));
    let center = Pos::new(area.size.width / 2.0 - 0.5, area.size.height / 2.0 - 0.5);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::INPUT);
    crate::thumbs::areas::draw_area(
        editor,
        &painter,
        &area_key,
        rect,
        center,
        zoom * PX_PER_METER,
    );
    let canvas = navigation::Viewport {
        screen: rect,
        center,
        zoom,
    };
    for arrival in arrivals_at_hand(editor, &area_key) {
        let at = canvas.to_screen(arrival.at);
        painter.circle_filled(at, 3.0, egui::Color32::from_rgb(255, 190, 90));
        painter.circle_stroke(at, 3.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
    }
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let alt = ui.input(|i| EditorKey::Unsnapped.held(i));
    if let Some(pointer) = response.hover_pos() {
        let ground = canvas.to_ground(pointer);
        let picked = PickedPlace::at(&content, area_id, ground, alt);
        let at = picked.at;
        let (ok, why) = match &picked.verdict {
            Ok(()) => (true, "outside every warp object".to_owned()),
            Err(problem) => (false, problem.expected.clone()),
        };
        painter.circle_stroke(
            canvas.to_screen(at),
            5.0,
            egui::Stroke::new(2.0, picked.color()),
        );
        response.clone().on_hover_ui(|ui| {
            ui.label(theme::strong(format!(
                "{}, {} · {}",
                forms::metres(at.x),
                forms::metres(at.y),
                if ok { "walkable" } else { "refused" }
            )));
            ui.label(theme::muted(format!("{why} · click to pick")).size(12.0));
        });
        if response.clicked() {
            picked.commit(editor, &area_key);
        }
    }
    form::hint(
        ui,
        &format!(
            "Click a cell for its centre, {} click for any point, in 1/16 m. {} keeps the old place.",
            EditorKey::Unsnapped.name(),
            EditorKey::Cancel.name()
        ),
    );
}
