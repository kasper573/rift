//! The climate page of an area's view: the spells of weather the area turns through, in their
//! order, the first the one a server starts on; how the course turns; and how the area's rain and
//! lightning fall. The spell picked is edited in the inspector, and the area beside it is shown
//! under its weather.

use engine::core::content::ContentFile;
use engine::systems::area::{AreaDef, Climate, NextSpell, Spell, SpellLength, Weather};
use ui::egui::{self, Ui};
use ui::input::InputField;
use ui::{chip, controls, draft, form, panel, sheet, theme, words};

use crate::app::Editor;
use crate::forms::{self, FormChange, FormCx};
use crate::project::ContentEdit;
use crate::row_form;
use crate::tabs::file_title;

/// What the climate page works on: a spell, by its place among the climate's, the rain, or the
/// lightning; and the spell whose weather the area is shown under.
#[derive(Default)]
pub struct ClimatePage {
    pub picked: ClimatePick,
    /// The spell shown, by its place: the one last picked.
    pub shown: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClimatePick {
    #[default]
    Spells,
    Rain,
    Lightning,
}

impl ClimatePage {
    /// The weather the area is shown under on the page: the spell shown's, else the first's.
    pub fn weather(&self, climate: &Climate) -> Weather {
        climate
            .spells
            .get(self.shown)
            .map_or_else(|| climate.first(), |spell| spell.weather)
    }

    pub(crate) fn pick_spell(&mut self, index: usize) {
        self.picked = ClimatePick::Spells;
        self.shown = index;
    }
}

/// The page's left: the spells in their order, how the course turns, and the rain and lightning.
pub fn show(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let bundle = editor.views.area.view(view_key).bundle.clone();
    let Some(Ok(area)) = editor.read::<AreaDef>(&bundle) else {
        ui.label(theme::muted("This area doesn't load; Output says why.").size(12.5));
        return;
    };
    let mut climate = area.climate.clone();
    let page = &editor.views.area.view(view_key).climate;
    let (picked, shown) = (
        page.picked,
        page.shown.min(climate.spells.len().saturating_sub(1)),
    );
    let mut pick = None;
    let added = sheet::section_with(ui, "Spells", |ui| {
        controls::button(ui, "Add")
            .on_hover_text("a copy of the spell picked, to change")
            .clicked()
    });
    form::hint(
        ui,
        "The weather turns from spell to spell, each giving way to one of those that may follow \
         it. A server starts on the first, and one that holds the weather still keeps it.",
    );
    let count = climate.spells.len();
    let mut moved = None;
    for (index, spell) in climate.spells.iter().enumerate() {
        let selected = picked == ClimatePick::Spells && index == shown;
        let response = panel::tile_across(ui, selected, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::strong(&spell.name));
                if index == 0 {
                    chip::plain(ui, "starts");
                }
                if let Some(to) = controls::reorder(ui, index, count) {
                    moved = Some((index, to));
                }
            });
            ui.label(theme::muted(summary(spell)).size(12.0));
            ui.label(theme::faint(followers(spell)).size(12.0));
        })
        .response
        .interact(egui::Sense::click());
        if response.clicked() {
            pick = Some(ClimatePick::Spells);
            editor
                .views
                .area
                .view_mut(view_key)
                .climate
                .pick_spell(index);
        }
    }
    if added && add_spell(editor, &bundle, shown) {
        editor
            .views
            .area
            .view_mut(view_key)
            .climate
            .pick_spell(shown + 1);
    }
    if let Some((from, to)) = moved
        && move_spell(editor, &bundle, from, to)
    {
        editor.views.area.view_mut(view_key).climate.pick_spell(to);
    }
    ui.add_space(8.0);
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        let mut cx = FormCx::schemaless(project, shapes, form, &mut actions)
            .editing(AreaDef::TABLE, Some(&bundle));
        sheet::section(ui, "Course");
        panel::tile_across(ui, false, |ui| {
            form::kv(ui, "turns over", |ui| {
                changed |= forms::of_type(ui, &mut cx, &mut climate.turns, "/climate/turns");
            });
            form::hint(ui, "How long a spell takes to give way to the next.");
            form::kv(ui, "seed", |ui| {
                changed |= forms::of_type(ui, &mut cx, &mut climate.seed, "/climate/seed");
            });
            form::hint(ui, "Which course the spells take, alike on every client.");
        });
    }
    ui.add_space(8.0);
    for (kind, title, about) in [
        (
            ClimatePick::Rain,
            "Rain",
            match &climate.rain {
                Some(rain) => format!(
                    "Falls from the clouds that hold water, on {} shell{}.",
                    rain.shells.len(),
                    words::plural(rain.shells.len())
                ),
                None => "None falls, however wet the clouds.".to_owned(),
            },
        ),
        (
            ClimatePick::Lightning,
            "Lightning",
            match &climate.lightning {
                Some(lightning) => format!(
                    "Strikes from storm cloud, flashing {} times the open sky's light.",
                    lightning.flash
                ),
                None => "None strikes, however stormy the clouds.".to_owned(),
            },
        ),
    ] {
        sheet::section(ui, title);
        let response = panel::tile_across(ui, picked == kind, |ui| {
            ui.label(theme::muted(about).size(12.0));
        })
        .response
        .interact(egui::Sense::click());
        if response.clicked() {
            pick = Some(kind);
        }
        ui.add_space(8.0);
    }
    if let Some(pick) = pick {
        editor.views.area.view_mut(view_key).climate.picked = pick;
    }
    for action in actions {
        row_form::act(editor, action, AreaDef::TABLE, &bundle);
    }
    if let Some(label) = changed.label(&bundle) {
        let edit = ContentEdit::new(label);
        editor.change::<AreaDef>(edit, &bundle, |area| {
            area.climate.turns = climate.turns;
            area.climate.seed = climate.seed;
            Ok(())
        });
    }
}

