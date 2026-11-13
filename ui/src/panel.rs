//! Panels and the cards inside them: a card of related fields and its heading, the body a panel
//! clips to, the hairline between panels and the resizable column of a sidebar.

use std::hash::Hash;

use egui::{Align, Id, Layout, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, UiBuilder, Vec2};

use crate::face;
use crate::face::TextRole;
use crate::glyph::Mark;
use crate::{ink, sheet, theme};

/// The frame of a card, picked out when it is `selected`.
pub fn tile(selected: bool) -> egui::Frame {
    let (fill, border) = card_ink(selected);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// A tile across the width available, picked out when it is `selected`.
pub fn tile_across<R>(
    ui: &mut Ui,
    selected: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    tile(selected).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
}

/// A tile under its `name`.
pub fn titled_tile<R>(
    ui: &mut Ui,
    selected: bool,
    name: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    tile_across(ui, selected, |ui| {
        title(ui, name);
        ui.add_space(4.0);
        add(ui)
    })
}

/// A card's look painted on a rect laid out by hand.
pub fn paint_tile(painter: &Painter, rect: Rect, selected: bool) {
    let (fill, border) = card_ink(selected);
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
}

/// A card's heading: its title, a subtitle at the right and a menu behind its more mark.
pub fn header(ui: &mut Ui, title: &str, subtitle: &str, menu: impl FnOnce(&mut Ui)) -> Response {
    header_with(ui, |ui| titled(ui, title), subtitle, menu)
}

/// A card's heading whose title is drawn by `title`, for one that is more than a name.
pub fn header_with(
    ui: &mut Ui,
    title: impl FnOnce(&mut Ui),
    subtitle: &str,
    menu: impl FnOnce(&mut Ui),
) -> Response {
    heading(ui, title, subtitle, |ui| {
        ui.menu_button(Mark::More.text(16.0).color(theme::FAINT), menu);
    })
}

pub fn title(ui: &mut Ui, title: &str) -> Response {
    heading(ui, |ui| titled(ui, title), "", |_| {})
}

/// A card's heading with a subtitle at its right and nothing behind it.
pub fn subtitled(ui: &mut Ui, title: &str, subtitle: &str) -> Response {
    heading(ui, |ui| titled(ui, title), subtitle, |_| {})
}

/// A box sunk into a panel, holding a list or text that is read rather than typed into.
pub fn well() -> egui::Frame {
    crate::input::frame().inner_margin(8.0)
}

/// Text to read or copy as it is, such as a path or a report.
pub fn code_block(ui: &mut Ui, text: &str) {
    well().show(ui, |ui| {
        ui.add(egui::Label::new(face::mono(text).size(12.5)).selectable(true));
    });
}

/// The body of a panel filling `rect`, top down and clipped to it.
pub fn body<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    let inner = add(&mut body);
    // Only `rect` is taken, so what overflows it for a frame, such as a scroll bar appearing,
    // never widens a resizable panel the body sits in.
    ui.advance_cursor_after_rect(rect);
    inner
}

/// Draws `add` while it is in view. Out of view it only takes the room it took when last drawn at
/// this width, as what a long list or form holds out of sight would otherwise cost every frame;
/// `id` names what is drawn, so it must change when that does. While a widget holds focus or a
/// popup is open everything is drawn, since they close once their widget goes undrawn.
pub fn in_view<R>(ui: &mut Ui, id: egui::Id, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let width = ui.available_width();
    let busy = ui.memory(|memory| memory.focused().is_some()) || egui::Popup::is_any_open(ui.ctx());
    let held = ui.data(|data| data.get_temp::<(f32, Vec2)>(id));
    if let Some((drawn_at, size)) = held
        && drawn_at == width
        && !busy
        && !ui.is_rect_visible(Rect::from_min_size(ui.cursor().min, size))
    {
        ui.allocate_space(size);
        return None;
    }
    let drawn = ui.scope(add);
    let size = drawn.response.rect.size();
    ui.data_mut(|data| data.insert_temp(id, (width, size)));
    Some(drawn.inner)
}

