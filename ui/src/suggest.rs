//! Ways to go on with what is being typed, listed by the field it is typed in: what a code editor
//! shows at its caret.

use egui::{
    Align2, Area, Context, Id, Margin, Order, Rect, Sense, Stroke, Vec2, WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::{float, form, ink, sheet};

/// One way to go on: what would be typed, and what it is besides.
pub struct Suggestion<'a> {
    pub text: &'a str,
    pub detail: &'a str,
}

const ROW: f32 = 22.0;
const PAD: f32 = 8.0;
/// How far the list hangs from its field, and how far a row's detail stands from its text.
const GAP: f32 = 4.0;
const APART: f32 = 24.0;

/// Lists `suggestions` by the field at `field`: under it where the screen has the room, over it
/// otherwise. `heading`, when there is one, says what is being typed; the `selected` suggestion
/// is lit. Returns the place of the suggestion clicked.
pub fn suggestions(
    gui: &Context,
    id: Id,
    field: Rect,
    heading: &str,
    suggestions: &[Suggestion],
    selected: usize,
) -> Option<usize> {
    let rows = suggestions.len() + usize::from(!heading.is_empty());
    let tall = rows as f32 * ROW + 2.0 * PAD;
    let under = field.bottom() + GAP + tall <= gui.content_rect().bottom();
    let (pivot, at) = if under {
        (Align2::LEFT_TOP, field.left_bottom() + Vec2::new(0.0, GAP))
    } else {
        (Align2::LEFT_BOTTOM, field.left_top() - Vec2::new(0.0, GAP))
    };
    let area = Area::new(id)
        .order(Order::Tooltip)
        .pivot(pivot)
        .fixed_pos(at)
        .fade_in(false);
    float::anchored(gui, area, |ui| {
        sheet::note(ui, Margin::same(PAD as i8), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = |role: TextRole, text: &str| {
                ui.painter()
                    .layout_no_wrap(text.to_owned(), role.font_id(), ink::INK)
                    .size()
                    .x
            };
            let wide = suggestions
                .iter()
                .map(|suggestion| {
                    width(TextRole::Mono, suggestion.text)
                        + APART
                        + width(TextRole::Small, suggestion.detail)
                })
                .fold(width(TextRole::Small, heading), f32::max)
                + 2.0 * PAD;
            if !heading.is_empty() {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(wide, ROW), Sense::hover());
                ui.painter().text(
                    rect.left_center() + Vec2::new(PAD, 0.0),
                    Align2::LEFT_CENTER,
                    heading,
                    TextRole::Small.font_id(),
                    ink::INK2,
                );
                if !suggestions.is_empty() {
                    form::rule_with(ui, Stroke::new(1.0, ink::ink_wash(0.18)));
                }
            }
            let mut clicked = None;
            for (place, suggestion) in suggestions.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(wide, ROW), Sense::click());
                response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, suggestion.text));
                if place == selected {
                    ui.painter().rect_filled(rect, 2.0, ink::HONEY2);
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 2.0, ink::wash(ink::HONEY2, 0.45));
                }
                ui.painter().text(
                    rect.left_center() + Vec2::new(PAD, 0.0),
                    Align2::LEFT_CENTER,
                    suggestion.text,
                    TextRole::Mono.font_id(),
                    ink::INK,
                );
                ui.painter().text(
                    rect.right_center() - Vec2::new(PAD, 0.0),
                    Align2::RIGHT_CENTER,
                    suggestion.detail,
                    TextRole::Small.font_id(),
                    ink::INK2,
                );
                if response.clicked() {
                    clicked = Some(place);
                }
            }
            clicked
        })
        .inner
    })
    .inner
}