/// The page's inspector: the spell picked, the rain or the lightning, edited.
pub fn inspector(editor: &mut Editor, ui: &mut Ui, view_key: &str) {
    let bundle = editor.views.area.view(view_key).bundle.clone();
    let Some(Ok(area)) = editor.read::<AreaDef>(&bundle) else {
        return;
    };
    let mut climate = area.climate.clone();
    let page = &editor.views.area.view(view_key).climate;
    let (picked, shown) = (
        page.picked,
        page.shown.min(climate.spells.len().saturating_sub(1)),
    );
    let mut actions = Vec::new();
    let mut changed = FormChange::default();
    let mut change: Option<String> = None;
    let mut rename = None;
    let mut removed = false;
    {
        let Editor {
            project,
            form,
            shapes,
            ..
        } = editor;
        let Some(project) = project.as_ref() else {
            return;
        };
        let mut cx = FormCx::schemaless(project, shapes, form, &mut actions)
            .editing(AreaDef::TABLE, Some(&bundle));
        match picked {
            ClimatePick::Spells => {
                let names: Vec<String> = climate.spells.iter().map(|s| s.name.clone()).collect();
                let at = format!("/climate/spells/{shown}");
                let spell = &mut climate.spells[shown];
                sheet::section(ui, &spell.name);
                let mut renamed = None;
                form::kv(ui, "name", |ui| {
                    let id = ui.id().with(("spell-name", &bundle, shown));
                    renamed = draft::drafted(ui, id, &spell.name, |ui, draft| {
                        InputField::new(draft).show(ui)
                    })
                    .filter(|name| !name.is_empty() && !names.contains(name));
                });
                if renamed.is_some() {
                    rename = renamed;
                }
                ui.add_space(6.0);
                sheet::section(ui, "Weather");
                changed |=
                    forms::of_type(ui, &mut cx, &mut spell.weather, &format!("{at}/weather"));
                ui.add_space(6.0);
                sheet::section(ui, "Lasts");
                changed |= forms::of_type(ui, &mut cx, &mut spell.lasts, &format!("{at}/lasts"));
                ui.add_space(6.0);
                let add = sheet::section_with(ui, "Followed by", |ui| {
                    controls::button(ui, "Add").clicked()
                });
                if spell.next.is_empty() {
                    form::hint(ui, "Nothing: once it comes, this spell holds for good.");
                }
                let mut gone = None;
                for (follows, next) in spell.next.iter_mut().enumerate() {
                    let path = format!("{at}/next/{follows}");
                    ui.horizontal(|ui| {
                        let before = next.spell.clone();
                        controls::combo(("follows", &bundle, shown, follows))
                            .selected_text(next.spell.as_str())
                            .show_ui(ui, |ui| {
                                for name in &names {
                                    ui.selectable_value(&mut next.spell, name.clone(), name);
                                }
                            });
                        if next.spell != before {
                            changed = FormChange::at(&format!("{path}/spell"));
                        }
                        changed |= forms::of_type(
                            ui,
                            &mut cx,
                            &mut next.weight,
                            &format!("{path}/weight"),
                        );
                        if controls::remove(ui, "follows no more").clicked() {
                            gone = Some(follows);
                        }
                    });
                }
                form::hint(
                    ui,
                    "Which spell comes next is drawn by these weights: one weighing twice \
                     another comes twice as often.",
                );
                if let Some(gone) = gone {
                    let next = spell.next.remove(gone);
                    change = Some(format!("{} no longer follows {}", next.spell, spell.name));
                }
                if add {
                    let candidate = names
                        .iter()
                        .find(|name| !spell.next.iter().any(|next| &next.spell == *name))
                        .cloned();
                    if let Some(candidate) = candidate {
                        change = Some(format!("{candidate} follows {}", spell.name));
                        spell.next.push(NextSpell {
                            spell: candidate,
                            weight: 1.0,
                        });
                    }
                }
                if climate.spells.len() > 1 {
                    ui.add_space(10.0);
                    if controls::danger(ui, "Remove this spell").clicked() {
                        removed = true;
                    }
                }
            }
            ClimatePick::Rain => {
                sheet::section(ui, "Rain");
                changed |= forms::of_type(ui, &mut cx, &mut climate.rain, "/climate/rain");
            }
            ClimatePick::Lightning => {
                sheet::section(ui, "Lightning");
                changed |=
                    forms::of_type(ui, &mut cx, &mut climate.lightning, "/climate/lightning");
            }
        }
    }
    for action in actions {
        row_form::act(editor, action, AreaDef::TABLE, &bundle);
    }
    if let Some(label) = change.or_else(|| changed.label(&bundle)) {
        write(editor, &bundle, &area, climate, &label);
    }
    if let Some(name) = rename {
        rename_spell(editor, &bundle, shown, &name);
    }
    if removed && remove_spell(editor, &bundle, shown) {
        editor
            .views
            .area
            .view_mut(view_key)
            .climate
            .pick_spell(shown.saturating_sub(1));
    }
}

