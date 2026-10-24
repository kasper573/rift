use engine::core::audio::playback::{SFX_CLIP, SfxDef};
use engine::core::babble::{BABBLE_LETTERS, BabbleBankDef, BabbleDef, LETTERS};
use engine::core::content::{Content, ContentFile, MODULES};
use engine::core::gui::ImageSource;
use engine::core::math::Rgba;
use engine::core::time::Seconds;
use engine::systems::GameSettings;
use engine::systems::actor::Action;
use engine::systems::actor::model::ModelDef;
use engine::systems::actor::plate::PlateLabel;
use engine::systems::item::card::CardLook;
use engine::systems::item::{self, ItemDef};
use engine::systems::npc::{NpcDef, NpcModule};
use engine::systems::player::PlayerDef;
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::quest::card::{Moment, QuestCardLook};
use engine::systems::quest::{self, Parties, QuestDef};
use engine::systems::script::{Reach, RiftScripts};
use engine::systems::skill::SkillDef;
use engine::systems::status::StatusEffectDef;
use strum::VariantArray;
use ui::card::{self};
use ui::controls;
use ui::egui::Ui;
use ui::glyph::{self, Mark};
use ui::theme;
use ui::words;

use super::quests::quest_parties_sections;
use crate::app::Editor;
use crate::rows;
use crate::stage::{self, Posed, PosedActor, Scenario, Scene};
use crate::tabs::shown_path;
use crate::thumbs::ProjectImages;
use crate::views::character;
use crate::views::script::panel;
use crate::views::sound::BabbleRequest;
use ui::input::InputField;

pub struct RowPreviewState {
    pub action: Action,
    pub playing: bool,
    pub started: f64,
    /// The model a status effect is shown on; the player's until one is picked.
    pub status_model: Option<String>,
    pub status_applied: bool,
}

impl Default for RowPreviewState {
    fn default() -> RowPreviewState {
        RowPreviewState {
            action: Action::Idle,
            playing: true,
            started: 0.0,
            status_model: None,
            status_applied: false,
        }
    }
}

pub(super) struct Pose {
    action: Action,
    playing: bool,
}

/// Where a table's preview goes: a column beside the form for one that shows a picture, under the
/// form for one that is a control or two.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PreviewPlace {
    Beside,
    Below,
}

pub fn preview_place(table: &str) -> Option<PreviewPlace> {
    match table {
        NpcDef::TABLE
        | PlayerDef::TABLE
        | ModelDef::TABLE
        | ItemDef::TABLE
        | QuestDef::TABLE
        | CharacterSnapshot::TABLE
        | StatusEffectDef::TABLE
        | SkillDef::TABLE => Some(PreviewPlace::Beside),
        SfxDef::TABLE | BabbleDef::TABLE | BabbleBankDef::TABLE => Some(PreviewPlace::Below),
        _ => None,
    }
}

/// The row as the game shows it, beside its form: the actor, item or quest card it stands for, a
/// status effect on an actor, a skill cast over and over.
pub fn show(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str) {
    match table {
        StatusEffectDef::TABLE => return show_status(editor, ui, key),
        SkillDef::TABLE => return show_skill(editor, ui, key),
        SfxDef::TABLE => return show_sfx(editor, ui, key),
        BabbleBankDef::TABLE => return show_babble_bank(editor, ui, key),
        BabbleDef::TABLE => return show_babble(editor, ui, key),
        _ => {}
    }
    let now = ui.input(|i| i.time);
    let state = &editor.views.preview;
    let pose = Pose {
        action: state.action,
        playing: state.playing,
    };
    draw_preview(editor, ui, table, key, &pose);
    if table == QuestDef::TABLE {
        quest_parties_sections(editor, ui, key);
    }
    if table == CharacterSnapshot::TABLE {
        character::preview_extras(editor, ui, key);
    }
    if matches!(table, NpcDef::TABLE | PlayerDef::TABLE | ModelDef::TABLE) {
        ui.horizontal(|ui| {
            let state = &mut editor.views.preview;
            controls::combo("preview-action")
                .selected_text(state.action.as_ref())
                .width(80.0)
                .show_ui(ui, |ui| {
                    for &action in Action::VARIANTS {
                        ui.selectable_value(&mut state.action, action, action.as_ref());
                    }
                });
            let (mark, word) = if state.playing {
                (Mark::Pause, "Pause")
            } else {
                (Mark::Play, "Play")
            };
            if glyph::button(ui, mark, 14.0, theme::TEXT, word).clicked() {
                state.playing = !state.playing;
                state.started = now;
            }
        });
        if editor.views.preview.playing {
            ui.ctx().request_repaint();
        }
    }
}

