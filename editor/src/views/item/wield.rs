//! The wield page of a held item: the stance it puts its wearer in (and the hand that claims), and
//! its grip, judged on a real model holding it in any stance and clip. Pairs come from the stance's
//! table; the inspector says which skeletons show the item, which wear it unseen, and which have a
//! set for its stance.

use engine::core::content::{ContentFile, NamedContent};
use engine::core::geometry::Degrees;
use engine::core::units::Meters;
use engine::systems::actor::model::{ActorModel, Skeletons};
use engine::systems::actor::render;
use engine::systems::actor::skeleton::SkeletonDef;
use engine::systems::actor::stance::{HandRole, PropHand, StanceDef};
use engine::systems::actor::{Action, ModelDef, Motion, Pose, held};
use engine::systems::item::{Grip, ItemDef, ItemKind, Wear};
use strum::VariantArray;
use ui::egui::{self, RichText, Ui, Vec2};
use ui::{controls, form, panel, theme};

use crate::app::Editor;
use crate::project::ContentEdit;
use crate::tabs::shown_path;
use crate::views::cues::{self, Scrub};
use crate::views::model::look::Shown;
use crate::views::stage::{self, StageState};

/// What the wield page's stage shows: a model, by its path, in a stance playing an action, pose or
/// move.
pub struct WieldView {
    pub model: Option<String>,
    pub stance: Option<String>,
    pub shown: Shown,
    pub scrub: Scrub,
    pub stage: StageState,
}

impl Default for WieldView {
    fn default() -> WieldView {
        WieldView {
            model: None,
            stance: None,
            shown: Shown {
                motion: Motion::Action(Action::Idle),
                pose: Pose::Idle,
            },
            scrub: Scrub::default(),
            stage: StageState::framing(1.1, false),
        }
    }
}

/// Puts the held item at `key` in the stance at `stance`, held by `grip`.
pub fn set_wield(editor: &mut Editor, key: &str, stance: &str, grip: Grip) -> bool {
    set_wield_as(
        editor,
        key,
        stance,
        grip,
        ContentEdit::new(format!("Wield {}", shown_path(key))),
    )
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ItemDef) {
    let Some(Wear::Held { stance, grip }) = row.wear().cloned() else {
        return;
    };
    let Some(reader) = editor.reader() else {
        return;
    };
    let stance_path = reader.path_of(stance).unwrap_or_default().to_owned();
    let width = ui.available_width();
    let fields_width = (width * 0.36).clamp(260.0, 400.0);
    let [left, right] = panel::split(ui, fields_width);
    panel::body(ui, left, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("wield", key))
            .auto_shrink([false, false])
            .show(ui, |ui| fields(editor, ui, key, &stance_path, grip));
    });
    panel::body(ui, right, |ui| {
        stage_side(editor, ui, key, &stance_path, grip)
    });
}

