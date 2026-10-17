use egui::{RichText, Sense, Ui, Vec2};
use engine::core::audio::playback::SfxDef;
use engine::core::content::Content;
use engine::systems::area::tileset::SurfaceDef;

use super::{NewSurfaceDialog, TileSheetDialog};
use crate::app::Editor;
use crate::audio::Waveform;
use crate::project::{ContentEdit, row_file};
use crate::ui::dialogs::{self, DialogAnswer};
use crate::ui::{inspector, theme, widgets};
use crate::views::model::sounds;
use crate::{forms, picker, rows, value};

const TABLE: &str = "surface";
const CARD: Vec2 = Vec2::new(190.0, 120.0);

pub fn bottom(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let surfaces = sounds::surfaces(editor);
    let brush = editor.views.tileset.view(key).surface.clone();
    let mut pick = None;
    let mut play = None;
    let mut new = false;
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
        for surface in &surfaces {
            let on = brush.as_deref() == Some(surface.key.as_str());
            let wave = sounds::wave_of_sfx(editor, &surface.footstep);
            let level = footstep_level(editor, &surface.footstep, wave.as_ref());
            let used = editor.views.tileset.surface_use(&surface.key);
            let response = widgets::card_frame(on)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(CARD.x);
                        ui.set_min_height(CARD.y);
                        ui.horizontal(|ui| {
                            ui.label(theme::strong(surface.name.to_lowercase()));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::chip(ui, "▶").clicked() {
                                        play = Some(surface.footstep.clone());
                                    }
                                },
                            );
                        });
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(CARD.x - 8.0, 30.0), Sense::hover());
                        if let Some(wave) = &wave {
                            sounds::draw_wave(ui.painter(), rect, &wave.bins, theme::WARN);
                        }
                        ui.label(theme::muted(used.text()).size(12.0));
                        ui.label(
                            theme::faint(format!("{} · {level}", surface.footstep)).size(11.5),
                        );
                    });
                })
                .response
                .interact(Sense::click());
            if response.clicked() {
                pick = Some(surface.key.clone());
            }
        }
    });
    ui.add_space(6.0);
    if ui.button("+ New surface").clicked() {
        new = true;
    }
    if let Some(surface) = pick {
        editor.views.tileset.view_mut(key).surface = Some(surface);
    }
    if let Some(footstep) = play {
        editor.views.model.sfx_to_play.push(footstep);
    }
    if new {
        let dialog = NewSurfaceDialog {
            footstep: sounds::first_sfx(editor).unwrap_or_default(),
            ..NewSurfaceDialog::default()
        };
        editor.views.tileset.view_mut(key).dialog = Some(TileSheetDialog::NewSurface(dialog));
    }
}

pub fn card(editor: &mut Editor, ui: &mut Ui, surface: &str) {
    let Some(content) = editor.loaded() else {
        return;
    };
    let Some(id) = content.by_key::<SurfaceDef>(surface) else {
        return;
    };
    let def = id.get(&content);
    let footstep = def.footstep.key(&content).to_owned();
    widgets::section(ui, &format!("surface/{surface}"));
    let mut renamed = None;
    let mut new_footstep = None;
    let mut play = false;
    widgets::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_header(ui, &def.name, "", |_| {});
        ui.add_space(4.0);
        widgets::kv(ui, "name", |ui| {
            let id = ui.id().with(("surface-name", surface));
            let typed = forms::drafted(ui, id, &def.name, |ui, draft| {
                forms::boxed_input(ui, None, |ui| {
                    ui.add(egui::TextEdit::singleline(draft).desired_width(160.0))
                })
            });
            if let Some(typed) = typed
                && !typed.trim().is_empty()
                && typed.trim() != def.name
            {
                renamed = Some(typed.trim().to_owned());
            }
        });
        widgets::kv(ui, "footstep", |ui| {
            play = widgets::chip(ui, "▶").clicked();
            let mut picked = footstep.clone();
            let changed = inspector::with_row_form(editor, TABLE, surface, |cx| {
                picker::row(ui, cx, "sfx", &mut picked, "/footstep")
            });
            if changed && !picked.is_empty() && picked != footstep {
                new_footstep = Some(picked);
            }
        });
    });
    let wave = sounds::wave_of_sfx(editor, &footstep);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width() - 8.0, 36.0), Sense::hover());
    if let Some(wave) = &wave {
        sounds::draw_wave(ui.painter(), rect, &wave.bins, theme::WARN);
    }
    let (_, gain) = sounds::volume_of(editor, &footstep);
    let role = sounds::role_of_sfx(editor, &footstep);
    match (wave.as_ref().and_then(|wave| wave.loudness), role) {
        (Some(loudness), Some(role)) => {
            let verdict = sounds::verdict(loudness, gain, &role.def);
            widgets::kv(ui, "loudness", |ui| {
                widgets::verdict(ui, verdict.ok, &verdict.loudness);
            });
        }
        (None, _) => widgets::kv_text(ui, "loudness", "not measured"),
        (_, None) => widgets::kv_text(ui, "loudness", "no role to measure against"),
    }
    widgets::kv_text(
        ui,
        "used on",
        editor.views.tileset.surface_use(surface).text(),
    );
    if play {
        editor.views.model.sfx_to_play.push(footstep);
    }
    if let Some(name) = renamed {
        let label = format!("Name surface {surface} {name}");
        let def = SurfaceDef {
            name,
            ..def.clone()
        };
        write_surface(editor, &content, surface, &def, ContentEdit::new(label));
    }
    if let Some(footstep) = new_footstep
        && let Some(sfx) = content.by_key::<SfxDef>(&footstep)
    {
        let label = format!("Footstep {footstep} for surface {surface}");
        let def = SurfaceDef {
            footstep: sfx,
            ..def.clone()
        };
        write_surface(editor, &content, surface, &def, ContentEdit::new(label));
    }
}