/// Adds a copy of the spell at `index` of the area at `bundle` right after it, under a name of
/// its own, to be changed from there.
pub fn add_spell(editor: &mut Editor, bundle: &str, index: usize) -> bool {
    let edit = ContentEdit::new(format!("Add a spell to {}", file_title(bundle)));
    editor.change::<AreaDef>(edit, bundle, |area| {
        let climate = &mut area.climate;
        let mut copy = climate
            .spells
            .get(index)
            .cloned()
            .ok_or("there is no such spell")?;
        copy.name = unused_name(climate, &copy.name);
        climate.spells.insert(index + 1, copy);
        Ok(())
    })
}

/// Moves the spell at `from` among the area's spells to `to`; moved first, it is the one a server
/// starts on.
pub fn move_spell(editor: &mut Editor, bundle: &str, from: usize, to: usize) -> bool {
    let edit = ContentEdit::new(format!(
        "Move a spell {} in {}",
        if to < from { "earlier" } else { "later" },
        file_title(bundle)
    ));
    editor.change::<AreaDef>(edit, bundle, |area| {
        let spells = &mut area.climate.spells;
        if from >= spells.len() || to >= spells.len() {
            return Err("there is no such spell".to_owned());
        }
        let spell = spells.remove(from);
        spells.insert(to, spell);
        Ok(())
    })
}

