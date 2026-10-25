use engine::core::content::LoadError;
use engine::core::time::{Seconds, UnixMillis};
use ui::egui;
use ui::egui::{Color32, FontId, Rect, Response, RichText, Sense, Ui, Vec2};
use ui::glyph;
use ui::{ink, theme};

use crate::icons::{IconTone, KindIcon};
use crate::keymap::EditorKey;
use crate::tabs::shown_path;

pub fn shortcut_button<'a>(label: impl Into<egui::WidgetText>, key: EditorKey) -> egui::Button<'a> {
    egui::Button::new(label).shortcut_text(key.name())
}

/// A menu button reading `label` after a kind's icon, with its shortcut when it has one.
pub fn kind_button(
    ui: &mut Ui,
    kind: KindIcon,
    label: &str,
    shortcut: Option<EditorKey>,
) -> Response {
    let id = ui.id().with(("kind button", label));
    let mut button = egui::Button::new((egui::Atom::custom(id, Vec2::splat(KIND_ICON)), label));
    if let Some(key) = shortcut {
        button = button.shortcut_text(key.name());
    }
    let laid = button.atom_ui(ui);
    paint_kind(ui, kind, id, laid)
}

/// A line of a list to pick from, reading `label` after a kind's icon.
pub fn kind_option(ui: &mut Ui, kind: KindIcon, label: &str, selected: bool) -> Response {
    let id = ui.id().with(("kind option", label));
    let button = egui::Button::selectable(
        selected,
        (egui::Atom::custom(id, Vec2::splat(KIND_ICON)), label),
    );
    let laid = button.atom_ui(ui);
    paint_kind(ui, kind, id, laid)
}

/// What stands for a kind: a file kind's icon, or the lettered badge of a module kind.
#[derive(Clone, Copy)]
pub enum KindMark<'a> {
    Icon(KindIcon),
    Badge(&'a str),
}

impl KindMark<'_> {
    fn color(self) -> Color32 {
        match self {
            KindMark::Icon(icon) => icon.color(IconTone::Normal),
            KindMark::Badge(kind) => badge_color(kind),
        }
    }

    fn paint(self, painter: &egui::Painter, rect: Rect) {
        match self {
            KindMark::Icon(icon) => icon.paint(painter, rect, IconTone::Normal),
            KindMark::Badge(kind) => paint_badge(painter, rect, kind),
        }
    }
}

/// A checkbox reading `label` after a kind's mark, both in the kind's colour.
pub fn kind_checkbox(ui: &mut Ui, mark: KindMark, label: &str, on: &mut bool) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut response = ui.add(egui::Checkbox::without_text(on));
        let id = ui.id().with(("kind checkbox", label));
        let text = egui::RichText::new(label).color(mark.color());
        let laid = egui::Button::new((egui::Atom::custom(id, Vec2::splat(KIND_ICON)), text))
            .frame(false)
            .atom_ui(ui);
        if let Some(rect) = laid.rect(id) {
            mark.paint(ui.painter(), rect);
        }
        let named = laid.response;
        if named.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        response | named
    })
    .inner
}

const KIND_ICON: f32 = 14.0;

fn paint_kind(ui: &Ui, kind: KindIcon, id: egui::Id, laid: egui::AtomLayoutResponse) -> Response {
    if let Some(rect) = laid.rect(id) {
        let tone = if ui.is_enabled() {
            IconTone::Normal
        } else {
            IconTone::Dim
        };
        kind.paint(ui.painter(), rect, tone);
    }
    laid.response
}

pub const BADGE_SIZE: f32 = 16.0;

pub fn badge(ui: &mut Ui, kind: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(BADGE_SIZE), Sense::hover());
    paint_badge(ui.painter(), rect, kind);
}

pub fn badge_color(kind: &str) -> Color32 {
    match kind {
        "population" | "resident" | "npc" | "player" | "nameplate" | "tint" | "voice" => ink::OCHRE,
        "object" | "safe_zone" | "area" | "walk" | "terrain" => ink::VERDIGRIS,
        "script" | "shown" | "requires" => ink::TYRIAN,
        "soundscape" | "sfx" | "babble" | "step" => ink::GOLD,
        "loot" | "item" | "stack" | "reward" | "hold" | "deliver" | "offer" | "shop" => ink::AZURE,
        "blueprint" | "prop" | "scatter" | "light" | "model" | "bust" => ink::INK2,
        "respawn" | "quest" | "defeat" | "hand_in" | "task" | "decide" | "pick_one" => ink::RUBRIC,
        _ => ink::INK3,
    }
}

pub fn badge_letter(kind: &str) -> String {
    match kind {
        "script" => "S".to_owned(),
        "population" | "resident" | "npc" => "N".to_owned(),
        "object" => "P".to_owned(),
        "safe_zone" => "Z".to_owned(),
        "soundscape" => "A".to_owned(),
        "blueprint" => "L".to_owned(),
        "prop" | "scatter" => "T".to_owned(),
        "loot" | "item" => "I".to_owned(),
        "respawn" => "F".to_owned(),
        "quest" => "R".to_owned(),
        "tint" | "voice" | "nameplate" => "L".to_owned(),
        other => other
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_default(),
    }
}

fn paint_badge(painter: &egui::Painter, rect: Rect, kind: &str) {
    painter.rect_filled(rect, 3.0, badge_color(kind));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        badge_letter(kind),
        FontId::monospace(rect.height() * 0.7),
        ink::CREAM,
    );
}

/// Why the content file at `path` can't be shown: the errors that keep the game from reading it,
/// each where it is in the file.
pub fn unreadable(ui: &mut Ui, path: &str, errors: &[LoadError]) {
    ui.horizontal(|ui| {
        glyph::alert(ui.painter(), ui.cursor().min + Vec2::new(8.0, 9.0), 7.0);
        ui.add_space(20.0);
        ui.label(RichText::new(format!("{} doesn't read", shown_path(path))).strong());
    });
    for error in errors {
        ui.label(
            RichText::new(format!(
                "{} · {}",
                shown_path(&error.at.location(&error.file)),
                error.problem
            ))
            .color(theme::MUTED)
            .size(12.5),
        );
    }
}

/// The time of day `at` falls on in the user's own time zone.
pub fn clock(at: UnixMillis, with_seconds: bool) -> String {
    let format = if with_seconds { "%H:%M:%S" } else { "%H:%M" };
    i64::try_from(at.0)
        .ok()
        .and_then(|millis| jiff::Timestamp::from_millisecond(millis).ok())
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime(format)
                .to_string()
        })
        .unwrap_or_default()
}

pub fn ago(elapsed: Seconds) -> String {
    let seconds = elapsed.0.max(0.0) as u64;
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
