//! The game's day in the editor: the track its times of day are edited on, the picker that names
//! one, the strip that plots what follows the sky over a day, and where content names a time.

use std::collections::BTreeMap;
use std::sync::Arc;

use engine::core::content::{
    AssetKind, Content, ContentFile, FolderReader, ImageRef, SchemaMark, fields_file,
};
use engine::core::math::Rgb;
use engine::core::units::{DayHours, HourOfDay};
use engine::systems::GameSettings;
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::scene::{self, SceneNode};
use engine::systems::area::{AreaDef, DayCycle, TimeOfDay, TimeOfDayName, Weather};
use engine::systems::script::{Sky, SkyCondition};
use serde_json::Value;
use ui::egui::{self, Id, Sense, Ui, UiBuilder};
use ui::timespans::{self, Timespan, TimespanEdit};
use ui::{chip, controls, day_strip, form, theme};

use crate::forms::{FormAction, FormChange, FormCx};
use crate::picker;
use crate::project::files::content_of;
use crate::project::{ContentEdit, Project, scene_file};
use crate::schema;

/// The hours of a day, as the track and the strips run.
const DAY: std::ops::RangeInclusive<f32> = 0.0..=24.0;
/// How many times an hour a strip asks what it plots.
const PER_HOUR: usize = 4;

/// The game's times of day on one track: each dragged, split, renamed, coloured and given its
/// icon there, with what follows the picked one listed under it.
pub(crate) fn track(
    ui: &mut Ui,
    cx: &mut FormCx,
    times: &mut Vec<TimeOfDay>,
    path: &str,
) -> FormChange {
    let ctx = ui.ctx().clone();
    let spans: Vec<Timespan> = times
        .iter()
        .map(|time| Timespan {
            name: time.name.0.clone(),
            from: time.from.0,
            eases: time.eases.0,
            color: time.color.into(),
            icon: icon_of(cx, &ctx, &time.icon),
        })
        .collect();
    let mut changed = FormChange::default();
    ui.vertical(|ui| {
        if times.is_empty() {
            form::hint(
                ui,
                "The day has no times yet: double-click the track to begin one.",
            );
        }
        match timespans::timespans(ui, DAY, &spans) {
            Some(TimespanEdit::Added { at, from }) => {
                let like = times
                    .get(at.checked_sub(1).unwrap_or(times.len().saturating_sub(1)))
                    .cloned();
                times.insert(
                    at.min(times.len()),
                    TimeOfDay {
                        name: unused_name(times),
                        from: HourOfDay(from),
                        eases: DayHours(0.0),
                        color: like.as_ref().map_or(Rgb::WHITE, |like| like.color),
                        icon: like.map_or_else(|| ImageRef::new(String::new()), |like| like.icon),
                    },
                );
                changed = FormChange::at(path);
            }
            Some(TimespanEdit::Removed { at }) if at < times.len() => {
                times.remove(at);
                changed = FormChange::at(path);
            }
            Some(TimespanEdit::Renamed { at, name }) => {
                if times.iter().any(|time| time.name.0 == name) {
                    cx.state.status = vec![format!("A time of day is called {name} already")];
                } else if let Some(time) = times.get_mut(at) {
                    time.name = TimeOfDayName(name);
                    changed = FormChange::at(path);
                }
            }
            Some(TimespanEdit::Moved { at, from }) => {
                if let Some(time) = times.get_mut(at) {
                    time.from = HourOfDay(from);
                    changed = FormChange::at(path);
                }
            }
            Some(TimespanEdit::Eased { at, eases }) => {
                if let Some(time) = times.get_mut(at) {
                    time.eases = DayHours(eases);
                    changed = FormChange::at(path);
                }
            }
            Some(TimespanEdit::Coloured { at, color }) => {
                if let Some(time) = times.get_mut(at) {
                    time.color = Rgb([color.r(), color.g(), color.b()]);
                    changed = FormChange::at(path);
                }
            }
            Some(TimespanEdit::PickIcon { .. }) => cx.autofocus = true,
            Some(TimespanEdit::Removed { .. }) | None => {}
        }
        let Some((at, time)) = timespans::picked(ui).and_then(|at| Some((at, times.get_mut(at)?)))
        else {
            return;
        };
        form::kv(ui, "icon", |ui| {
            let mut file = time.icon.path();
            if picker::asset_of(
                ui,
                cx,
                AssetKind::Image,
                &mut file,
                &format!("{path}/{at}/icon"),
            ) {
                time.icon = ImageRef::new(file);
                changed = FormChange::at(path);
            }
        });
        followers(ui, cx, &time.name);
    });
    changed
}

