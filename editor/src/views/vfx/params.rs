//! The parameters a vfx or shader row declares, and the one set of controls that fills them: the
//! preview's knobs, a shader layer's fills, and a script's values for a vfx.

use engine::core::content::{ContentFile, serial};
use engine::core::math::Rgba;
use engine::core::vfx::{
    ParamFill, ParamKind, ParamValue, ShaderParam, VfxDef, VfxParam, VfxShaderDef,
};
use engine::field;
use serde_json::{Map, Value};
use strum::VariantArray;
use ui::chip;
use ui::controls;
use ui::egui::{Color32, Ui};
use ui::form;
use ui::glyph::{self, Mark};
use ui::theme;

use crate::forms::{self, FormChange, FormCx};
use crate::project::Project;
use ui::input::{NumberField, SliderField};

const PARAM_INDENT: f32 = 22.0;
const BOUND_WIDTH: f32 = 64.0;
const SOURCE_WIDTH: f32 = 110.0;
const VALUE_AND_RESET: f32 = 110.0;
const MIN_SLIDER: f32 = 60.0;

/// A parameter as a row declares it: the kind of value that fills it, the value when nothing
/// does, and for a ranged number, the slider's bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    pub default: ParamValue,
    pub range: Option<(f32, f32)>,
}

impl From<&ShaderParam> for ParamSpec {
    fn from(param: &ShaderParam) -> ParamSpec {
        ParamSpec {
            name: param.name.clone(),
            kind: param.kind,
            default: param.default,
            range: param.range(),
        }
    }
}

impl From<&VfxParam> for ParamSpec {
    fn from(param: &VfxParam) -> ParamSpec {
        ParamSpec {
            name: param.name.clone(),
            kind: param.kind,
            default: param.kind.blank(),
            range: None,
        }
    }
}

/// What declares parameters: a vfx, or a shader a vfx layers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ParamOwner {
    Vfx,
    Shader,
}

impl ParamOwner {
    pub fn of_table(table: &str) -> Option<ParamOwner> {
        match table {
            VfxDef::TABLE => Some(ParamOwner::Vfx),
            VfxShaderDef::TABLE => Some(ParamOwner::Shader),
            _ => None,
        }
    }
}

/// The parameters the `owner` row at `key` declares, when it reads.
pub fn params_of(project: &Project, owner: ParamOwner, key: &str) -> Vec<ParamSpec> {
    match owner {
        ParamOwner::Shader => project.read::<VfxShaderDef>(key).map_or_else(
            |_| Vec::new(),
            |shader| shader.params.iter().map(ParamSpec::from).collect(),
        ),
        ParamOwner::Vfx => project.read::<VfxDef>(key).map_or_else(
            |_| Vec::new(),
            |vfx| vfx.params.iter().map(ParamSpec::from).collect(),
        ),
    }
}

/// One knob for a parameter's value, by its kind: a checkbox, a slider within the range or a
/// number field without one, or a color button. Says whether it moved.
pub(crate) fn param_control(ui: &mut Ui, spec: &ParamSpec, value: &mut ParamValue) -> bool {
    match spec.kind {
        ParamKind::Bool => {
            let mut flag = matches!(value, ParamValue::Bool(true));
            if ui.checkbox(&mut flag, "").changed() {
                *value = ParamValue::Bool(flag);
                return true;
            }
        }
        ParamKind::Number => {
            let mut amount = match value {
                ParamValue::Number(number) => *number,
                _ => 1.0,
            };
            let moved = match spec.range {
                Some((min, max)) => SliderField::new(&mut amount, min..=max).show(ui),
                None => NumberField::new(&mut amount).width(BOUND_WIDTH).show(ui),
            };
            if moved {
                *value = ParamValue::Number(tidy(f64::from(amount)) as f32);
                return true;
            }
        }
        ParamKind::Color => {
            let held = match value {
                ParamValue::Color(color) => *color,
                _ => Rgba::WHITE,
            };
            let mut color = Color32::from(held);
            if ui.color_edit_button_srgba(&mut color).changed() {
                *value = ParamValue::Color(Rgba::from(color.to_srgba_unmultiplied()));
                return true;
            }
        }
    }
    false
}