pub fn new_surface_dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    tileset: &str,
    mut dialog: NewSurfaceDialog,
) -> Option<NewSurfaceDialog> {
    let footsteps = sounds::sfx_keys(editor);
    let mut answer = None;
    dialogs::card(ctx, "new-surface", 400.0, |ui| {
        ui.label(RichText::new("New surface").size(16.0).strong());
        ui.add_space(6.0);
        ui.label(theme::muted(
            "A name and a footstep: every tile painted with it sounds like this.",
        ));
        ui.add_space(8.0);
        widgets::kv(ui, "name", |ui| {
            let response = forms::boxed_input(ui, None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(200.0))
            });
            widgets::default_focus(ui, &response);
        });
        widgets::kv(ui, "footstep", |ui| {
            widgets::combo_box("new-surface-footstep")
                .selected_text(theme::mono(&dialog.footstep))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for sfx in &footsteps {
                        ui.selectable_value(&mut dialog.footstep, sfx.clone(), theme::mono(sfx));
                    }
                });
        });
        if let Some(error) = &dialog.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.add_space(10.0);
        answer = dialogs::answer_row(ui, "Add surface");
    });
    match answer {
        None => Some(dialog),
        Some(DialogAnswer::Cancel) => None,
        Some(DialogAnswer::Confirm) => match add_surface(editor, tileset, &dialog) {
            Ok(()) => None,
            Err(error) => Some(NewSurfaceDialog {
                error: Some(error),
                ..dialog
            }),
        },
    }
}

fn add_surface(
    editor: &mut Editor,
    tileset: &str,
    dialog: &NewSurfaceDialog,
) -> Result<(), String> {
    let key = rows::key_from_name(&dialog.name);
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    if let Some(problem) = rows::key_problem(&project.files, TABLE, &key) {
        return Err(problem);
    }
    let content = editor.loaded().ok_or("the folder does not load")?;
    let footstep = content
        .by_key::<SfxDef>(&dialog.footstep)
        .ok_or("pick a footstep")?;
    let def = SurfaceDef {
        name: dialog.name.trim().to_owned(),
        footstep,
    };
    let json = value::untyped(&content, &def).ok_or("the surface does not write")?;
    let mut edit = ContentEdit::new(format!("New surface {key}"));
    edit.write_json(row_file(TABLE, &key), &json);
    editor.try_apply(edit)?;
    editor.views.tileset.view_mut(tileset).surface = Some(key);
    Ok(())
}

fn write_surface(
    editor: &mut Editor,
    content: &Content,
    key: &str,
    def: &SurfaceDef,
    mut edit: ContentEdit,
) {
    if let Some(json) = value::untyped(content, def) {
        edit.write_json(row_file(TABLE, key), &json);
        editor.apply(edit);
    }
}

fn footstep_level(editor: &Editor, footstep: &str, wave: Option<&Waveform>) -> String {
    let (_, gain) = sounds::volume_of(editor, footstep);
    wave.and_then(|wave| wave.loudness).map_or_else(
        || "not measured".to_owned(),
        |loudness| format!("{:.1} LUFS", loudness.loudest_at(gain)),
    )
}
