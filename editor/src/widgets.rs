use engine::core::content::{ContentFile, LoadError};
use engine::systems::area::blueprint::BlueprintDef;
use engine::systems::area::{AreaDef, AreaModuleKind};
use engine::systems::item::ItemDef;
use engine::systems::npc::NpcDef;
use engine::systems::quest::QuestDef;
use engine::systems::script::RunnerKind;
use engine::systems::shop::ShopDef;
use engine::systems::skill::SkillDef;
use engine::systems::status::StatusEffectDef;
use ui::egui;
use ui::egui::{Color32, FontId, Rect, Response, RichText, Sense, Ui, Vec2};
use ui::glyph;
use ui::{ink, theme};

use crate::icons::{self, IconTone, KindFamily, KindIcon};
use crate::keymap::EditorKey;
use crate::tabs::shown_path;

pub(crate) fn shortcut_button<'a>(
    label: impl Into<egui::WidgetText>,
    key: EditorKey,
) -> egui::Button<'a> {
    egui::Button::new(label).shortcut_text(key.name())
}

/// A menu button reading `label` after a kind's icon, with its shortcut when it has one.
pub(crate) fn kind_button(
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
pub(crate) fn kind_option(ui: &mut Ui, kind: KindIcon, label: &str, selected: bool) -> Response {
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
pub(crate) enum KindMark {
    Icon(KindIcon),
    Badge(Badge),
}

impl KindMark {
    fn color(self) -> Color32 {
        match self {
            KindMark::Icon(icon) => icon.color(IconTone::Normal),
            KindMark::Badge(badge) => badge.color,
        }
    }

    fn paint(self, painter: &egui::Painter, rect: Rect) {
        match self {
            KindMark::Icon(icon) => icon.paint(painter, rect, IconTone::Normal),
            KindMark::Badge(badge) => paint_badge(painter, rect, badge),
        }
    }
}

/// A checkbox reading `label` after a kind's mark, both in the kind's colour.
pub(crate) fn kind_checkbox(ui: &mut Ui, mark: KindMark, label: &str, on: &mut bool) -> Response {
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

const BADGE_SIZE: f32 = 16.0;

pub(crate) fn badge(ui: &mut Ui, badge: Badge) {
    mark(ui, KindMark::Badge(badge));
}

/// What stands for a kind, at a badge's size.
pub(crate) fn mark(ui: &mut Ui, mark: KindMark) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(BADGE_SIZE), Sense::hover());
    mark.paint(ui.painter(), rect);
}

/// What stands for the content files of `table`: their icon.
pub(crate) fn table_mark(ui: &mut Ui, table: &str) {
    mark(ui, KindMark::Icon(icons::of_table(table)));
}

/// A letter in a coloured square, as the editor marks a kind of module or step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Badge {
    pub letter: char,
    pub color: Color32,
}

impl Badge {
    /// The badge of a kind of area module.
    pub(crate) fn of_module(kind: AreaModuleKind) -> Badge {
        let (letter, color) = match kind {
            AreaModuleKind::Object => ('P', ink::VERDIGRIS),
            AreaModuleKind::SafeZone => ('Z', ink::VERDIGRIS),
            AreaModuleKind::Soundscape => ('A', ink::GOLD),
            AreaModuleKind::Prop | AreaModuleKind::Scatter => ('T', ink::INK2),
            AreaModuleKind::Light => ('L', ink::INK2),
            AreaModuleKind::Blueprint => ('B', ink::AZURE),
            AreaModuleKind::Population | AreaModuleKind::Resident => ('N', ink::OCHRE),
            AreaModuleKind::Script => ('S', ink::TYRIAN),
        };
        Badge { letter, color }
    }

    /// The badge of a kind only its schema's tag names, among those of the content files of
    /// `table`: its first letter, in the colour of the table's family.
    pub(crate) fn of_tag(tag: &str, table: Option<&str>) -> Badge {
        Badge {
            letter: tag
                .chars()
                .next()
                .map_or(' ', |first| first.to_ascii_uppercase()),
            color: table
                .and_then(KindFamily::of_table)
                .map_or(ink::INK3, KindFamily::color),
        }
    }
}

/// What runs a script: an object of an area, by its badge, or the content files of a table, by
/// their icon.
pub(crate) fn runner_mark(kind: RunnerKind) -> KindMark {
    let table = match kind {
        RunnerKind::Object => return KindMark::Badge(Badge::of_module(AreaModuleKind::Object)),
        RunnerKind::Any => return KindMark::Badge(Badge::of_tag(kind.into(), None)),
        RunnerKind::Npc => NpcDef::TABLE,
        RunnerKind::Item => ItemDef::TABLE,
        RunnerKind::Area => AreaDef::TABLE,
        RunnerKind::Quest => QuestDef::TABLE,
        RunnerKind::Shop => ShopDef::TABLE,
        RunnerKind::Skill => SkillDef::TABLE,
        RunnerKind::StatusEffect => StatusEffectDef::TABLE,
        RunnerKind::Blueprint => BlueprintDef::TABLE,
    };
    KindMark::Icon(icons::of_table(table))
}

fn paint_badge(painter: &egui::Painter, rect: Rect, badge: Badge) {
    painter.rect_filled(rect, 3.0, badge.color);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        badge.letter,
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

/// The gutter every tab's content keeps from the tab body's edge, so views line up.
pub(crate) const VIEW_PADDING: egui::Margin = egui::Margin::symmetric(12, 8);
