//! The vfx preview: a row's layers played on a site of the designer's choosing (an actor, an area,
//! a crowd in an area, or the screen), on the stage every preview shares.

use std::collections::{BTreeMap, HashMap};
use std::mem;
use ui::chip;
use ui::egui;
use ui::theme;

use engine::core::geometry::{Degrees, PlacedShape, Shape};
use engine::core::math::Pos;
use engine::core::render::ToScreen;
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::core::vfx::{ParamValue, VfxLayer, VfxParams};
use engine::systems::actor::{Action, ModelDef};
use strum::{EnumDiscriminants, IntoStaticStr, VariantArray};
use ui::controls;
use ui::egui::{Pos2, Stroke, Ui};

use super::params::{ParamSpec, param_control};
use crate::app::Editor;
use crate::stage::{self, Posed, PosedActor, PosedSite, PosedVfx, Scene};
use ui::input::NumberField;

const REST_BETWEEN_PLAYS: f32 = 0.5;
const SLIDER_SURROUND: f32 = 130.0;
const MOST_ACTORS: u32 = 100;
const FIRST_CROWD: u32 = 3;
const FIELD_WIDTH: f32 = 72.0;

/// The vfx previews: what the open row asked to see, and the values tried for its parameters in
/// place of their defaults, which the rest follow as they change.
pub struct VfxViews {
    target: VfxTarget,
    pub background: bool,
    started: Option<(String, f64)>,
    pub tried: HashMap<String, BTreeMap<String, ParamValue>>,
}

impl Default for VfxViews {
    fn default() -> VfxViews {
        VfxViews {
            target: VfxTarget::default(),
            background: true,
            started: None,
            tried: HashMap::new(),
        }
    }
}

/// What the vfx plays on: one actor, an area, actors standing in an area, or the whole screen.
#[derive(Clone, Debug, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(VfxTargetKind), derive(VariantArray, IntoStaticStr))]
enum VfxTarget {
    #[strum_discriminants(strum(serialize = "actor"))]
    Actor { model: String, action: Action },
    #[strum_discriminants(strum(serialize = "area"))]
    Area { shape: Shape, facing: Degrees },
    #[strum_discriminants(strum(serialize = "area + actors"))]
    Crowd {
        shape: Shape,
        facing: Degrees,
        count: u32,
        action: Action,
    },
    #[strum_discriminants(strum(serialize = "screen"))]
    Screen,
}

impl Default for VfxTarget {
    fn default() -> VfxTarget {
        VfxTarget::Actor {
            model: String::new(),
            action: Action::Idle,
        }
    }
}

