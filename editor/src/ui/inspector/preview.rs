use egui::{Align, Color32, Layout, RichText, Sense, Ui, Vec2};
use engine::core::content::Content;
use engine::core::math::Direction;
use engine::core::time::{PlaybackRate, Seconds};
use engine::systems::GameSettings;
use engine::systems::actor::model::ModelDef;
use engine::systems::actor::{Action, ActorModel};
use engine::systems::dialogue::ChoiceReveal;
use engine::systems::quest::{ActiveQuest, Parties, QuestDef};
use engine::systems::script::{Reach, RiftScripts};
use serde_json::Value;
use strum::VariantArray;

use super::quests::quest_parties_sections;
use super::usages::show_usages_grouped;
use super::{theme, widgets};
use crate::app::Editor;
use crate::project::ProjectFiles;
use crate::views::character;
use crate::views::model::preview;
use crate::views::script::panel;
use crate::{forms, thumbs, value};

pub struct InspectorPreviewState {
    pub action: Action,
    pub direction: Direction,
    pub playing: bool,
    pub started: f64,
}

impl Default for InspectorPreviewState {
    fn default() -> InspectorPreviewState {
        InspectorPreviewState {
            action: Action::Idle,
            direction: Direction::S,
            playing: true,
            started: 0.0,
        }
    }
}

pub(super) struct Pose {
    action: Action,
    direction: Direction,
    elapsed: f32,
}

