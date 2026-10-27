//! The clips page: the skeleton's libraries and every clip they carry, with its length, cues and
//! the set cells that play it; the chosen clip on the stage on any model wearing the skeleton,
//! holding any item, and laid out large to time its cues. Cues live on the skeleton, so one apex
//! serves every model that plays the clip; a model overriding the clip is listed.

use engine::core::content::Content;
use engine::core::content::NamedContent;
use engine::core::units::{Meters, MetersPerSec};
use engine::systems::actor::Action;
use engine::systems::actor::model::ActorModel;
use engine::systems::actor::skeleton::{BuiltSkeleton, ClipName, SkeletonId};
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{chip, form, panel, theme};

use crate::app::Editor;
use crate::tabs::shown_path;
use crate::views::cues;

pub fn show(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    id: SkeletonId,
    built: &BuiltSkeleton,
) {
    if built.clips.is_empty() {
        form::hint(
            ui,
            "No library carries a clip: drop a .glb of clips made on the rig onto this tab.",
        );
        return;
    }
    let chosen = super::played_clip(editor, key, built).expect("a library carries a clip");
    let width = ui.available_width();
    let list_width = (width * 0.34).clamp(220.0, 360.0);
    let mut pick: Option<ClipName> = None;
    let [left, right] = panel::split(ui, list_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("clips", key))
            .auto_shrink([false, false])
            .show(ui, |ui| pick = list(editor, ui, key, built, &chosen));
    });
    panel::body(ui, right, |ui| {
        super::stage_pickers(editor, ui, key, content, built, false);
        super::item_picker(editor, ui, key, content, "holding");
        let size = Vec2::new(
            ui.available_width(),
            (ui.available_height() * 0.5).max(140.0),
        );
        let props = super::holding(editor, key, content, id);
        super::stage_rig(editor, ui, key, content, built, size, props);
        ui.add_space(6.0);
        clip(editor, ui, key, content, id, built, &chosen);
    });
    if let Some(name) = pick {
        let view = editor.views.skeleton.view_mut(key);
        if view.clip.as_ref() != Some(&name) {
            view.clip = Some(name);
            view.scrub.restart();
        }
    }
    editor
        .status
        .left
        .push(format!("{} · {chosen}", shown_path(key)));
    editor.status.left.push(cues::KEYS.to_owned());
}

/// The libraries with their weight and clip count, then every clip with its length and uses.
fn list(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    built: &BuiltSkeleton,
    chosen: &ClipName,
) -> Option<ClipName> {
    let def = &built.def;
    form::section_count(ui, "Libraries", def.libraries.len());
    for (index, library) in def.libraries.iter().enumerate() {
        let count = built
            .clips
            .values()
            .filter(|clip| clip.library == index)
            .count();
        let weight = super::weight(editor, &library.file.path())
            .map_or_else(String::new, |weight| format!(" · {weight}"));
        ui.label(format!("clips/{}.glb · {count}{weight}", library.key));
    }
    ui.add_space(8.0);
    let view = editor.views.skeleton.view_mut(key);
    ui.horizontal(|ui| {
        form::section_count(ui, "Clips", built.clips.len());
        if chip::toggle(ui, "unused only", view.unused_only).clicked() {
            view.unused_only = !view.unused_only;
        }
    });
    let unused_only = view.unused_only;
    let mut pick = None;
    for (name, found) in &built.clips {
        let uses = uses(built, name);
        if unused_only && !uses.is_empty() {
            continue;
        }
        let cues = def.cues.get(name).map_or(0, Vec::len);
        let mut note = format!("{:.2} s", found.length.0);
        if cues > 0 {
            note.push_str(&format!(" · {cues} cues"));
        }
        note.push_str(&match uses.len() {
            0 => " · unused".to_owned(),
            1 => " · 1 cell".to_owned(),
            count => format!(" · {count} cells"),
        });
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::selectable(
                    name == chosen,
                    theme::mono(name.as_str()).size(12.5),
                ))
                .clicked()
            {
                pick = Some(name.clone());
            }
            ui.label(theme::faint(note).size(12.0));
        });
    }
    pick
}