/// One of the game's times of day, picked among them by their marks and hours.
pub(crate) fn pick_time(
    ui: &mut Ui,
    cx: &mut FormCx,
    time: &mut TimeOfDayName,
    path: &str,
) -> FormChange {
    let times = cx
        .project
        .settings()
        .ok()
        .and_then(|settings| settings.daylight.clone())
        .map(|cycle| cycle.times)
        .unwrap_or_default();
    let ctx = ui.ctx().clone();
    let mut changed = FormChange::default();
    let known = times.iter().any(|known| known.name == *time);
    let shown = if known {
        egui::RichText::new(&time.0)
    } else {
        theme::warn(&time.0)
    };
    controls::combo(ui.id().with(path).with("time"))
        .selected_text(shown)
        .show_ui(ui, |ui| {
            if times.is_empty() {
                form::hint(ui, "The game's settings name no times of day.");
            }
            for (at, known) in times.iter().enumerate() {
                let until = times[(at + 1) % times.len()].from.0;
                let text = format!("{} · {} to {}", known.name.0, known.from.0, until);
                let icon = icon_of(cx, &ctx, &known.icon);
                let row = ui
                    .scope_builder(UiBuilder::new().sense(Sense::click()), |ui| match icon {
                        Some(icon) => chip::emblem(ui, text, known.color.into(), icon),
                        None => chip::swatch(ui, text, known.color.into()),
                    })
                    .response
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if row.clicked() {
                    *time = known.name.clone();
                    changed = FormChange::at(path);
                    ui.close();
                }
            }
        });
    changed
}

/// How much of what follows `when` is there through a day, plotted over the game's times of day
/// under a calm sky; and a word when it never or always is, which is seldom meant.
pub(crate) fn strip(ui: &mut Ui, cx: &mut FormCx, when: &SkyCondition) {
    let Some(content) = cx.content.clone() else {
        return;
    };
    let Some(cycle) = daylight(&content) else {
        return;
    };
    let spans = cycle.timespans(|_| None);
    match shares(&content, when) {
        Ok(shares) => {
            day_strip::day_strip(ui, DAY, &spans, |hour| {
                let at = (hour * PER_HOUR as f32).round() as usize;
                shares[at.min(shares.len() - 1)]
            });
            if shares.iter().all(|share| *share <= 0.0) {
                form::hint(ui, "Never holds: nothing follows it at any hour.");
            } else if shares.iter().all(|share| *share >= 1.0) {
                form::hint(ui, "Always holds: the same as leaving it empty.");
            }
        }
        Err(fault) => form::error_line(ui, Some(&fault.to_string())),
    }
}

/// How much of what follows `when` is there at each step of a day in a game of `content`, under
/// a calm sky.
pub(crate) fn shares(content: &Content, when: &SkyCondition) -> Result<Vec<f32>, String> {
    let steps = 24 * PER_HOUR;
    let skies = (0..=steps).map(|step| Sky {
        hour: Some(HourOfDay::wrapped(step as f32 / PER_HOUR as f32)),
        weather: Weather::default(),
        rain: 0.0,
    });
    when.through(content, skies)
        .map_err(|fault| fault.to_string())
}

/// Where a content file names a time of day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeUse {
    pub file: String,
    pub pointer: String,
    pub time: TimeOfDayName,
}

/// Everywhere the project's content names a time of day, found by the mark such a field
/// carries in its schema.
pub(crate) fn uses(project: &Project) -> Vec<TimeUse> {
    let shapes = &project.shapes;
    let reader = project.reader();
    let mut uses = Vec::new();
    for (table, shape) in &shapes.tables {
        for path in project.files.paths_of(table) {
            let file = fields_file(&path);
            if let Some(json) = project.files.json(&file) {
                uses.extend(named_in(&reader, &file, "", &shape.shape, &json));
            }
            let area = (table == AreaDef::TABLE)
                .then(|| project.read::<AreaDef>(&path).ok())
                .flatten();
            let blueprint = (table == BlueprintDef::TABLE)
                .then(|| project.read::<BlueprintDef>(&path).ok())
                .flatten();
            let nodes: &[SceneNode] = match (&area, &blueprint) {
                (Some(area), _) => &area.scene,
                (_, Some(blueprint)) => &blueprint.scene,
                _ => &[],
            };
            let file = scene_file(&path);
            scene::walk(nodes, &mut |pointer, node, _| {
                if let SceneNode::Module(node) = node
                    && let Ok(json) = reader.to_json(&node.module)
                {
                    uses.extend(named_in(
                        &reader,
                        &file,
                        pointer,
                        &shapes.scene_module,
                        &json,
                    ));
                }
            });
        }
    }
    uses
}