pub fn preview(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    layers: Vec<VfxLayer>,
    inside: Option<Vec<VfxLayer>>,
    specs: &[ParamSpec],
    drafts: &[(String, String)],
) {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return;
    };
    let models: Vec<String> = content.table::<ModelDef>().paths().to_vec();
    let total = layers
        .iter()
        .map(|layer| layer.ends().0)
        .fold(0.5_f32, f32::max);
    let now = ui.input(|input| input.time);
    let views = &mut editor.views.vfx;
    if let VfxTarget::Actor { model, .. } = &mut views.target
        && model.is_empty()
        && let Some(first) = models.first()
    {
        *model = first.clone();
    }
    ui.horizontal_wrapped(|ui| {
        for &kind in VfxTargetKind::VARIANTS {
            let on = VfxTargetKind::from(&views.target) == kind;
            if chip::mode(ui, <&str>::from(kind), on).clicked() && !on {
                views.target = VfxTarget::fresh(kind, &models);
            }
        }
    });
    ui.horizontal_wrapped(|ui| match &mut views.target {
        VfxTarget::Actor { model, action } => {
            controls::combo("preview-model")
                .width(160.0)
                .selected_text(theme::mono(model.as_str()).size(13.0))
                .show_ui(ui, |ui| {
                    for candidate in &models {
                        ui.selectable_value(model, candidate.clone(), candidate);
                    }
                });
            ui.end_row();
            action_combo(ui, action);
        }
        VfxTarget::Area { shape, facing } => {
            shape_fields(ui, shape);
            facing_field(ui, facing);
        }
        VfxTarget::Crowd {
            shape,
            facing,
            count,
            action,
        } => {
            shape_fields(ui, shape);
            facing_field(ui, facing);
            ui.end_row();
            ui.label(theme::faint("actors").size(12.0));
            ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_SURROUND).max(60.0);
            let mut crowd = *count as f32;
            if controls::slider(ui, &mut crowd, 1.0..=MOST_ACTORS as f32).changed() {
                *count = crowd.round() as u32;
            }
            ui.label(theme::faint(count.to_string()).size(12.0));
            ui.end_row();
            action_combo(ui, action);
        }
        VfxTarget::Screen => {}
    });
    let started = match &views.started {
        Some((playing, started)) if playing == key => *started,
        _ => {
            views.started = Some((key.to_owned(), now));
            now
        }
    };
    let elapsed = ((now - started) as f32)
        .rem_euclid(total + REST_BETWEEN_PLAYS)
        .min(total);
    ui.horizontal(|ui| {
        ui.checkbox(&mut views.background, "background");
        ui.label(theme::faint(format!("{elapsed:.2} s of {total:.2}")).size(12.0));
    });
    let tried = views.tried.entry(key.to_owned()).or_default();
    if !specs.is_empty() && ui.button("reset to defaults").clicked() {
        tried.clear();
    }
    ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_SURROUND).max(60.0);
    let mut params = Vec::new();
    for spec in specs {
        let mut value = tried
            .get(&spec.name)
            .copied()
            .filter(|value| mem::discriminant(value) == mem::discriminant(&spec.default))
            .unwrap_or(spec.default);
        ui.horizontal(|ui| {
            ui.label(theme::muted(&spec.name).monospace().size(12.5));
            if param_control(ui, spec, &mut value) {
                tried.insert(spec.name.clone(), value);
            }
        });
        params.push((spec.name.clone(), value));
    }
    let params = VfxParams(params);
    let background = views.background;
    let target = views.target.clone();
    let center = stage::center(&content);
    let footprint = target.footprint(center);
    let mut posed = Posed::default();
    let on = match &target {
        VfxTarget::Actor { model, action } => {
            let Some(model) = content.by_path::<ModelDef>(model) else {
                ui.label(theme::warn("pick a model to play the vfx on"));
                return;
            };
            posed.actors.push(PosedActor {
                action: *action,
                ..PosedActor::standing(model, center)
            });
            PosedSite::Actor(0)
        }
        VfxTarget::Area { .. } => PosedSite::Area(footprint.expect("an area has a footprint")),
        VfxTarget::Crowd { count, action, .. } => {
            let footprint = footprint.expect("a crowd has a footprint");
            let models: Vec<_> = content.table::<ModelDef>().ids().collect();
            if !models.is_empty() {
                for (index, at) in stage::spread(&footprint, *count).into_iter().enumerate() {
                    posed.actors.push(PosedActor {
                        action: *action,
                        ..PosedActor::standing(models[index % models.len()], at)
                    });
                }
            }
            PosedSite::Area(footprint)
        }
        VfxTarget::Screen => PosedSite::Screen,
    };
    posed.vfx = Some(PosedVfx {
        layers,
        on,
        inside,
        params,
        elapsed: Seconds(elapsed),
        drafts: drafts.to_vec(),
    });
    let Some(shown) = stage::show(
        editor,
        ui,
        &format!("vfx/{key}"),
        Scene::Posed(posed),
        background,
    ) else {
        return;
    };
    if let Some(footprint) = footprint {
        let outline: Vec<Pos2> = footprint
            .outline()
            .into_iter()
            .map(|point| shown.screen(point.to_screen()))
            .collect();
        if let (Some(first), true) = (outline.first().copied(), outline.len() > 1) {
            let mut closed = outline.clone();
            closed.push(first);
            ui.painter().add(egui::Shape::dashed_line(
                &closed,
                Stroke::new(1.0, theme::WARN),
                4.0,
                3.0,
            ));
        }
    }
}