/// The body of a panel filling the room left in it, top down and clipped to it.
pub fn rest<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    body(ui, rect, add)
}

/// The room left in a panel parted in two side by side, the first `width` wide, with the hairline
/// between them drawn: each to fill with `body`.
pub fn split(ui: &Ui, width: f32) -> [Rect; 2] {
    let rect = ui.available_rect_before_wrap();
    let width = width.min(rect.width());
    let left = Rect::from_min_size(rect.min, Vec2::new(width, rect.height()));
    let right = Rect::from_min_max(egui::pos2(left.right() + SPLIT_GAP, rect.top()), rect.max);
    divide(
        ui,
        left.expand2(Vec2::new(SPLIT_GAP / 2.0, 0.0)),
        Edge::Right,
    );
    [left, right]
}

const SPLIT_GAP: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The hairline at a panel's `edge`, parting it from the one beside it.
pub fn divide(ui: &Ui, panel: Rect, edge: Edge) {
    let stroke = Stroke::new(1.0, ink::ink_wash(0.35));
    let painter = ui.painter();
    match edge {
        Edge::Top => painter.hline(panel.x_range(), panel.top() + 0.5, stroke),
        Edge::Bottom => painter.hline(panel.x_range(), panel.bottom() - 0.5, stroke),
        Edge::Left => painter.vline(panel.left() + 0.5, panel.y_range(), stroke),
        Edge::Right => painter.vline(panel.right() - 0.5, panel.y_range(), stroke),
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A resizable column on one `side` of its window, its inner edge drawn.
pub fn column(ui: &mut Ui, id: impl Hash + std::fmt::Debug, side: Side, add: impl FnOnce(&mut Ui)) {
    let id = egui::Id::new(("column", id));
    let (panel, divider) = match side {
        Side::Left => (egui::Panel::left(id), Edge::Right),
        Side::Right => (egui::Panel::right(id), Edge::Left),
    };
    let column = panel
        .resizable(true)
        .default_size(360.0)
        .size_range(260.0..=720.0)
        .show_separator_line(false)
        .frame(egui::Frame::new())
        .show(ui, |ui| rest(ui, add));
    divide(ui, column.response.rect, divider);
}

/// What a column holds, scrolling when it outgrows it.
pub fn scrolled(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt("column")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new().inner_margin(PADDING).show(ui, add);
        });
}

/// The cross that closes a window, at `rect`.
pub fn close_button(ui: &mut Ui, id: Id, rect: Rect) -> Response {
    let response = ui.interact(rect, id.with("close"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close")
    });
    sheet::cross(ui.painter(), rect, response.hovered());
    response
}

/// Which corner a resize grip sits in: the bottom right unless there is no room to pull there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GripCorner {
    BottomRight,
    TopRight,
}

/// The grip in a window's curled corner that resizes it.
pub fn corner_grip(ui: &mut Ui, id: Id, curl: Rect) -> Response {
    let response = ui
        .interact(curl, id.with("grip"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ResizeHandle, ui.is_enabled(), "Resize")
    });
    response
}

