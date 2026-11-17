//! Ways to go on with what is being typed, listed by the field it is typed in: what a code editor
//! shows at its caret.

use egui::{
    Align, Align2, Area, Context, Id, Layout, Margin, Order, Rect, Response, Sense, Stroke, Ui,
    UiBuilder, Vec2, WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::{float, form, ink, sheet};

/// One way to go on: what would be typed, what it is besides, and whether it is dimmed for being
/// refused for now. A suggestion that names a thing shows as `drawn` draws it, where it has a way.
pub struct Suggestion<'a> {
    pub text: &'a str,
    pub detail: &'a str,
    pub muted: bool,
    pub drawn: Option<&'a dyn DrawnSuggestion>,
}

/// A suggestion that shows as more than words: how wide it is, and how it draws itself in the
/// room its words would take.
pub trait DrawnSuggestion {
    fn width(&self) -> f32;
    fn draw(&self, ui: &mut Ui);
}

const ROW: f32 = 22.0;
const PAD: f32 = 8.0;
/// How far the list hangs from its field, and how far a row's detail stands from its text.
const GAP: f32 = 4.0;
const APART: f32 = 24.0;

/// Lists `suggestions` by the field at `field`: under it where the screen has the room, over it
/// otherwise. `heading`, when there is one, says what is being typed; the `selected` suggestion
/// is lit. Returns the place of the suggestion clicked.
/// What a list of suggestions answers: the place of the one clicked, and the row the pointer is
/// on, with its place.
pub struct Suggested {
    pub clicked: Option<usize>,
    pub pointed: Option<(usize, Response)>,
}

pub fn suggestions(
    gui: &Context,
    id: Id,
    field: Rect,
    heading: &str,
    suggestions: &[Suggestion],
    selected: usize,
) -> Suggested {
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
                    suggestion.drawn.map_or_else(
                        || width(TextRole::Mono, suggestion.text),
                        |drawn| drawn.width(),
                    ) + APART
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
            let mut pointed = None;
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
                let ink = if suggestion.muted {
                    ink::INK3
                } else {
                    ink::INK
                };
                match suggestion.drawn {
                    Some(drawn) => {
                        let room = UiBuilder::new()
                            .max_rect(rect.shrink2(Vec2::new(PAD, 0.0)))
                            .layout(Layout::left_to_right(Align::Center));
                        drawn.draw(&mut ui.new_child(room));
                    }
                    None => {
                        ui.painter().text(
                            rect.left_center() + Vec2::new(PAD, 0.0),
                            Align2::LEFT_CENTER,
                            suggestion.text,
                            TextRole::Mono.font_id(),
                            ink,
                        );
                    }
                }
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
                if response.hovered() {
                    pointed = Some((place, response));
                }
            }
            Suggested { clicked, pointed }
        })
        .inner
    })
    .inner
}