impl VfxTarget {
    fn fresh(kind: VfxTargetKind, models: &[String]) -> VfxTarget {
        match kind {
            VfxTargetKind::Actor => VfxTarget::Actor {
                model: models.first().cloned().unwrap_or_default(),
                action: Action::Idle,
            },
            VfxTargetKind::Area => VfxTarget::Area {
                shape: Shape::Circle {
                    radius: Meters(1.5),
                },
                facing: Degrees(0.0),
            },
            VfxTargetKind::Crowd => VfxTarget::Crowd {
                shape: Shape::Circle {
                    radius: Meters(2.5),
                },
                facing: Degrees(0.0),
                count: FIRST_CROWD,
                action: Action::Idle,
            },
            VfxTargetKind::Screen => VfxTarget::Screen,
        }
    }

    fn footprint(&self, at: Pos<Meters>) -> Option<PlacedShape> {
        match self {
            VfxTarget::Area { shape, facing } | VfxTarget::Crowd { shape, facing, .. } => {
                Some(PlacedShape {
                    at,
                    facing: facing.radians(),
                    shape: *shape,
                })
            }
            VfxTarget::Actor { .. } | VfxTarget::Screen => None,
        }
    }
}

fn action_combo(ui: &mut Ui, action: &mut Action) {
    controls::combo("preview-action")
        .width(90.0)
        .selected_text(theme::mono(action.as_ref()).size(13.0))
        .show_ui(ui, |ui| {
            for candidate in Action::VARIANTS {
                ui.selectable_value(action, *candidate, candidate.as_ref());
            }
        });
}

fn facing_field(ui: &mut Ui, facing: &mut Degrees) {
    NumberField::new(&mut facing.0)
        .range(0.0..=360.0)
        .unit("°")
        .width(FIELD_WIDTH)
        .show(ui);
}

fn shape_fields(ui: &mut Ui, shape: &mut Shape) {
    let kinds = ["circle", "rect", "cone", "line"];
    let current = match shape {
        Shape::Circle { .. } => "circle",
        Shape::Rect { .. } => "rect",
        Shape::Cone { .. } => "cone",
        Shape::Line { .. } => "line",
    };
    let mut picked = current;
    controls::combo("preview-shape")
        .width(80.0)
        .selected_text(theme::mono(current).size(13.0))
        .show_ui(ui, |ui| {
            for kind in kinds {
                ui.selectable_value(&mut picked, kind, kind);
            }
        });
    if picked != current {
        *shape = match picked {
            "rect" => Shape::Rect {
                width: Meters(3.0),
                height: Meters(2.0),
            },
            "cone" => Shape::Cone {
                angle: Degrees(90.0),
                length: Meters(2.5),
            },
            "line" => Shape::Line {
                length: Meters(4.0),
                width: Meters(1.0),
            },
            _ => Shape::Circle {
                radius: Meters(1.5),
            },
        };
    }
    let metres = |ui: &mut Ui, label: &str, value: &mut Meters| {
        ui.label(theme::faint(label).size(12.0));
        NumberField::new(&mut value.0)
            .range(0.1..=12.0)
            .width(FIELD_WIDTH)
            .show(ui);
    };
    match shape {
        Shape::Circle { radius } => metres(ui, "radius", radius),
        Shape::Rect { width, height } => {
            metres(ui, "width", width);
            metres(ui, "height", height);
        }
        Shape::Cone { angle, length } => {
            ui.label(theme::faint("angle").size(12.0));
            NumberField::new(&mut angle.0)
                .range(5.0..=360.0)
                .unit("°")
                .width(FIELD_WIDTH)
                .show(ui);
            metres(ui, "length", length);
        }
        Shape::Line { length, width } => {
            metres(ui, "length", length);
            metres(ui, "width", width);
        }
    }
}
