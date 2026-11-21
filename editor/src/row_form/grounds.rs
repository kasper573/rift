//! The previews of what lies on the ground: a water over each bed it lies on, from its shore out
//! past its clarity, and a terrain dry and under a metre of a water, each drawn by the game's own
//! renderer under a light picked beside it.

use engine::core::content::{ContentFile, NamedContent};
use engine::core::math::Pos;
use engine::core::units::{HourOfDay, Meters, Radians};
use engine::systems::area::{
    self, AreaDef, PICTURED_BED, PICTURED_CLARITIES, PICTURED_REACH, PICTURED_SHORE,
    PICTURED_UNDER, Pictured, TerrainDef, Weather,
};
use engine::systems::water::WaterDef;
use strum::{AsRefStr, VariantArray};
use ui::egui::{self, Color32, Ui, Vec2};
use ui::{chip, form, sheet, theme};

use crate::app::Editor;
use crate::forms::FormCx;
use crate::picker;
use crate::tabs::file_title;
use crate::views::area::camera::{LENS, OrbitCamera, PITCH};
use crate::views::stage::{self as view_stage, StageLight, StageState, Turntable};

/// The light a ground's preview is drawn under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, VariantArray, AsRefStr)]
#[strum(serialize_all = "lowercase")]
pub enum PreviewLight {
    #[default]
    Noon,
    Sunset,
    Night,
    Overcast,
}

impl PreviewLight {
    /// The hour and the weather the preview is lit at, `fair` being the weather it has when none
    /// is picked.
    fn lit(self, fair: Weather) -> StageLight {
        let (hour, weather) = match self {
            PreviewLight::Noon => (HourOfDay::NOON, fair),
            PreviewLight::Sunset => (HourOfDay(17.5), fair),
            PreviewLight::Night => (HourOfDay(0.0), fair),
            PreviewLight::Overcast => (
                HourOfDay::NOON,
                Weather {
                    clouds: 1.0,
                    water: 0.6,
                    ..fair
                },
            ),
        };
        StageLight {
            hour: Some(hour),
            weather: Some(weather),
        }
    }
}

/// A water as the game draws it: over each bed it lies on in the areas that lay it, from the
/// shore out to twice its clarity, and the areas a change to it reaches.
pub(super) fn show_water(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview stands once the content loads"));
        return;
    };
    let Some(water) = content.by_path::<WaterDef>(key) else {
        ui.label(theme::muted("the preview stands once the row loads"));
        return;
    };
    let clarity = water.get(&content).clarity;
    let beds = area::pictured_beds(&content, water);
    sheet::section(ui, "over each bed it lies on");
    lights(editor, ui);
    let view_key = format!("preview:{}/{key}", WaterDef::TABLE);
    let across = PICTURED_SHORE.0 + PICTURED_REACH.0;
    let down = (beds.len() as f32 * PICTURED_BED.0).max(engine::core::ground::MIN_SIDE as f32);
    let width = ui.available_width();
    let turntable = turntable(
        editor,
        ui,
        view_key,
        Vec2::new(width, (width * 0.7).min(420.0)),
        (Pos::new(across / 2.0, down / 2.0), across),
        Pictured::Water(water),
    );
    if let Some(turntable) = turntable {
        let painter = ui.painter_at(turntable.rect);
        let font = egui::FontId::proportional(11.5);
        for (band, bed) in beds.iter().enumerate() {
            let at = Pos::new(PICTURED_SHORE.0 / 2.0, (band as f32 + 0.5) * PICTURED_BED.0);
            painter.text(
                turntable.canvas.to_screen(at),
                egui::Align2::CENTER_CENTER,
                bed.get(&content).shown_name(),
                font.clone(),
                Color32::WHITE,
            );
        }
        let deepest = Meters(clarity.0 * PICTURED_CLARITIES);
        let marks = [
            (Meters(0.0), "shore".to_owned()),
            (Meters(clarity.0 / 4.0), metres_deep(clarity.0 / 4.0)),
            (clarity, format!("{}: clarity", metres_deep(clarity.0))),
            (deepest, metres_deep(deepest.0)),
        ];
        for (depth, said) in marks {
            let out = area::pictured_out(clarity, depth);
            let near = Pos::new(PICTURED_SHORE.0 + out.0, down);
            let far = Pos::new(PICTURED_SHORE.0 + out.0, 0.0);
            let (near, far) = (
                turntable.canvas.lifted(near, Meters(0.0)),
                turntable.canvas.lifted(far, Meters(0.0)),
            );
            painter.line_segment(
                [near, far],
                egui::Stroke::new(1.0, Color32::from_white_alpha(70)),
            );
            painter.text(
                near + Vec2::new(0.0, 4.0),
                egui::Align2::CENTER_TOP,
                said,
                font.clone(),
                Color32::WHITE,
            );
        }
        stand(editor, ui, turntable);
    }
    sheet::section(ui, "laid in");
    let laid: Vec<String> = area::laying(&content, water)
        .map(|area| area.path(&content).to_owned())
        .collect();
    if laid.is_empty() {
        form::hint(
            ui,
            "No area lays this water yet: the Water tool of an area's view lays it.",
        );
    }
    let mut opened = None;
    ui.horizontal_wrapped(|ui| {
        for path in &laid {
            if chip::plain(ui, file_title(path)).clicked() {
                opened = Some(path.clone());
            }
        }
    });
    if let Some(path) = opened {
        editor.open_row(AreaDef::TABLE, &path);
    }
}