pub fn show_preview(editor: &mut Editor, ui: &mut Ui) {
    let selection = editor.selection.clone();
    let keys: Vec<String> = selection.keys.clone();
    if keys.is_empty() {
        return;
    }
    let several = keys.len() > 1;
    let columns = if several { 2 } else { 1 };
    let width = (ui.available_width() - if several { 8.0 } else { 0.0 }) / columns as f32;
    let height = if several { width * 0.7 } else { width * 0.72 };
    let now = ui.input(|i| i.time);
    let state = &editor.views.preview;
    let elapsed = if state.playing {
        now - state.started
    } else {
        0.0
    };
    let pose = Pose {
        action: state.action,
        direction: state.direction,
        elapsed: elapsed as f32,
    };
    egui::Grid::new("previews")
        .num_columns(columns)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (index, key) in keys.iter().enumerate() {
                let Some(value) = editor.row_value(&selection.table, key) else {
                    continue;
                };
                draw_preview(
                    editor,
                    ui,
                    &selection.table,
                    key,
                    &value,
                    Vec2::new(width, height),
                    &pose,
                );
                if (index + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
    if selection.table == "quest" && keys.len() == 1 {
        quest_parties_sections(editor, ui, &keys[0]);
    }
    if selection.table == "test_character" && keys.len() == 1 {
        character::preview_extras(editor, ui, &keys[0]);
    }
    if matches!(selection.table.as_str(), "npc" | "player" | "model") {
        ui.horizontal(|ui| {
            let state = &mut editor.views.preview;
            widgets::combo_box("preview-action")
                .selected_text(state.action.as_ref())
                .width(80.0)
                .show_ui(ui, |ui| {
                    for &action in Action::VARIANTS {
                        ui.selectable_value(&mut state.action, action, action.as_ref());
                    }
                });
            widgets::combo_box("preview-direction")
                .selected_text(state.direction.as_ref())
                .width(56.0)
                .show_ui(ui, |ui| {
                    for &direction in Direction::VARIANTS {
                        ui.selectable_value(&mut state.direction, direction, direction.as_ref());
                    }
                });
            if ui.button(if state.playing { "⏸" } else { "▸" }).clicked() {
                state.playing = !state.playing;
                state.started = now;
            }
        });
        if editor.views.preview.playing {
            ui.ctx().request_repaint();
        }
    }
    ui.add_space(6.0);
    for key in &keys {
        show_usages_grouped(editor, ui, &selection.table, key, several);
    }
}

pub(super) fn draw_preview(
    editor: &mut Editor,
    ui: &mut Ui,
    table: &str,
    key: &str,
    value: &Value,
    size: Vec2,
    pose: &Pose,
) {
    let ctx = ui.ctx().clone();
    let Some(content) = editor
        .project
        .as_ref()
        .and_then(|project| project.content.clone())
    else {
        return;
    };
    if table == "quest" {
        let Editor { project, form, .. } = editor;
        let project = project.as_ref().expect("a project");
        ui.vertical(|ui| {
            ui.set_width(size.x);
            quest_card(ui, &ctx, form, &project.files, &content, key, size.x);
        });
        return;
    }
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(78, 128, 52));
    let Editor { project, form, .. } = editor;
    let project = project.as_ref().expect("a project");
    painter.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Outside,
    );
    if table == "item" {
        if let Some(path) = value["icon"].as_str()
            && let Some(thumb) = form.thumbnails.image(&ctx, &project.files, path)
        {
            let scale = (size.y * 0.5 / thumb.size[1].max(1) as f32).clamp(1.0, 6.0);
            let shown = Vec2::new(thumb.size[0] as f32 * scale, thumb.size[1] as f32 * scale);
            painter.image(
                thumb.texture.id(),
                egui::Rect::from_center_size(rect.center(), shown),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        return;
    }
    let model_key = match table {
        "model" => Some(key.to_owned()),
        "test_character" => Some(
            content
                .built::<GameSettings>()
                .player
                .get(&content)
                .model
                .key(&content)
                .to_owned(),
        ),
        _ => value["model"].as_str().map(str::to_owned),
    };
    let Some(id) = model_key.and_then(|model_key| content.by_key::<ModelDef>(&model_key)) else {
        return;
    };
    let model = ActorModel::get(&content, id);
    let Some(thumb) = form
        .thumbnails
        .image(&ctx, &project.files, model.sheet().as_str())
    else {
        return;
    };
    let region = model.frame(
        pose.action,
        pose.direction,
        Seconds(pose.elapsed),
        PlaybackRate(1.0),
    );
    let drawn = model.drawn_size(region);
    let scale = (size.y * 0.6 / drawn.height.max(1.0))
        .clamp(1.0, 4.0)
        .floor();
    let shown = Vec2::new(drawn.width * scale, drawn.height * scale);
    let feet = preview::backdrop(
        editor,
        &painter,
        rect,
        format!("inspector-preview:{table}/{key}"),
        scale,
    );
    let sprite = preview::sprite_rect(feet, shown);
    let tint = value["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["type"] == "tint")
        .map(|m| {
            let [r, g, b, a] = value::rgba(&m["rgba"]);
            Color32::from_rgba_unmultiplied(r, g, b, a)
        })
        .unwrap_or(Color32::WHITE);
    let uv = thumbs::uv(
        thumb.size,
        region.min().x,
        region.min().y,
        region.size.width,
        region.size.height,
    );
    painter.image(thumb.texture.id(), sprite, uv, tint);
    if let Some(name) = value["name"].as_str() {
        let color = match attitude(&content, table, key, value) {
            Some(true) => design_tokens::ink::DANGER,
            _ => Color32::WHITE,
        };
        painter.text(
            sprite.center_top() - Vec2::new(0.0, 8.0),
            egui::Align2::CENTER_BOTTOM,
            name,
            egui::FontId::proportional(13.0),
            color,
        );
    }
}

pub(super) fn quest_card(
    ui: &mut Ui,
    ctx: &egui::Context,
    form: &mut forms::FormState,
    files: &ProjectFiles,
    content: &Content,
    key: &str,
    width: f32,
) {
    let Some(id) = content.by_key::<QuestDef>(key) else {
        return;
    };
    if !content.has_built::<RiftScripts>() {
        return;
    }
    let def = id.get(content);
    let parties = Parties::of(content, content.built::<RiftScripts>());
    let progress = def.progress(content, &parties, &ActiveQuest::fresh(content, id), None);
    let ink = Color32::from_rgb(236, 230, 214);
    let soft = Color32::from_rgba_unmultiplied(236, 230, 214, 150);
    let mut icon = |ui: &mut Ui, path: &str, size: f32| {
        if let Some(thumb) = form.thumbnails.image(ctx, files, path) {
            ui.add(egui::Image::from_texture((
                thumb.texture.id(),
                Vec2::splat(size),
            )));
        } else {
            ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        }
    };
    egui::Frame::new()
        .fill(Color32::from_rgb(38, 32, 28))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(120, 96, 60)))
        .corner_radius(8.0)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                icon(ui, def.icon(content).as_str(), 28.0);
                ui.label(RichText::new(&def.title).size(17.0).color(ink).strong());
            });
            ui.label(
                RichText::new(format!("{} · {}", def.kinds(), def.meta(&parties, id)))
                    .size(12.0)
                    .color(soft),
            );
            ui.label(RichText::new(&def.blurb).size(13.0).color(ink));
            ui.label(RichText::new("OBJECTIVES").size(11.0).color(soft));
            for (objective, progress) in def.objectives().zip(&progress) {
                ui.horizontal(|ui| {
                    icon(ui, objective.icon(content).as_str(), 20.0);
                    ui.label(RichText::new(&progress.label).size(13.0).color(ink));
                    if progress.counted {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{} / {}", progress.have, progress.need))
                                    .size(12.5)
                                    .color(ink),
                            );
                        });
                    }
                });
            }
            if def.reveals(ChoiceReveal::Gains) {
                ui.label(RichText::new("REWARDS").size(11.0).color(soft));
                ui.horizontal_wrapped(|ui| {
                    if def.xp > 0 {
                        widgets::chip_colored(
                            ui,
                            &format!("{} XP", def.xp),
                            Color32::TRANSPARENT,
                            ink,
                        );
                    }
                    for stack in def.rewards() {
                        let item = stack.item.get(content);
                        ui.horizontal(|ui| {
                            icon(ui, item.icon.as_str(), 16.0);
                            let label = if stack.count == 1 {
                                item.name.clone()
                            } else {
                                format!("{} ×{}", item.name, stack.count)
                            };
                            widgets::chip_colored(ui, &label, Color32::TRANSPARENT, ink);
                        });
                    }
                });
                if !def.pick_one().is_empty() {
                    ui.label(
                        RichText::new("PICK ONE WHEN YOU RETURN")
                            .size(11.0)
                            .color(soft),
                    );
                    ui.horizontal(|ui| {
                        for stack in def.pick_one() {
                            icon(ui, stack.item.get(content).icon.as_str(), 28.0);
                        }
                    });
                }
            }
            ui.label(
                RichText::new("Accept or decline in the conversation")
                    .size(12.0)
                    .color(soft),
            );
        });
}

fn attitude(content: &Content, table: &str, key: &str, value: &Value) -> Option<bool> {
    if !content.has_built::<RiftScripts>() {
        return None;
    }
    let scripts = content.built::<RiftScripts>();
    let mut friendly = false;
    for (index, module) in value["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if !module["type"]
            .as_str()
            .is_some_and(|tag| tag.ends_with("_script"))
        {
            continue;
        }
        let Some(bound) = panel::owner_row(content, table, key, &format!("/modules/{index}"))
        else {
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