fn fields(editor: &mut Editor, ui: &mut Ui, key: &str, stance_path: &str, grip: Grip) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let stances: Vec<(String, String)> = content
        .table::<StanceDef>()
        .iter()
        .map(|(id, stance)| (id.path(&content).to_owned(), stance.shown_name().to_owned()))
        .collect();
    let mut picked = stance_path.to_owned();
    let mut edited = grip;
    egui::Grid::new(("wield-fields", key))
        .num_columns(2)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(theme::muted("slot"));
            ui.label("hand");
            ui.end_row();
            ui.label(theme::muted("stance"));
            controls::combo(("wield-stance", key))
                .selected_text(shown_path(&picked).to_owned())
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (path, name) in &stances {
                        ui.selectable_value(&mut picked, path.clone(), name)
                            .on_hover_text(path);
                    }
                });
            ui.end_row();
            if let Some(def) = content
                .by_path::<StanceDef>(stance_path)
                .map(|id| id.get(&content))
            {
                ui.label(theme::muted("hand"));
                ui.label(format!("{} (from the stance)", claims(def)));
                ui.end_row();
            }
            ui.label(theme::muted("grip · offset · m"));
            ui.horizontal(|ui| {
                for axis in &mut edited.offset.0 {
                    let mut value = axis.0;
                    ui.add(
                        egui::DragValue::new(&mut value)
                            .speed(0.002)
                            .fixed_decimals(3),
                    );
                    *axis = Meters(value);
                }
            });
            ui.end_row();
            ui.label(theme::muted("grip · turn · °"));
            ui.horizontal(|ui| {
                for axis in &mut edited.turn.0 {
                    let mut value = axis.0;
                    ui.add(
                        egui::DragValue::new(&mut value)
                            .speed(0.5)
                            .range(-180.0..=180.0)
                            .fixed_decimals(1),
                    );
                    *axis = Degrees(value);
                }
            });
            ui.end_row();
        });
    form::hint(
        ui,
        "The grip moves and turns the prop in the hand's frame, where +Y runs along the palm, until \
         its handle sits in it.",
    );
    if picked != stance_path || edited != grip {
        let label = if picked != stance_path {
            format!("Put {} in {}", shown_path(key), shown_path(&picked))
        } else {
            format!("Grip {}", shown_path(key))
        };
        set_wield_as(
            editor,
            key,
            &picked,
            edited,
            ContentEdit::new(label).merging(),
        );
    }
    ui.add_space(10.0);
    let Some(stance) = content.by_path::<StanceDef>(stance_path) else {
        return;
    };
    let def = stance.get(&content);
    form::section_count(ui, "Pairs", def.paired.len());
    for pairing in &def.paired {
        ui.label(format!(
            "+ {} → {}",
            pairing.with.get(&content).shown_name(),
            pairing.becomes.get(&content).shown_name()
        ));
    }
    ui.horizontal(|ui| {
        ui.label(theme::faint(format!(
            "From {}'s paired table.",
            shown_path(stance_path)
        )));
        if ui.small_button("Edit it there").clicked() {
            editor.open_row(StanceDef::TABLE, stance_path);
        }
    });
}

/// The model holding the item on the stage, in the chosen stance and clip.
fn stage_side(editor: &mut Editor, ui: &mut Ui, key: &str, stance_path: &str, grip: Grip) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let models: Vec<(String, &ActorModel)> = content
        .table::<ModelDef>()
        .iter()
        .filter_map(|(id, _)| {
            let model = ActorModel::loaded(&content, id)?;
            (!model.hands().is_empty()).then(|| (id.path(&content).to_owned(), model))
        })
        .collect();
    let view = &editor.views.item.view_mut(key).wield;
    let mut model_path = view
        .model
        .clone()
        .filter(|path| models.iter().any(|(known, _)| known == path))
        .or_else(|| models.first().map(|(path, _)| path.clone()));
    let mut stance = view.stance.clone().or_else(|| Some(stance_path.to_owned()));
    let mut shown = view.shown.clone();
    let Some((_, model)) = model_path
        .as_ref()
        .and_then(|path| models.iter().find(|(known, _)| known == path))
    else {
        form::hint(ui, "No model with hands loaded, so nothing holds the item.");
        return;
    };
    let mut choices: Vec<Shown> = Action::VARIANTS
        .iter()
        .map(|action| Shown {
            motion: Motion::Action(*action),
            pose: Pose::Idle,
        })
        .collect();
    let mut moves: Vec<_> = model.skeleton().def.moves().cloned().collect();
    moves.sort();
    moves.dedup();
    choices.extend(moves.into_iter().map(|name| Shown {
        motion: Motion::Move(name),
        pose: Pose::Idle,
    }));
    ui.horizontal(|ui| {
        ui.label(theme::muted("on"));
        controls::combo(("wield-model", key))
            .selected_text(model_path.as_deref().map(shown_path).unwrap_or_default())
            .width(170.0)
            .show_ui(ui, |ui| {
                for (path, _) in &models {
                    ui.selectable_value(&mut model_path, Some(path.clone()), shown_path(path));
                }
            });
        ui.label(theme::muted("stance"));
        controls::combo(("wield-stage-stance", key))
            .selected_text(stance.as_deref().map_or("base", shown_path))
            .width(150.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut stance, None, "base");
                for (id, def) in content.table::<StanceDef>().iter() {
                    let path = id.path(&content).to_owned();
                    ui.selectable_value(&mut stance, Some(path), def.shown_name());
                }
            });
        ui.label(theme::muted("plays"));
        controls::combo(("wield-shown", key))
            .selected_text(shown.label())
            .width(140.0)
            .show_ui(ui, |ui| {
                for choice in &choices {
                    ui.selectable_value(&mut shown, choice.clone(), choice.label());
                }
            });
    });
    let view = &mut editor.views.item.view_mut(key).wield;
    if model_path != view.model || stance != view.stance || shown != view.shown {
        view.model = model_path.clone();
        view.stance = stance.clone();
        view.shown = shown.clone();
        view.scrub.restart();
    }
    let stance_id = stance
        .as_deref()
        .and_then(|path| content.by_path::<StanceDef>(path));
    let resolved = model.resolve(stance_id, &shown.motion, shown.pose);
    let clip = resolved
        .source
        .map(|source| (model.rig_clip(source), resolved.length.millis()));
    cues::advance(
        ui,
        &mut editor.views.item.view_mut(key).wield.scrub,
        clip.as_ref().map(|(_, length)| *length),
    );
    let scrub = &editor.views.item.view_mut(key).wield.scrub;
    let playhead = scrub.playhead;
    let crossed = scrub.crossed(resolved.cues, resolved.length.millis());
    let props = content
        .by_path::<ItemDef>(key)
        .map(|item| {
            let shown = held::listed(&content, model.hands().len(), &[item]);
            render::hung(&content, model, &shown)
                .into_keys()
                .filter_map(|hand| stage::prop_in_hand(&content, model, hand, item, &grip))
                .collect()
        })
        .unwrap_or_default();
    let size = Vec2::new(ui.available_width(), ui.available_height() - 24.0);
    let mut state = std::mem::take(&mut editor.views.item.view_mut(key).wield.stage);
    let turntable = stage::turntable(editor, ui, &format!("wield:{key}"), size, &mut state);
    editor.views.item.view_mut(key).wield.stage = state;
    if let Some(turntable) = turntable {
        let held = clip.as_ref().map(|_| playhead.seconds());
        let standing = stage::standing(
            model.body(),
            clip.map(|(clip, _)| clip),
            held,
            crossed,
            props,
        );
        turntable.stand(editor, ui, vec![standing]);
    }
    form::hint(ui, cues::KEYS);
}