/// Renames the spell at `index` to `name`, and every spell's naming of it as one that may follow.
pub fn rename_spell(editor: &mut Editor, bundle: &str, index: usize, name: &str) -> bool {
    let edit = ContentEdit::new(format!(
        "Rename a spell of {} to {name}",
        file_title(bundle)
    ));
    editor.change::<AreaDef>(edit, bundle, |area| {
        let spells = &mut area.climate.spells;
        if spells.iter().any(|spell| spell.name == name) {
            return Err(format!("a spell is called {name} already"));
        }
        let old = std::mem::replace(
            &mut spells.get_mut(index).ok_or("there is no such spell")?.name,
            name.to_owned(),
        );
        for next in spells.iter_mut().flat_map(|spell| &mut spell.next) {
            if next.spell == old {
                next.spell = name.to_owned();
            }
        }
        Ok(())
    })
}

/// Removes the spell at `index`, and every spell's naming of it as one that may follow. The last
/// spell stays, as a climate turns through one at least.
pub fn remove_spell(editor: &mut Editor, bundle: &str, index: usize) -> bool {
    let edit = ContentEdit::new(format!("Remove a spell of {}", file_title(bundle)));
    editor.change::<AreaDef>(edit, bundle, |area| {
        let spells = &mut area.climate.spells;
        if spells.len() < 2 {
            return Err("a climate keeps one spell at least".to_owned());
        }
        if index >= spells.len() {
            return Err("there is no such spell".to_owned());
        }
        let gone = spells.remove(index);
        for spell in spells.iter_mut() {
            spell.next.retain(|next| next.spell != gone.name);
        }
        Ok(())
    })
}

/// Writes `climate` as `area`'s, at `bundle`.
fn write(editor: &mut Editor, bundle: &str, area: &AreaDef, climate: Climate, label: &str) {
    let mut edited = area.clone();
    edited.climate = climate;
    editor.write_content(label, bundle, &edited);
}

/// `name` with the lowest number after it that no spell of `climate` is called yet.
fn unused_name(climate: &Climate, name: &str) -> String {
    (2..)
        .map(|number| format!("{name} {number}"))
        .find(|candidate| !climate.spells.iter().any(|spell| &spell.name == candidate))
        .unwrap_or_else(|| name.to_owned())
}

/// The spell's weather and how long it lasts, in a line.
fn summary(spell: &Spell) -> String {
    let weather = spell.weather;
    let mut parts = vec![format!("clouds {}", percent(weather.clouds))];
    if weather.rain > 0.0 {
        parts.push(format!("rain {}", percent(weather.rain)));
    }
    if weather.lightning > 0.0 {
        parts.push(format!("lightning {}/min", weather.lightning));
    }
    parts.push(lasting(spell.lasts));
    parts.join(" · ")
}

/// The spells that may follow, in a line.
fn followers(spell: &Spell) -> String {
    if spell.next.is_empty() {
        return "holds for good".to_owned();
    }
    let names: Vec<&str> = spell.next.iter().map(|next| next.spell.as_str()).collect();
    format!("then {}", names.join(" or "))
}

fn percent(share: f32) -> String {
    format!("{}%", (share * 100.0).round())
}

fn lasting(lasts: SpellLength) -> String {
    let minutes = |seconds: f32| (seconds / 60.0).round();
    let (shortest, longest) = (minutes(lasts.shortest.0), minutes(lasts.longest.0));
    if shortest == longest {
        format!("{shortest} min")
    } else {
        format!("{shortest}–{longest} min")
    }
}
