use engine::core::audio::playback::SfxDef;
use engine::core::content::{ContentFile, parent_of};
use engine::systems::area::tileset::SurfaceDef;
use ui::chip;
use ui::controls;
use ui::dialog;
use ui::egui;
use ui::egui::{RichText, Sense, Ui, Vec2};
use ui::form;
use ui::glyph::Mark;
use ui::input;
use ui::panel;
use ui::sheet;
use ui::theme;

use super::sounds;
use super::{NewSurfaceDialog, TileSheetDialog};
use crate::app::Editor;
use crate::project::ContentEdit;
use crate::shell::row_form;
use crate::tabs::shown_path;
use crate::{picker, rows};
use ui::input::InputField;

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
            let wave = sounds::wave_of_sfx(editor, ui.ctx(), &surface.footstep);
            let used = editor.views.tileset.surface_use(&surface.key);
            let response = panel::card(on)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(CARD.x);
                        ui.set_min_height(CARD.y);
                        ui.horizontal(|ui| {
                            ui.label(theme::strong(surface.name.to_lowercase()));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if chip::plain(ui, Mark::Play.text(12.0)).clicked() {
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
                        ui.label(theme::faint(&surface.footstep).size(11.5));
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
        editor.views.sound.play.push(footstep);
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
    let (Some(Ok(def)), Some(reader)) = (editor.read::<SurfaceDef>(surface), editor.reader())
    else {
        return;
    };
    let footstep = reader.path_of(def.footstep).unwrap_or_default().to_owned();
    sheet::section(ui, shown_path(surface));
    let mut renamed = None;
    let mut new_footstep = None;
    let mut play = false;
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, &def.name, "", |_| {});
        ui.add_space(4.0);
        form::kv(ui, "name", |ui| {
            let id = ui.id().with(("surface-name", surface));
            let typed = input::drafted(ui, id, &def.name, |ui, draft| {
                InputField::new(draft).show(ui)
            });
            if let Some(typed) = typed
                && !typed.trim().is_empty()
                && typed.trim() != def.name
            {
                renamed = Some(typed.trim().to_owned());
            }
        });
        form::kv(ui, "footstep", |ui| {
            play = chip::plain(ui, Mark::Play.text(12.0)).clicked();
            let mut picked = footstep.clone();
            let changed = row_form::with_row_form(editor, SurfaceDef::TABLE, surface, |cx| {
                picker::row(ui, cx, SfxDef::TABLE, &mut picked, "/footstep")
            });
            if changed && !picked.is_empty() && picked != footstep {
                new_footstep = Some(picked);
            }
        });
    });
    let wave = sounds::wave_of_sfx(editor, ui.ctx(), &footstep);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width() - 8.0, 36.0), Sense::hover());
    if let Some(wave) = &wave {
        sounds::draw_wave(ui.painter(), rect, &wave.bins, theme::WARN);
    }
    form::kv_text(
        ui,
        "used on",
        editor.views.tileset.surface_use(surface).text(),
    );
    if play {
        editor.views.sound.play.push(footstep);
    }
    if let Some(name) = renamed {
        let label = format!("Name surface {surface} {name}");
        let def = SurfaceDef {
            name,
            ..SurfaceDef::clone(&def)
        };
        editor.write_content(&label, surface, &def);
    }
    if let Some(footstep) = new_footstep
        && let Some(sfx) = reader.id_of::<SfxDef>(&footstep)
    {
        let label = format!("Footstep {footstep} for surface {surface}");
        let def = SurfaceDef {
            footstep: sfx,
            ..SurfaceDef::clone(&def)
        };
        editor.write_content(&label, surface, &def);
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
    dialog::card(ctx, "new-surface", 400.0, |ui| {
        ui.label(RichText::new("New surface").size(16.0).strong());
        ui.add_space(6.0);
        ui.label(theme::muted(
            "A name and a footstep: every tile painted with it sounds like this.",
        ));
        ui.add_space(8.0);
        form::kv(ui, "name", |ui| {
            let response = InputField::new(&mut dialog.name).width(200.0).show(ui);
            input::default_focus(ui, &response);
        });
        form::kv(ui, "footstep", |ui| {
            controls::combo("new-surface-footstep")
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
        answer = dialog::answer_row(ui, "Add surface");
    });
    match answer {
        None => Some(dialog),
        Some(dialog::Answer::Cancel) => None,
        Some(dialog::Answer::Confirm) => match add_surface(editor, tileset, &dialog) {
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
    let name = rows::name_from(&dialog.name);
    let project = editor.project.as_ref().ok_or("no folder is open")?;
    let folder = parent_of(tileset);
    let key = rows::new_path(&project.files, folder, SurfaceDef::TABLE, &name);
    if let Some(problem) =
        rows::name_problem(&project.files, folder, &name, rows::name_kind(&key), None)
    {
        return Err(problem);
    }
    let reader = project.reader();
    let footstep = reader
        .id_of::<SfxDef>(&dialog.footstep)
        .ok_or("pick a footstep")?;
    let def = SurfaceDef {
        name: dialog.name.trim().to_owned(),
        footstep,
    };
    let mut edit = ContentEdit::new(format!("New surface {name}"));
    edit.write_content_in(&reader, &key, &def)?;
    editor.try_apply(edit)?;
    editor.views.tileset.view_mut(tileset).surface = Some(key);
    Ok(())
}
