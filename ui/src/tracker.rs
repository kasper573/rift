//! A tracker: a list lettered over the world under a heading that folds it away, on a dusk plate
//! as faint as the player sets it. Only the heading takes the pointer, so a click on what is
//! listed goes on to the world.

use egui::emath::TSTransform;
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, WidgetInfo,
    WidgetType,
};

use crate::face::TextRole;
use crate::glyph::Mark;
use crate::{dusk, ink};

pub const WIDTH: f32 = 244.0;
/// How far an entry's rows stand in from its bullet.
pub const INDENT: f32 = 16.0;
pub const ROW_GAP: f32 = 2.0;
pub const ENTRY_GAP: f32 = 6.0;

/// What a tracker is headed with: its title, how many it lists, and what those are called.
pub struct TrackerHeading<'a> {
    pub title: &'a str,
    pub listed: usize,
    pub lists: &'a str,
}

/// A tracker as it was drawn: the room it took, and whether its heading was clicked to fold it
/// or to open it again.
pub struct Tracked {
    pub drawn: Rect,
    pub toggled: bool,
}

/// What a row of a tracker tells, which tints it and may put a mark before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerTone {
    Plain,
    Done,
    Timed,
    Bad,
}

/// A line of a tracker: wrapped lettering `width` wide from `at`, behind a mark if it has one.
pub struct TrackerLine<'a> {
    pub at: Pos2,
    pub width: f32,
    pub mark: Option<(Mark, Color32)>,
    pub text: &'a str,
    pub font: FontId,
    pub color: Color32,
}

/// Draws a tracker: its heading, and unless it is `folded` what `entries` letters from the spot
/// it is given down to the foot it tells, all on a plate `opacity` of the way to solid.
pub fn show(
    ui: &mut Ui,
    heading: TrackerHeading,
    folded: bool,
    opacity: f32,
    entries: impl FnOnce(&mut Ui, Pos2) -> f32,
) -> Tracked {
    let backing = ui.painter().add(Shape::Noop);
    let (head, toggled) = head(ui, &heading, folded);
    let foot = if folded {
        head.bottom()
    } else {
        entries(ui, head.left_bottom())
    };
    let drawn = Rect::from_min_max(head.min, Pos2::new(head.right(), foot));
    ui.painter().set(
        backing,
        dusk::plate_shape(drawn.expand(PLATE_PADDING), opacity),
    );
    Tracked { drawn, toggled }
}

/// The diamond an entry opens with, for an entry whose first line starts at `at`.
pub fn bullet(ui: &Ui, at: Pos2) {
    let diamond = Pos2::new(at.x + DIAMOND + 2.0, at.y + TextRole::Body.size() * 0.62);
    for (grown, color) in [(1.5, Color32::BLACK), (0.0, ink::GOLD2)] {
        let corners = [Vec2::UP, Vec2::RIGHT, Vec2::DOWN, Vec2::LEFT]
            .map(|corner| diamond + corner * (DIAMOND + grown))
            .to_vec();
        ui.painter()
            .add(Shape::convex_polygon(corners, color, Stroke::NONE));
    }
}

impl TrackerTone {
    // A tint goes with what the row's mark and words already say, and stays pale enough to read
    // on the edge every letter carries.
    pub fn color(self) -> Color32 {
        match self {
            TrackerTone::Plain => Color32::WHITE,
            TrackerTone::Done => Color32::from_rgb(0xDA, 0xF5, 0xDD),
            TrackerTone::Timed => Color32::from_rgb(0xFF, 0xEB, 0xB8),
            TrackerTone::Bad => Color32::from_rgb(0xFF, 0xD9, 0xD2),
        }
    }

    pub fn mark(self) -> Option<Mark> {
        match self {
            TrackerTone::Plain => None,
            TrackerTone::Done => Some(Mark::Check),
            TrackerTone::Timed => Some(Mark::Hourglass),
            TrackerTone::Bad => Some(Mark::Cross),
        }
    }
}

/// Letters the `mark` of a row at `at`, and tells how far the row's words stand in behind it.
pub fn mark(ui: &Ui, at: Pos2, mark: Mark, color: Color32, size: f32) -> f32 {
    let icon = mark.icon();
    let font = FontId::new(size, icon.weight.family());
    let galley = ui
        .painter()
        .layout_no_wrap(icon.glyph.to_owned(), font, color);
    let width = galley.size().x;
    dusk::lettering(ui.painter(), at, galley, dusk::EDGED, TSTransform::IDENTITY);
    width + MARK_GAP
}

impl TrackerLine<'_> {
    pub fn letter(self, ui: &mut Ui, id: Id) -> Rect {
        let mut job = LayoutJob::default();
        job.wrap.max_width = self.width;
        if let Some((mark, color)) = self.mark {
            let icon = mark.icon();
            let icons = FontId::new(self.font.size, icon.weight.family());
            let format = TextFormat {
                valign: Align::Center,
                ..TextFormat::simple(icons, color)
            };
            job.append(icon.glyph, 0.0, format);
        }
        let lead = if self.mark.is_some() { MARK_GAP } else { 0.0 };
        job.append(self.text, lead, TextFormat::simple(self.font, self.color));
        let galley = ui.painter().layout_job(job);
        let rect = Rect::from_min_size(self.at, galley.size());
        ui.interact(rect, id, Sense::hover())
            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, self.text));
        dusk::lettering(
            ui.painter(),
            self.at,
            galley,
            dusk::EDGED,
            TSTransform::IDENTITY,
        );
        rect
    }
}

fn head(ui: &mut Ui, heading: &TrackerHeading, folded: bool) -> (Rect, bool) {
    let (rect, pressed) = ui.allocate_exact_size(Vec2::new(WIDTH, HEADING), Sense::click());
    let (title, lists) = (heading.title, heading.lists);
    let title = if folded {
        format!("{title} ({})", heading.listed)
    } else {
        title.to_owned()
    };
    let said = if folded {
        format!("{title} · show {lists}")
    } else {
        format!("{title} · hide {lists}")
    };
    pressed.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, said.clone()));
    let galley = ui
        .painter()
        .layout_no_wrap(title, TextRole::Section.font_id(), ink::GOLD2);
    let at = Align2::LEFT_CENTER.anchor_size(rect.left_center(), galley.size());
    dusk::lettering(
        ui.painter(),
        at.min,
        galley,
        dusk::EDGED,
        TSTransform::IDENTITY,
    );
    let tip = if folded { Vec2::RIGHT } else { Vec2::DOWN };
    caret(ui, rect.right_center() - Vec2::new(CARET + 2.0, 0.0), tip);
    (rect, pressed.clicked())
}

fn caret(ui: &Ui, at: Pos2, tip: Vec2) {
    let across = tip.rot90();
    let shape = |grown: f32, color: Color32| {
        let size = CARET + grown;
        Shape::convex_polygon(
            vec![
                at + tip * size,
                at - tip * size * 0.6 + across * size,
                at - tip * size * 0.6 - across * size,
            ],
            color,
            Stroke::NONE,
        )
    };
    ui.painter().add(shape(1.5, Color32::BLACK));
    ui.painter().add(shape(0.0, ink::GOLD2));
}

const HEADING: f32 = 20.0;
const DIAMOND: f32 = 4.0;
const CARET: f32 = 4.5;
const PLATE_PADDING: f32 = 8.0;
const MARK_GAP: f32 = 4.0;
