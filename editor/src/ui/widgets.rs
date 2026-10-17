use std::fmt::Display;

use egui::{Align, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};

use super::theme;

pub use design_tokens::widgets::{
    card_frame, chip, chip_colored, combo_box, danger, key_hint, pill, primary, section,
    section_heading, tab_strip, toggle,
};

pub fn section_count(ui: &mut Ui, text: &str, count: impl Display) {
    section(ui, &format!("{text} · {count}"));
}

pub fn kv(ui: &mut Ui, key: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(96.0, ui.spacing().interact_size.y),
            egui::Label::new(theme::muted(key)),
        );
        value(ui);
    });
}

pub fn verdict(ui: &mut Ui, ok: bool, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(if ok { "✓" } else { "!" }).color(if ok {
            theme::OK
        } else {
            theme::WARN
        }));
        ui.label(RichText::new(text).size(12.5));
    });
}

pub fn kv_text(ui: &mut Ui, key: &str, value: impl Into<String>) {
    kv(ui, key, |ui| {
        ui.label(value.into());
    });
}

pub fn kv_code(ui: &mut Ui, key: &str, value: impl Into<String>) {
    kv(ui, key, |ui| {
        ui.label(theme::mono(value.into()));
    });
}

pub fn keyed_button(ui: &mut Ui, label: &str, keys: &str, enabled: bool) -> Response {
    let label_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), theme::TEXT)
        .size()
        .x;
    let keys_width = ui
        .painter()
        .layout_no_wrap(keys.to_owned(), FontId::monospace(11.0), theme::TEXT)
        .size()
        .x;
    let size = Vec2::new(label_width + keys_width + 36.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let ink = if enabled { theme::TEXT } else { theme::FAINT };
    let fill = if enabled && response.hovered() {
        theme::INPUT
    } else {
        theme::RAISED
    };
    ui.painter().rect(
        rect,
        4.0,
        fill,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.min.x + 8.0;
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::new(14.0, theme::medium()),
        ink,
    );
    x += label_width + 8.0;
    let cap = egui::Rect::from_min_size(
        egui::pos2(x, rect.center().y - 9.0),
        Vec2::new(keys_width + 10.0, 18.0),
    );
    ui.painter().rect(
        cap,
        3.0,
        theme::INPUT,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        cap.center(),
        egui::Align2::CENTER_CENTER,
        keys,
        FontId::monospace(11.0),
        if enabled { theme::MUTED } else { theme::FAINT },
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

pub fn key_hint_text(ui: &mut Ui, keys: &str, text: &str) {
    ui.horizontal(|ui| {
        key_hint(ui, keys);
        ui.label(theme::muted(text));
    });
}

pub fn chip_closable(ui: &mut Ui, text: &str) -> (Response, bool) {
    let mut closed = false;
    let response = egui::Frame::new()
        .fill(theme::ACCENT_SOFT)
        .stroke(Stroke::new(1.0, theme::ACCENT))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(text).size(12.5).monospace());
                if ui
                    .add(
                        egui::Label::new(RichText::new("✕").size(11.0).color(theme::MUTED))
                            .sense(Sense::click()),
                    )
                    .clicked()
                {
                    closed = true;
                }
            });
        })
        .response;
    (response.interact(Sense::click()), closed)
}

pub const BADGE_SIZE: f32 = 16.0;

pub fn badge(ui: &mut Ui, kind: &str) {
    let color = theme::badge_color(kind);
    let letter = theme::badge_letter(kind);
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(BADGE_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::monospace(11.0),
        Color32::BLACK,
    );
}

pub fn card_header(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    menu: impl FnOnce(&mut Ui),
) -> Response {
    let response = ui
        .horizontal(|ui| {
            ui.label(theme::strong(title).monospace());
            if !subtitle.is_empty() {
                ui.label(
                    RichText::new(subtitle.to_uppercase())
                        .size(11.0)
                        .color(theme::MUTED),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(theme::faint("…"), menu);
            });
        })
        .response;
    response.interact(Sense::click())
}