/// The chosen clip laid out large to time, where it plays and who overrides it.
fn clip(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &Content,
    id: SkeletonId,
    built: &BuiltSkeleton,
    chosen: &ClipName,
) {
    let found = built.clips[chosen];
    let library = &built.def.libraries[found.library].key;
    ui.label(
        RichText::new(format!(
            "{chosen} · clips/{library}.glb · {:.2} s",
            found.length.0
        ))
        .strong(),
    );
    let held = built.def.cues.get(chosen).cloned().unwrap_or_default();
    let scrub = &mut editor.views.skeleton.view_mut(key).scrub;
    if let Some(cues) = cues::timeline(ui, scrub, found.length.millis(), &held, true) {
        super::set_cues(editor, key, chosen, cues);
    }
    pace(editor, ui, key, built, chosen);
    let uses = uses(built, chosen);
    let named = if uses.is_empty() {
        "no set names it: downloaded with its library and never played".to_owned()
    } else {
        let cells: Vec<String> = uses
            .iter()
            .map(|(set, entry)| {
                let set = match set {
                    Some(stance) => stance.get(content).shown_name().to_owned(),
                    None => "base".to_owned(),
                };
                format!("{set}: {entry}")
            })
            .collect();
        format!("used in {}", cells.join(" · "))
    };
    ui.label(theme::muted(named));
    let overriding: Vec<String> = super::wearers(content, id)
        .into_iter()
        .filter_map(|(path, model)| {
            let model: &ActorModel = model?;
            model
                .own_clips()
                .any(|(own, _)| own == chosen)
                .then(|| shown_path(&path).to_owned())
        })
        .collect();
    if !overriding.is_empty() {
        ui.label(theme::muted(format!(
            "overridden by {} (its own {chosen})",
            overriding.join(", ")
        )));
    }
}

/// The ground the clip covers, for a clip some set plays to walk, run or sprint or that has a pace.
fn pace(editor: &mut Editor, ui: &mut Ui, key: &str, built: &BuiltSkeleton, chosen: &ClipName) {
    let def = &built.def;
    let held = def.paces.get(chosen).copied();
    let gait = def.sets().any(|(_, set)| {
        Action::GAITS.iter().any(|gait| {
            set.clips
                .get(gait)
                .is_some_and(|clip| clip.clip() == chosen)
        })
    });
    if held.is_none() && !gait {
        return;
    }
    let mut value = held.map_or(0.0, |pace| pace.0.0);
    let dragged = ui
        .horizontal(|ui| {
            ui.label(theme::muted("pace · m/s"));
            let dragged = ui
                .add(
                    egui::DragValue::new(&mut value)
                        .speed(0.01)
                        .range(0.01..=20.0)
                        .fixed_decimals(2),
                )
                .changed();
            if held.is_none() {
                ui.label(theme::faint("none yet: a gait plays it"));
            }
            dragged
        })
        .inner;
    form::hint(
        ui,
        "How fast the clip carries a body at scale 1: the speed its planted feet slide back under \
         it. A body plays the gait nearest its speed, sped up or slowed to cover it exactly.",
    );
    if dragged {
        super::set_pace(editor, key, chosen, MetersPerSec(Meters(value)));
    }
}

/// The set cells that play `clip`: the stance of each set (none for base) and the entry naming it.
pub fn uses(
    built: &BuiltSkeleton,
    clip: &ClipName,
) -> Vec<(Option<engine::systems::actor::stance::StanceId>, String)> {
    let def = &built.def;
    let base = std::iter::once((None, &def.base));
    let stances = def.stances.iter().map(|set| (Some(set.stance), &set.set));
    base.chain(stances)
        .flat_map(|(stance, set)| {
            set.entries()
                .filter(|(_, named)| named.clip() == clip)
                .map(move |(entry, _)| {
                    let entry = entry.rsplit('/').next().unwrap_or_default().to_owned();
                    (stance, entry)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Every library clip no set names.
pub fn unused(built: &BuiltSkeleton) -> Vec<&ClipName> {
    built
        .clips
        .keys()
        .filter(|clip| uses(built, clip).is_empty())
        .collect()
}
