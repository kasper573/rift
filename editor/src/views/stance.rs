//! The stance view: a stance is the name items and skeleton sets share. Its own fields are in the
//! row form; the view shows its paired table, the items that put a wearer in it, and every
//! skeleton with what its set for the stance names, or that it plays base.

use engine::core::content::{ContentFile, NamedContent};
use engine::systems::actor::model::Skeletons;
use engine::systems::actor::skeleton::SkeletonDef;
use engine::systems::actor::stance::{Pairing, StanceDef};
use engine::systems::item::{ItemDef, Wear};
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, sheet, theme};

use crate::app::Editor;
use crate::shell::row_form::usages;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;

/// Adds to the stance at `key` that beside an item of the stance at `with` it becomes the stance at
/// `becomes`, in place of what it became beside `with` before.
pub fn add_pair(editor: &mut Editor, key: &str, with: &str, becomes: &str) -> bool {
    let Some(Ok(row)) = editor.read::<StanceDef>(key) else {
        return false;
    };
    let Some(reader) = editor.reader() else {
        return false;
    };
    let (Some(with), Some(becomes)) = (
        reader.id_of::<StanceDef>(with),
        reader.id_of::<StanceDef>(becomes),
    ) else {
        editor.toast("a pair names two stances");
        return false;
    };
    let mut stance = (*row).clone();
    stance.paired.retain(|pairing| pairing.with != with);
    stance.paired.push(Pairing { with, becomes });
    editor.write_content(&format!("Pair {}", shown_path(key)), key, &stance)
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    if super::read_shown::<StanceDef>(editor, ui, key).is_none() {
        return;
    }
    sidebar::row(ui, "stance", |ui| {
        row_form::show_row(editor, ui, StanceDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "stance", |editor, ui| {
        usages::show_usages_grouped(editor, ui, key, true);
    });
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.label(theme::mono(shown_path(key)).color(theme::CODE));
            ui.add_space(4.0);
            let Some(content) = editor.loaded() else {
                return;
            };
            let Some(id) = content.by_path::<StanceDef>(key) else {
                form::hint(ui, "The stance did not load: the output panel says why.");
                return;
            };
            let def = id.get(&content);
            egui::ScrollArea::vertical()
                .id_salt(("stance", key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    sheet::section(ui, "paired");
                    paired(editor, ui, key, &content, def);
                    ui.add_space(10.0);
                    sheet::section(ui, "items in it");
                    let items: Vec<(String, String)> = content
                        .table::<ItemDef>()
                        .iter()
                        .filter(|(_, item)| {
                            matches!(item.wear(), Some(Wear::Held { stance, .. }) if *stance == id)
                        })
                        .map(|(item, row)| {
                            (item.path(&content).to_owned(), row.shown_name().to_owned())
                        })
                        .collect();
                    if items.is_empty() {
                        form::hint(ui, "No item puts its wearer in this stance.");
                    }
                    for (path, name) in items {
                        if ui.link(format!("{name} · {}", shown_path(&path))).clicked() {
                            editor.open_row(ItemDef::TABLE, &path);
                        }
                    }
                    ui.add_space(10.0);
                    sheet::section(ui, &format!("{} · where it is a set", def.shown_name()));
                    for (skeleton, row) in content.table::<SkeletonDef>().iter() {
                        let path = skeleton.path(&content).to_owned();
                        let line = match row.set(id) {
                            Some(set) => {
                                let mut named: Vec<String> = set
                                    .clips
                                    .keys()
                                    .map(|action| action.as_ref().to_owned())
                                    .collect();
                                named.extend(set.poses.keys().map(|pose| pose.as_ref().to_owned()));
                                let moves = set.moves.len();
                                let mut line = named.join(", ");
                                if moves > 0 {
                                    line.push_str(&format!(" + {moves} moves"));
                                }
                                if line.is_empty() {
                                    "a set naming nothing yet".to_owned()
                                } else {
                                    line
                                }
                            }
                            None => "no set · base".to_owned(),
                        };
                        let shows = Skeletons::loaded(&content, skeleton)
                            .is_some_and(|built| built.def.hands.len() >= def.hands.0 as usize);
                        ui.horizontal(|ui| {
                            if ui.link(row.shown_name()).on_hover_text(&path).clicked() {
                                editor.open_row(SkeletonDef::TABLE, &path);
                            }
                            let text = RichText::new(line).size(12.5);
                            ui.label(if row.set(id).is_some() {
                                text
                            } else {
                                text.color(theme::FAINT)
                            });
                            if !shows {
                                ui.label(theme::faint("· wears its items unseen"));
                            }
                        });
                    }
                    ui.add_space(6.0);
                    form::hint(
                        ui,
                        "A stance is a name shared by items and skeletons. This page is where the \
                         two meet.",
                    );
                });
        });
    editor.status.left.push(shown_path(key).to_owned());
}

fn paired(
    editor: &mut Editor,
    ui: &mut Ui,
    key: &str,
    content: &engine::core::content::Content,
    def: &StanceDef,
) {
    egui::Grid::new(("paired", key))
        .num_columns(2)
        .spacing(Vec2::new(16.0, 4.0))
        .show(ui, |ui| {
            for pairing in &def.paired {
                ui.label(format!("+ {}", pairing.with.get(content).shown_name()));
                ui.label(format!("→ {}", pairing.becomes.get(content).shown_name()));
                ui.end_row();
            }
        });
    let stances: Vec<(String, String)> = content
        .table::<StanceDef>()
        .iter()
        .map(|(id, stance)| (id.path(content).to_owned(), stance.shown_name().to_owned()))
        .collect();
    let id = ui.id().with(("new-pair", key));
    let (mut with, mut becomes): (Option<String>, Option<String>) =
        ui.data_mut(|data| data.get_temp(id).unwrap_or_default());
    let mut add = false;
    ui.horizontal(|ui| {
        ui.label(theme::muted("beside"));
        for (field, salt) in [(&mut with, "with"), (&mut becomes, "becomes")] {
            controls::combo((salt, key))
                .selected_text(field.as_deref().map_or("pick a stance".to_owned(), |path| {
                    shown_path(path).to_owned()
                }))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (path, name) in &stances {
                        ui.selectable_value(field, Some(path.clone()), name);
                    }
                });
            if salt == "with" {
                ui.label(theme::muted("it becomes"));
            }
        }
        add = ui
            .add_enabled(
                with.is_some() && becomes.is_some(),
                egui::Button::new("+ Add pair"),
            )
            .clicked();
    });
    ui.data_mut(|data| data.insert_temp(id, (with.clone(), becomes.clone())));
    if add && let (Some(with), Some(becomes)) = (with, becomes) {
        add_pair(editor, key, &with, &becomes);
        ui.data_mut(|data| data.remove::<(Option<String>, Option<String>)>(id));
    }
}