/// A terrain as the game draws it: a patch of it dry, and the same patch under a metre of a
/// water picked beside it.
pub(super) fn show_terrain(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview stands once the content loads"));
        return;
    };
    let Some(terrain) = content.by_path::<TerrainDef>(key) else {
        ui.label(theme::muted("the preview stands once the row loads"));
        return;
    };
    lights(editor, ui);
    patch(
        editor,
        ui,
        (key, "dry"),
        "dry",
        Pictured::Terrain(terrain, None),
    );
    let mut under = editor.views.preview.under.clone().unwrap_or_default();
    let mut actions = Vec::new();
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let project = project.as_ref().expect("a project");
        let mut cx = FormCx::schemaless(project, shapes, form, &mut actions)
            .editing(TerrainDef::TABLE, Some(key));
        form::kv(ui, "under", |ui| {
            if picker::row(ui, &mut cx, WaterDef::TABLE, &mut under, "/preview/under") {
                editor.views.preview.under = Some(under.clone());
            }
        });
    }
    for action in actions {
        crate::row_form::act(editor, action, TerrainDef::TABLE, key);
    }
    let Some(water) = content.by_path::<WaterDef>(&under) else {
        form::hint(ui, "Pick a water to see this ground through it.");
        return;
    };
    let said = format!("under {} m of water", PICTURED_UNDER.0);
    let pictured = Pictured::Terrain(terrain, Some(water));
    patch(editor, ui, (key, "wet"), &said, pictured);
}

/// A patch of a terrain under the heading `said`, on the stage the terrain `key` keeps for
/// `which` of its patches.
fn patch(
    editor: &mut Editor,
    ui: &mut Ui,
    (key, which): (&str, &str),
    said: &str,
    pictured: Pictured,
) {
    sheet::section(ui, said);
    let view_key = format!("preview:{}/{key}/{which}", TerrainDef::TABLE);
    let width = ui.available_width();
    let turntable = turntable(
        editor,
        ui,
        view_key,
        Vec2::new(width, (width * 0.6).min(280.0)),
        (view_stage::center(), PATCH_SEEN),
        pictured,
    );
    if let Some(turntable) = turntable {
        stand(editor, ui, turntable);
    }
}

/// The stage `view_key` keeps, `size` large, drawing `pictured`: first looked down on at `at`
/// from far enough to take in `across` of it.
fn turntable(
    editor: &mut Editor,
    ui: &mut Ui,
    view_key: String,
    size: Vec2,
    (at, across): (Pos<Meters>, f32),
    pictured: Pictured,
) -> Option<Turntable> {
    let mut state = editor
        .views
        .preview
        .stages
        .remove(&view_key)
        .unwrap_or_else(|| StageState::looking(looking_down_on(at, across)));
    let turntable = view_stage::turntable(editor, ui, &view_key, size, &mut state, pictured);
    editor.views.preview.stages.insert(view_key, state);
    turntable
}

/// Stands the ground of `turntable` bare, in the light the previews are set to on a fair day.
fn stand(editor: &mut Editor, ui: &Ui, turntable: Turntable) {
    let fair = turntable.canvas.area().climate.first();
    let lit = editor.views.preview.light.lit(fair);
    turntable.stand(editor, ui, Vec::new(), lit);
}

/// How much of a terrain's patch its preview takes in, across.
const PATCH_SEEN: f32 = 5.0;

/// The chips picking the light the previews are drawn under.
fn lights(editor: &mut Editor, ui: &mut Ui) {
    let picked = &mut editor.views.preview.light;
    ui.horizontal(|ui| {
        for light in PreviewLight::VARIANTS {
            if chip::mode(ui, light.as_ref(), picked == light).clicked() {
                *picked = *light;
            }
        }
    });
}

/// A camera looking down at `at` from the editor's pitch, from as far as takes in `across`
/// metres of ground.
fn looking_down_on(at: Pos<Meters>, across: f32) -> OrbitCamera {
    OrbitCamera {
        target: at,
        yaw: Radians(0.0),
        pitch: PITCH.radians(),
        distance: Meters(across / 2.0 / (LENS.radians().0 / 2.0).tan()),
    }
}

fn metres_deep(depth: f32) -> String {
    format!("{} m", crate::forms::metres((depth * 10.0).round() / 10.0))
}