/// A grip in a slip's `at` corner that resizes it, hatched to show where to take hold.
pub fn grip(ui: &mut Ui, id: Id, slip: Rect, at: GripCorner) -> Response {
    let size = GRIP;
    let (corner, cursor) = match at {
        GripCorner::BottomRight => (
            Rect::from_min_max(slip.max - Vec2::splat(size), slip.max),
            egui::CursorIcon::ResizeNwSe,
        ),
        GripCorner::TopRight => (
            Rect::from_min_max(
                Pos2::new(slip.max.x - size, slip.min.y),
                Pos2::new(slip.max.x, slip.min.y + size),
            ),
            egui::CursorIcon::ResizeNeSw,
        ),
    };
    let response = ui
        .interact(corner, id.with("grip"), Sense::drag())
        .on_hover_cursor(cursor);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ResizeHandle, ui.is_enabled(), "Resize")
    });
    let color = if response.hovered() || response.dragged() {
        ink::INK
    } else {
        ink::INK3
    };
    let (tip, down) = match at {
        GripCorner::BottomRight => (corner.max - Vec2::splat(3.0), -1.0),
        GripCorner::TopRight => (corner.right_top() + Vec2::new(-3.0, 3.0), 1.0),
    };
    let mut reach = 3.0;
    while reach <= size - 3.0 {
        ui.painter().line_segment(
            [
                tip - Vec2::new(reach, 0.0),
                tip + Vec2::new(0.0, down * reach),
            ],
            Stroke::new(1.0, color),
        );
        reach += 4.0;
    }
    response
}

/// The room between a column's edge and what it holds.
pub const PADDING: egui::Margin = egui::Margin::symmetric(10, 8);

const RADIUS: f32 = 4.0;
const GRIP: f32 = 14.0;

fn card_ink(selected: bool) -> (egui::Color32, egui::Color32) {
    if selected {
        (theme::ACCENT_SOFT, theme::ACCENT)
    } else {
        (theme::INPUT, theme::BORDER)
    }
}

fn titled(ui: &mut Ui, title: &str) {
    ui.add(egui::Label::new(theme::strong(title).monospace()).truncate());
}

/// A card's heading: `trailing` and the subtitle at its right, the title in the room they leave.
fn heading(
    ui: &mut Ui,
    title: impl FnOnce(&mut Ui),
    subtitle: &str,
    trailing: impl FnOnce(&mut Ui),
) -> Response {
    let response = ui
        .horizontal(|ui| {
            ui.set_min_height(ui.spacing().interact_size.y);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                trailing(ui);
                if !subtitle.is_empty() {
                    ui.label(TextRole::Band.widget_text(subtitle, theme::MUTED));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), title);
            });
        })
        .response;
    response.interact(Sense::click())
}

/// A view's right panel: the inspector of what is selected inside it. A resizable column on the
/// right of its window, whose `body` works on `state`.
pub fn inspector<S>(ui: &mut Ui, state: &mut S, id: &str, body: impl FnOnce(&mut S, &mut Ui)) {
    column(ui, ("inspector", id), Side::Right, |ui| {
        sheet::tabs(
            ui,
            sheet::TabSize::Sub,
            &[sheet::Tab::new((), "Inspector")],
            &mut (),
        );
        scrolled(ui, |ui| body(state, ui));
    });
}

/// The row a view works on, laid out on the left of its window.
pub fn left_scrolled(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    column(ui, ("left scrolled", id), Side::Left, |ui| {
        scrolled(ui, body)
    });
}

/// The left column of a view whose body scrolls what it must itself, such as a tree under a few
/// fixed boxes.
pub fn left_framed(ui: &mut Ui, id: &str, body: impl FnOnce(&mut Ui)) {
    column(ui, ("left framed", id), Side::Left, |ui| {
        egui::Frame::new().inner_margin(PADDING).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            body(ui)
        });
    });
}

/// The folders above the last segment of `path`, each clickable, then `name`. Answers the path of
/// the folder clicked.
pub fn breadcrumb(ui: &mut Ui, path: &str, name: &str) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut folder = String::new();
        let segments: Vec<&str> = path.split('/').collect();
        for segment in segments.split_last().map_or(&[][..], |(_, above)| above) {
            if !folder.is_empty() {
                folder.push('/');
            }
            folder.push_str(segment);
            let response = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(*segment)
                            .monospace()
                            .color(theme::MUTED),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Show in the content tree");
            if response.clicked() {
                clicked = Some(folder.clone());
            }
            ui.label(theme::faint("/"));
        }
        ui.label(theme::strong(name).monospace());
    });
    clicked
}