/// A status effect on one actor of the designer's choosing, applied and removed at a press, in a
/// game world that plays the effect out for real.
fn show_status(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return;
    };
    let Some(effect) = content.by_path::<StatusEffectDef>(key) else {
        ui.label(theme::muted("the preview plays once the row loads"));
        return;
    };
    let models: Vec<String> = content.table::<ModelDef>().paths().to_vec();
    let state = &mut editor.views.preview;
    let chosen = state.status_model.get_or_insert_with(|| {
        content
            .built::<GameSettings>()
            .player
            .get(&content)
            .model
            .path(&content)
            .to_owned()
    });
    ui.horizontal(|ui| {
        controls::combo("preview-status-model")
            .selected_text(theme::mono(chosen.as_str()).size(13.0))
            .width(160.0)
            .show_ui(ui, |ui| {
                for candidate in &models {
                    ui.selectable_value(chosen, candidate.clone(), candidate);
                }
            });
        let label = if state.status_applied {
            "remove status effect"
        } else {
            "apply status effect"
        };
        if ui.button(label).clicked() {
            state.status_applied = !state.status_applied;
        }
    });
    let Some(model) = content.by_path::<ModelDef>(chosen) else {
        ui.label(theme::warn("pick a model to show the effect on"));
        return;
    };
    let scene = Scene::Live(Scenario::Status {
        model,
        effect,
        applied: state.status_applied,
    });
    stage::show(editor, ui, &format!("status_effect/{key}"), scene, true);
}

/// A skill cast on a loop by a character set up to cast it, at whatever it targets, in a game
/// world that plays each cast out for real.
fn show_skill(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(content) = editor.loaded() else {
        ui.label(theme::muted("the preview plays once the content loads"));
        return;
    };
    let Some(skill) = content.by_path::<SkillDef>(key) else {
        ui.label(theme::muted("the preview plays once the row loads"));
        return;
    };
    let scene = Scene::Live(Scenario::Skill { skill });
    if let Some(shown) = stage::show(editor, ui, &format!("skill/{key}"), scene, true)
        && let Some(report) = shown.report
    {
        ui.label(theme::muted(report).size(12.0));
    }
}

/// An sfx played as the game plays it, and the clip it plays put in from a file.
fn show_sfx(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let mut replace = false;
    ui.horizontal(|ui| {
        let sound = &mut editor.views.sound;
        if sound.sounding.contains(key) {
            if controls::marked_primary(ui, Mark::Stop, "stop").clicked() {
                sound.stop.push(key.to_owned());
            }
        } else if controls::marked_primary(ui, Mark::Play, "play").clicked() {
            sound.play.push(key.to_owned());
        }
        replace = ui.button("Replace the clip…").clicked();
    });
    if replace
        && let Some(source) = rfd::FileDialog::new()
            .set_title(format!("The clip of {}", shown_path(key)))
            .add_filter("sound", SFX_CLIP.exts())
            .pick_file()
        && let Err(error) = rows::put_parts(editor, key, SFX_CLIP, &[source])
    {
        editor.toast(error);
    }
}

/// The letters a babble bank speaks with, the ones it lacks named, and more put in from files
/// named by their letters.
fn show_babble_bank(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(files) = editor.project.as_ref().map(|project| &project.files) else {
        return;
    };
    let lacking: Vec<String> = LETTERS
        .chars()
        .map(String::from)
        .filter(|letter| {
            !BABBLE_LETTERS
                .exts()
                .iter()
                .any(|ext| files.contains(&format!("{key}/{}", BABBLE_LETTERS.named(letter, ext))))
        })
        .collect();
    let mut import = false;
    ui.horizontal(|ui| {
        ui.label(theme::muted(if lacking.is_empty() {
            "every letter, a to z".to_owned()
        } else {
            format!("lacks {}", words::join_and(&lacking))
        }));
        import = ui.button("Import letters…").clicked();
    });
    if import
        && let Some(sources) = rows::picked_files(
            rfd::FileDialog::new()
                .set_title(format!(
                    "Letters for {}, each named by its letter",
                    shown_path(key)
                ))
                .add_filter("sound", BABBLE_LETTERS.exts()),
        )
        && let Err(error) = rows::put_parts(editor, key, BABBLE_LETTERS, &sources)
    {
        editor.toast(error);
    }
}

/// A babble speaking a line of the designer's choosing, through the game's own typewriter.
fn show_babble(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let sound = &mut editor.views.sound;
    InputField::new(&mut sound.babble_text).rows(2).show(ui);
    if sound.babbling() {
        if controls::marked_primary(ui, Mark::Stop, "stop").clicked() {
            sound.stop_babble = true;
        }
    } else if controls::marked_primary(ui, Mark::Play, "play").clicked() {
        sound.babble = Some(BabbleRequest {
            babble: key.to_owned(),
            text: sound.babble_text.clone(),
        });
    }
}