/// A shader layer's fill for one of its shader's parameters: a value of its own, or one of the
/// owning vfx row's parameters of the same kind to take it from. A parameter left at its
/// default writes nothing.
pub fn param_fill(
    ui: &mut Ui,
    cx: &mut FormCx,
    spec: &ParamSpec,
    fills: &mut Map<String, Value>,
    path: &str,
) -> FormChange {
    let fill = fills
        .get(&spec.name)
        .and_then(|fill| serde_json::from_value::<ParamFill>(fill.clone()).ok());
    let bindable: Vec<String> = match (&cx.row_table, &cx.row_key) {
        (Some(table), Some(key)) if table == VfxDef::TABLE => {
            params_of(cx.project, ParamOwner::Vfx, key)
                .into_iter()
                .filter(|own| own.kind == spec.kind)
                .map(|own| own.name)
                .collect()
        }
        _ => Vec::new(),
    };
    let mut changed = FormChange::default();
    let mut set = |fills: &mut Map<String, Value>, next: Option<ParamFill>| {
        match next {
            Some(next) => fills.insert(
                spec.name.clone(),
                serial::json_of(&next).unwrap_or(Value::Null),
            ),
            None => fills.remove(&spec.name),
        };
        changed = FormChange::at(path);
    };
    ui.horizontal(|ui| {
        forms::label_column(ui).label(theme::muted(&spec.name).monospace().size(13.0));
        let beside = if bindable.is_empty() {
            VALUE_AND_RESET
        } else {
            VALUE_AND_RESET + SOURCE_WIDTH
        };
        ui.spacing_mut().slider_width = (ui.available_width() - beside).max(MIN_SLIDER);
        match &fill {
            Some(ParamFill::Param { param }) => {
                chip::plain(ui, format!("◇ {param}"));
            }
            other => {
                let mut value = match other {
                    Some(ParamFill::Value(value)) => *value,
                    _ => spec.default,
                };
                if other.is_none() {
                    ui.style_mut().visuals.override_text_color = Some(theme::MUTED);
                }
                if param_control(ui, spec, &mut value) {
                    set(fills, Some(ParamFill::Value(value)));
                }
            }
        }
        if !bindable.is_empty() {
            let source = match &fill {
                Some(ParamFill::Param { param }) => format!("◇ {param}"),
                _ => "value".to_owned(),
            };
            controls::combo(ui.id().with(path).with("source"))
                .width(SOURCE_WIDTH)
                .selected_text(theme::mono(source).size(12.5))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, "value").clicked() {
                        set(fills, Some(ParamFill::Value(spec.default)));
                    }
                    for param in &bindable {
                        if ui.selectable_label(false, format!("◇ {param}")).clicked() {
                            set(
                                fills,
                                Some(ParamFill::Param {
                                    param: param.clone(),
                                }),
                            );
                        }
                    }
                });
        }
        if fill.is_some()
            && glyph::button(ui, Mark::Undo, 12.0, theme::FAINT, "Default")
                .on_hover_text("Back to the shader's default")
                .clicked()
        {
            set(fills, None);
        }
    });
    forms::error_line(ui, cx, path);
    changed
}

/// The parameters of the shader row `value`: each a name, a kind, a default, and for a number its
/// range.
pub fn shader_params(ui: &mut Ui, cx: &mut FormCx, value: &mut Value) -> FormChange {
    let path = field!(VfxShaderDef, params);
    forms::of_row(ui, cx, value, path, |ui, cx, shader: &mut VfxShaderDef| {
        params(ui, cx, &mut shader.params, path)
    })
}