pub fn code_block(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace().size(12.5)).selectable(true));
        });
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED).size(12.5));
}

pub fn search_box(
    ui: &mut Ui,
    text: &mut String,
    placeholder: &str,
    key: Option<&str>,
) -> Response {
    let mut inner = None;
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::muted("🔍"));
                let width = ui.available_width() - key.map_or(0.0, |k| 14.0 + 7.0 * k.len() as f32);
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(placeholder)
                        .frame(egui::Frame::NONE)
                        .desired_width(width.max(40.0)),
                );
                if let Some(key) = key {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| key_hint(ui, key));
                }
                inner = Some(response);
            });
        });
    inner.expect("search field")
}

pub fn plural<N: PartialEq + From<u8>>(count: N) -> &'static str {
    if count == N::from(1) { "" } else { "s" }
}

pub fn toggle_chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let (fill, ink) = if on {
        (theme::ACCENT_SOFT, theme::TEXT)
    } else {
        (theme::RAISED, theme::MUTED)
    };
    chip_colored(ui, text, fill, ink)
}

pub fn mode_chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let (fill, ink) = if on {
        (theme::ACCENT, Color32::WHITE)
    } else {
        (theme::RAISED, theme::TEXT)
    };
    chip_colored(ui, text, fill, ink)
}

pub fn card_title(ui: &mut Ui, title: &str) -> Response {
    card_header(ui, title, "", |_| {})
}

pub fn rule(ui: &Ui) {
    let rect = ui.max_rect();
    let y = ui.cursor().min.y;
    ui.painter().line_segment(
        [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
        Stroke::new(1.0, theme::BORDER),
    );
}

pub fn dashed_rect(painter: &egui::Painter, rect: Rect, stroke: Stroke, dash: f32, gap: f32) {
    painter.extend(egui::Shape::dashed_line(
        &[
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ],
        stroke,
        dash,
        gap,
    ));
}

pub fn floating_chip(
    painter: &egui::Painter,
    at: egui::Pos2,
    anchor: egui::Align2,
    text: &str,
    fill: Color32,
    ink: Color32,
    size: f32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), ink);
    let rect = anchor.anchor_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(rect, 3.0, fill);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Outside,
    );
    painter.galley(rect.min + Vec2::new(5.0, 2.0), galley, ink);
    rect
}

pub fn text_within(
    painter: &egui::Painter,
    at: egui::Pos2,
    anchor: egui::Align2,
    text: &str,
    font: FontId,
    ink: Color32,
    width: f32,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, ink);
    rect
}

pub fn default_focus(ui: &Ui, response: &Response) {
    if ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
}

pub fn digit_pressed(ui: &Ui) -> Option<usize> {
    const DIGITS: [egui::Key; 9] = [
        egui::Key::Num1,
        egui::Key::Num2,
        egui::Key::Num3,
        egui::Key::Num4,
        egui::Key::Num5,
        egui::Key::Num6,
        egui::Key::Num7,
        egui::Key::Num8,
        egui::Key::Num9,
    ];
    ui.input(|i| DIGITS.iter().position(|key| i.key_pressed(*key)))
}

pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

pub fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn clock(unix_seconds: u64, with_seconds: bool) -> String {
    let seconds = unix_seconds % 86_400;
    let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
    if with_seconds {
        format!("{hours:02}:{minutes:02}:{:02}", seconds % 60)
    } else {
        format!("{hours:02}:{minutes:02} UTC")
    }
}

pub fn ago(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if seconds < 60 {
        "just now".to_owned()
    } else if minutes < 60 {
        format!("{minutes} min ago")
    } else if hours < 24 {
        format!("{hours} h ago")
    } else if days == 1 {
        "yesterday".to_owned()
    } else if days < 7 {
        format!("{days} days ago")
    } else if days < 14 {
        "last week".to_owned()
    } else if days < 31 {
        format!("{} weeks ago", days / 7)
    } else {
        format!("{} months ago", days / 30)
    }
}