fn draw_preview(editor: &mut Editor, ui: &mut Ui, table: &str, key: &str, pose: &Pose) {
    let ctx = ui.ctx().clone();
    let Some(content) = editor.loaded() else {
        return;
    };
    if table == QuestDef::TABLE || table == ItemDef::TABLE {
        let Editor { project, form, .. } = &mut *editor;
        let mut images = ProjectImages {
            ctx: &ctx,
            files: &project.as_ref().expect("a project").files,
            thumbnails: &mut form.thumbnails,
        };
        if table == QuestDef::TABLE {
            quest_card(ui, &mut images, &content, key);
            return;
        }
        item_card(ui, &mut images, &content, key);
        ui.add_space(12.0);
    }
    let center = stage::center(&content);
    let mut posed = Posed::default();
    if table == ItemDef::TABLE {
        posed.items.extend(content.by_path::<ItemDef>(key));
    } else {
        let Some(look) = actor_look(editor, &content, table, key) else {
            return;
        };
        let Some(model) = content.by_path::<ModelDef>(&look.model) else {
            ui.label(theme::muted("pick a model to see it"));
            return;
        };
        posed.actors.push(PosedActor {
            action: pose.action,
            tint: look.tint,
            plate: look.plate,
            held: (!pose.playing).then_some(Seconds(0.0)),
            ..PosedActor::standing(model, center)
        });
    }
    stage::show(
        editor,
        ui,
        &format!("{table}/{key}"),
        Scene::Posed(posed),
        true,
    );
}

/// How the row's actor stands on the stage: the model it wears, its tint, and the plate it wears
/// in game, an npc's name and role once it has a nameplate, a test character's name as a player.
struct ActorLook {
    model: String,
    tint: Rgba,
    plate: Option<PlateLabel>,
}

fn actor_look(editor: &Editor, content: &Content, table: &str, key: &str) -> Option<ActorLook> {
    let project = editor.project.as_ref()?;
    let reader = project.reader();
    match table {
        ModelDef::TABLE => Some(ActorLook {
            model: key.to_owned(),
            tint: Rgba::WHITE,
            plate: None,
        }),
        NpcDef::TABLE => {
            let npc = project.read::<NpcDef>(key).ok()?;
            let hostile = attitude(content, key, &npc) == Some(true);
            Some(ActorLook {
                model: reader.path_of(npc.model)?.to_owned(),
                tint: npc.tint(),
                plate: npc
                    .nameplate()
                    .map(|role| PlateLabel::npc(&npc.name, role, hostile)),
            })
        }
        PlayerDef::TABLE => {
            let player = project.read::<PlayerDef>(key).ok()?;
            Some(ActorLook {
                model: reader.path_of(player.model)?.to_owned(),
                tint: player.tint(),
                plate: None,
            })
        }
        CharacterSnapshot::TABLE => {
            let snapshot = project.read::<CharacterSnapshot>(key).ok()?;
            let player = project.read_id(project.settings().ok()?.player)?;
            Some(ActorLook {
                model: reader.path_of(player.model)?.to_owned(),
                tint: player.tint(),
                plate: Some(PlateLabel::player(&snapshot.name)),
            })
        }
        _ => None,
    }
}

fn quest_card(ui: &mut Ui, images: &mut ProjectImages, content: &Content, key: &str) {
    let Some(quest) = content.by_path::<QuestDef>(key) else {
        return;
    };
    if !content.has_built::<RiftScripts>() {
        return;
    }
    let parties = Parties::of(content, content.built::<RiftScripts>());
    let width = ui.available_width();
    let look = QuestCardLook {
        width: width.min(card::DOCKED_WIDTH),
        max_height: card::MAX_HEIGHT,
        moment: Some(Moment::Offer),
    };
    ui.vertical(|ui| {
        ui.set_width(width);
        quest::card::show(ui, images, content, quest, look, |ui, images| {
            quest::card::offer(ui, images, content, &parties, quest, None);
        });
    });
}

fn item_card(ui: &mut Ui, images: &mut ProjectImages, content: &Content, key: &str) {
    let Some(item) = content.by_path::<ItemDef>(key) else {
        return;
    };
    let def = item.get(content);
    let art = images.image(&def.icon).map(|image| image.texture);
    let look = CardLook {
        width: ui.available_width().min(card::WIDTH),
        closable: false,
        ..CardLook::PINNED
    };
    item::card::face_of(def, art, look).show(ui, |ui, plate| {
        item::card::summary(ui, content, item, plate);
        item::card::details(ui, content, item, |_| None);
        item::card::grant_lines(ui, images, content, item);
    });
}

fn attitude(content: &Content, key: &str, npc: &NpcDef) -> Option<bool> {
    if !content.has_built::<RiftScripts>() {
        return None;
    }
    let scripts = content.built::<RiftScripts>();
    let mut friendly = false;
    for (index, module) in npc.modules.iter().enumerate() {
        if !matches!(module, NpcModule::Script(_)) {
            continue;
        }
        let pointer = format!("/{MODULES}/{index}");
        let Some(bound) = panel::owner_row(content, NpcDef::TABLE, key, &pointer) else {
            continue;
        };
        for reach in scripts.may_do(bound.row).reaches {
            match reach {
                Reach::Hostile => return Some(true),
                Reach::Friendly => friendly = true,
                _ => {}
            }
        }
    }
    friendly.then_some(false)
}