/// Which skeletons show the item, which wear it unseen, and which have a set for its stance.
pub fn inspect(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let Some(item) = content.by_path::<ItemDef>(key) else {
        return;
    };
    let Some(Wear::Held { stance, .. }) = item.get(&content).wear() else {
        return;
    };
    let def = stance.get(&content);
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Wield", "", |_| {});
        ui.add_space(4.0);
        form::kv_text(
            ui,
            "hands",
            format!("{} · {} (from the stance)", def.hands.0, claims(def)),
        );
        for (id, skeleton) in content.table::<SkeletonDef>().iter() {
            let Some(built) = Skeletons::loaded(&content, id) else {
                continue;
            };
            let hands = built.def.hands.len();
            let shows = if hands >= def.hands.0 as usize {
                "shows it".to_owned()
            } else if hands == 0 {
                "wears it unseen (no hands)".to_owned()
            } else {
                format!("wears it unseen ({hands} hands)")
            };
            let set = if built.def.set(*stance).is_some() {
                format!("a set for {}", def.shown_name())
            } else {
                "no set · base".to_owned()
            };
            ui.label(
                RichText::new(format!("{} · {shows} · {set}", skeleton.shown_name())).size(12.5),
            );
        }
    });
}

fn claims(stance: &StanceDef) -> String {
    let hand = match stance.hand {
        HandRole::Main => "main",
        HandRole::Off => "off",
        HandRole::Either => "either",
    };
    let prop = match (stance.hands.0, stance.prop) {
        (2.., PropHand::Last) => ", hung from the last",
        _ => "",
    };
    match stance.hands.0 {
        1 => format!("1 hand · {hand}{prop}"),
        count => format!("{count} hands · from {hand}{prop}"),
    }
}

fn set_wield_as(
    editor: &mut Editor,
    key: &str,
    stance: &str,
    grip: Grip,
    edit: ContentEdit,
) -> bool {
    let Some(Ok(row)) = editor.read::<ItemDef>(key) else {
        return false;
    };
    let Some(stance) = editor
        .reader()
        .and_then(|reader| reader.id_of::<StanceDef>(stance))
    else {
        editor.toast(format!("{stance} is not a stance"));
        return false;
    };
    let mut item = (*row).clone();
    let ItemKind::Equipment { wear, .. } = &mut item.kind else {
        return false;
    };
    let held = Wear::Held { stance, grip };
    if *wear == held {
        return false;
    }
    *wear = held;
    editor.write_content_as(edit, key, &item)
}
