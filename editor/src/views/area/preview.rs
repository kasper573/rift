use engine::core::math::Pos;
use engine::systems::area;
use engine::systems::script::Place;
use ui::controls;
use ui::egui;

use crate::views::navigation::PX_PER_METER;
use ui::egui::{Sense, Ui, Vec2};
use ui::form;
use ui::panel;
use ui::sheet;
use ui::theme;
use ui::words;

use super::{pins, select};
use crate::app::{Clipboard, Editor};
use crate::forms;
use crate::keymap::EditorKey;
use crate::views::navigation;

pub(super) fn clipboard_panel(editor: &Editor, ui: &mut Ui) {
    let Some(clip) = editor.clipboard.as_ref().and_then(Clipboard::scene) else {
        return;
    };
    ui.label(
        theme::muted(format!(
            "clipboard · {} row{}",
            clip.count,
            words::plural(clip.count)
        ))
        .monospace()
        .size(13.0),
    );
    ui.add_space(4.0);
    let text = clip.nodes.text();
    let shown: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
    panel::code_block(ui, &shown);
    if text.lines().count() > 40 {
        ui.label(theme::faint("…").size(12.5));
    }
    ui.add_space(6.0);
}

pub fn pick_preview(editor: &mut Editor, ui: &mut Ui) {
    let Some(pick) = editor.pick.clone() else {
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let Some(content) = project.content.clone() else {
        return;
    };
    let mut area_key = pick.area.clone().or_else(|| {
        let place = project.value_at::<Place>(&pick.file, &pick.pointer)?;
        project.reader().path_of(place.area).map(str::to_owned)
    });
    let mut areas: Vec<String> = content
        .ids::<area::AreaDef>()
        .map(|id| id.path(&content).to_owned())
        .collect();
    areas.sort();
    sheet::section(ui, "Pick a place");
    controls::combo("pick-area")
        .width(ui.available_width())
        .selected_text(theme::mono(
            area_key
                .clone()
                .unwrap_or_else(|| "choose an area".to_owned()),
        ))
        .show_ui(ui, |ui| {
            for key in &areas {
                if ui
                    .selectable_label(area_key.as_deref() == Some(key.as_str()), key)
                    .clicked()
                {
                    area_key = Some(key.clone());
                }
            }
        });
    if area_key != pick.area
        && let Some(pick) = editor.pick.as_mut()
    {
        pick.area = area_key.clone();
    }
    let Some(area_key) = area_key else {
        return;
    };
    let Some(area_id) = content.by_path::<area::AreaDef>(&area_key) else {
        return;
    };
    let area = area::load(&content, area_id);
    let width = ui.available_width();
    let height = (width * area.size.height / area.size.width.max(1.0)).clamp(80.0, 360.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let zoom = navigation::fit_zoom(
        rect.size(),
        Vec2::new(area.size.width, area.size.height),
        PX_PER_METER,
    );
    let center = Pos::new(area.size.width / 2.0 - 0.5, area.size.height / 2.0 - 0.5);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::INPUT);
    crate::thumbs::areas::draw_area(
        editor,
        &painter,
        &area_key,
        rect,
        center,
        zoom * PX_PER_METER,
    );
    let canvas = navigation::Viewport {
        screen: rect,
        center,
        zoom,
    };
    for arrival in pins::arrivals(editor, &area_key) {
        let at = canvas.to_screen(arrival.at);
        painter.circle_filled(at, 3.0, egui::Color32::from_rgb(255, 190, 90));
        painter.circle_stroke(at, 3.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
    }
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let alt = ui.input(|i| EditorKey::Unsnapped.held(i));
    if let Some(pointer) = response.hover_pos() {
        let ground = canvas.to_ground(pointer);
        let at = select::snap_to(ground, alt);
        let place = Place { area: area_id, at };
        let verdict = area::check_place(&content, place);
        let (ok, why) = match &verdict {
            Ok(()) => (true, "outside every warp object".to_owned()),
            Err(problem) => (false, problem.expected.clone()),
        };
        painter.circle_stroke(
            canvas.to_screen(at),
            5.0,
            egui::Stroke::new(
                2.0,
                if ok {
                    egui::Color32::from_rgb(255, 230, 120)
                } else {
                    theme::DANGER
                },
            ),
        );
        response.clone().on_hover_ui(|ui| {
            ui.label(theme::strong(format!(
                "{}, {} · {}",
                forms::metres(at.x),
                forms::metres(at.y),
                if ok { "walkable" } else { "refused" }
            )));
            ui.label(theme::muted(format!("{why} · click to pick")).size(12.0));
        });
        if response.clicked() {
            if let Err(problem) = verdict {
                editor.toast(format!("{}: {}", problem.rule, problem.expected));
                return;
            }
            pins::commit_pick(editor, &area_key, at);
        }
    }
    form::hint(
        ui,
        &format!(
            "Click a cell for its centre, {} click for any point, in 1/16 m. {} keeps the old place.",
            EditorKey::Unsnapped.name(),
            EditorKey::Cancel.name()
        ),
    );
}