fn params(ui: &mut Ui, cx: &mut FormCx, params: &mut Vec<ShaderParam>, path: &str) -> FormChange {
    let mut changed = FormChange::default();
    form::section_count(ui, "params", params.len());
    let reader = cx.project.reader();
    let mut remove = None;
    for (index, param) in params.iter_mut().enumerate() {
        let param_path = format!("{path}/{index}");
        let kept = reader
            .to_json(&*param)
            .is_ok_and(|held| cx.locks.iter().any(|lock| lock.keeps(path, &held)));
        ui.horizontal(|ui| {
            if kept {
                ui.label(theme::faint("·")).on_hover_text(forms::KEPT_TIP);
            } else if controls::remove(ui, "Remove").clicked() {
                remove = Some(index);
            }
            let mut picked = param.kind;
            ui.add_enabled_ui(!kept, |ui| {
                ui.scope(|ui| {
                    ui.set_width(120.0);
                    let name_path = format!("{param_path}{}", field!(ShaderParam, name));
                    changed |= forms::text_of(ui, cx, &mut param.name, &name_path);
                });
                controls::combo(("param-kind", index))
                    .width(90.0)
                    .selected_text(theme::mono(kind_name(param.kind)).size(13.0))
                    .show_ui(ui, |ui| {
                        for &kind in ParamKind::VARIANTS {
                            ui.selectable_value(&mut picked, kind, kind_name(kind));
                        }
                    });
            });
            if picked != param.kind {
                *param = fresh_param(std::mem::take(&mut param.name), picked);
                changed = FormChange::at(&param_path);
            }
            let default_path = format!("{param_path}{}", field!(ShaderParam, default));
            match &mut param.default {
                ParamValue::Color(color) => {
                    let mut shown = Color32::from(*color);
                    if ui.color_edit_button_srgba(&mut shown).changed() {
                        *color = Rgba::from(shown.to_srgba_unmultiplied());
                        changed |= FormChange::at(&default_path);
                    }
                }
                ParamValue::Bool(flag) => {
                    if ui.checkbox(flag, "").changed() {
                        changed |= FormChange::at(&default_path);
                    }
                }
                ParamValue::Number(_) => {}
            }
        });
        forms::error_line(
            ui,
            cx,
            &format!("{param_path}{}", field!(ShaderParam, name)),
        );
        if param.kind == ParamKind::Number {
            changed |= number_param_fields(ui, cx, param, &param_path);
        }
    }
    if let Some(index) = remove {
        params.remove(index);
        changed = FormChange::at(path);
    }
    forms::error_line(ui, cx, path);
    if ui.button("+ param").clicked() {
        let name = (1..)
            .map(|n| format!("param_{n}"))
            .find(|name| params.iter().all(|param| param.name != *name))
            .unwrap_or_default();
        params.push(ShaderParam {
            default: ParamValue::Number(1.0),
            max: Some(2.0),
            ..fresh_param(name, ParamKind::Number)
        });
        changed = FormChange::at(path);
    }
    changed
}

/// A parameter of `kind` as one is first declared: a number from 0 to 1, a white color, a flag
/// left off.
fn fresh_param(name: String, kind: ParamKind) -> ShaderParam {
    let number = kind == ParamKind::Number;
    ShaderParam {
        name,
        kind,
        default: match kind {
            ParamKind::Number => ParamValue::Number(0.0),
            ParamKind::Bool => ParamValue::Bool(false),
            ParamKind::Color => ParamValue::Color(Rgba::WHITE),
        },
        min: number.then_some(0.0),
        max: number.then_some(1.0),
    }
}

fn kind_name(kind: ParamKind) -> &'static str {
    kind.into()
}

fn number_param_fields(
    ui: &mut Ui,
    cx: &mut FormCx,
    param: &mut ShaderParam,
    path: &str,
) -> FormChange {
    let mut changed = FormChange::default();
    ui.horizontal(|ui| {
        ui.add_space(PARAM_INDENT);
        for (bound, held, at) in [
            ("min", &mut param.min, field!(ShaderParam, min)),
            ("max", &mut param.max, field!(ShaderParam, max)),
        ] {
            ui.label(theme::faint(bound).size(12.0));
            let mut amount = held.unwrap_or_default();
            if NumberField::new(&mut amount).width(BOUND_WIDTH).show(ui) {
                *held = Some(tidy(f64::from(amount)) as f32);
                changed |= FormChange::at(&format!("{path}{at}"));
            }
        }
        ui.label(theme::faint("default").size(12.0));
        let range = param.range().filter(|(min, max)| min < max);
        let mut amount = match param.default {
            ParamValue::Number(number) => number,
            _ => 0.0,
        };
        let moved = match range {
            Some((min, max)) => SliderField::new(&mut amount, min..=max).show(ui),
            None => NumberField::new(&mut amount).width(BOUND_WIDTH).show(ui),
        };
        if moved {
            param.default = ParamValue::Number(tidy(f64::from(amount)) as f32);
            changed |= FormChange::at(&format!("{path}{}", field!(ShaderParam, default)));
        }
    });
    for at in [
        field!(ShaderParam, min),
        field!(ShaderParam, max),
        field!(ShaderParam, default),
    ] {
        forms::error_line(ui, cx, &format!("{path}{at}"));
    }
    changed
}

/// A slider's position as a number worth writing into a row: four decimals, no float noise.
fn tidy(number: f64) -> f64 {
    (number * 10_000.0).round() / 10_000.0
}