/// Carries a change of the settings' times of day, from `before` to `after`, into everything
/// that names one: a time renamed is renamed there, and what followed a time taken away follows
/// the one before it. Answers how many places `edit` changes for it.
pub fn carry_names(
    project: &Project,
    edit: &mut ContentEdit,
    before: &[TimeOfDay],
    after: &[TimeOfDay],
) -> usize {
    let gone: Vec<&TimeOfDay> = before
        .iter()
        .filter(|time| !after.iter().any(|kept| kept.name == time.name))
        .collect();
    let came: Vec<&TimeOfDay> = after
        .iter()
        .filter(|time| !before.iter().any(|known| known.name == time.name))
        .collect();
    let (old, new) = match (
        gone.as_slice(),
        came.as_slice(),
        after.len() + 1 == before.len(),
    ) {
        ([old], [new], false) => (&old.name, new.name.clone()),
        ([old], [], true) => {
            let at = before
                .iter()
                .position(|time| time.name == old.name)
                .unwrap_or(0);
            let Some(heir) = after.get((at + after.len()).saturating_sub(1) % after.len().max(1))
            else {
                return 0;
            };
            (&old.name, heir.name.clone())
        }
        _ => return 0,
    };
    uses(project)
        .into_iter()
        .filter(|named| named.time == *old)
        .filter(|named| {
            edit.update_at(
                project,
                &named.file,
                &named.pointer,
                |time: &mut TimeOfDayName| {
                    *time = new.clone();
                },
            )
            .is_ok()
        })
        .count()
}

/// Where `json`, a value of the type whose schema is `root` kept at `prefix` in `file`, names a
/// time of day.
fn named_in(
    reader: &FolderReader,
    file: &str,
    prefix: &str,
    root: &Value,
    json: &Value,
) -> Vec<TimeUse> {
    schema::marked_fields(root, json, SchemaMark::TimeOfDay)
        .into_iter()
        .filter_map(|pointer| {
            let time = reader.from_json(json.pointer(&pointer)?).ok()?;
            Some(TimeUse {
                file: file.to_owned(),
                pointer: format!("{prefix}{pointer}"),
                time,
            })
        })
        .collect()
}

fn daylight(content: &Content) -> Option<&DayCycle> {
    content.try_built::<GameSettings>()?.daylight.as_ref()
}

fn icon_of(cx: &mut FormCx, ctx: &egui::Context, icon: &ImageRef) -> Option<egui::TextureId> {
    cx.state
        .thumbnails
        .image(ctx, cx.files, &icon.path())
        .map(|thumb| thumb.texture.id())
}

/// A name no time of the day has yet.
fn unused_name(times: &[TimeOfDay]) -> TimeOfDayName {
    (1..)
        .map(|n| TimeOfDayName(format!("Time {n}")))
        .find(|name| times.iter().all(|time| time.name != *name))
        .expect("some name is unused")
}

/// What follows the time of day `time`, file by file, each opened by a click.
fn followers(ui: &mut Ui, cx: &mut FormCx, time: &TimeOfDayName) {
    let revision = cx.files.revision();
    let id = Id::new("day::uses");
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<Arc<(u64, Vec<TimeUse>)>>(id))
        .filter(|cached| cached.0 == revision);
    let found = cached.unwrap_or_else(|| {
        let found = Arc::new((revision, uses(cx.project)));
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, found.clone()));
        found
    });
    let mut by_file: BTreeMap<&str, usize> = BTreeMap::new();
    for named in found.1.iter().filter(|named| named.time == *time) {
        *by_file.entry(named.file.as_str()).or_default() += 1;
    }
    let places: usize = by_file.values().sum();
    form::section_count(ui, &format!("Follows {}", time.0), places);
    if by_file.is_empty() {
        form::hint(ui, "Nothing follows this time of day yet.");
    }
    for (file, count) in by_file {
        let Some((table, path)) = content_of(file) else {
            continue;
        };
        let shown = format!("{} · {count}", crate::tabs::shown_path(path));
        if ui
            .link(shown)
            .on_hover_text(format!("Open the {table}"))
            .clicked()
        {
            cx.actions.push(FormAction::OpenRow {
                table: table.to_owned(),
                key: path.to_owned(),
            });
        }
    }
}
