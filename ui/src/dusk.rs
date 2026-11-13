//! The dark ground of notices: what floats over the world to tell the player something stays apart
//! from the paper of windows and cards.

use std::sync::Arc;

use egui::emath::TSTransform;
use egui::epaint::{Shadow, TextShape};
use egui::{
    Color32, CornerRadius, FontId, Frame, Galley, Margin, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::ink::{Ground, Prose};
use crate::paint::TextShadow;

pub const FILL: Color32 = Color32::from_rgb(38, 38, 43);
pub const BORDER: Color32 = Color32::from_rgb(50, 50, 57);
pub const HOVER: Color32 = Color32::from_rgb(48, 48, 54);
pub const TEXT: Color32 = Color32::from_rgb(228, 228, 232);
pub const DANGER: Color32 = Prose::Danger.on(Ground::Dusk);
/// A player's name floating over the world.
pub const PLAYER: Color32 = Prose::Place.on(Ground::Dusk);
pub const TOWNSPERSON: Color32 = Prose::Name.on(Ground::Dusk);
pub const NOTICE: Color32 = Prose::Item.on(Ground::Dusk);

pub const RADIUS: CornerRadius = CornerRadius::same(6);
pub const SHADOW: Shadow = Shadow {
    offset: [0, 6],
    blur: 18,
    spread: 0,
    color: Color32::from_black_alpha(140),
};

/// Under text floating over the world with no ground of its own: a soft drop below it.
pub const TEXT_SHADOW: &[TextShadow] = &[TextShadow {
    offset: Vec2::new(0.0, 2.0),
    color: Color32::from_black_alpha(204),
}];
/// Under small text floating over the world: a hard edge a pixel down and right.
pub const SMALL_TEXT_SHADOW: &[TextShadow] = &[TextShadow {
    offset: Vec2::new(1.0, 1.0),
    color: Color32::BLACK,
}];
/// Under text that must read over the brightest ground the world can put behind it: a glow
/// around it, an edge on every side of each letter and a hard shadow below, so the letter stands
/// on its own dark ground wherever it is.
pub const EDGED: &[TextShadow] = &[
    glow(2.0, 0.0),
    glow(-2.0, 0.0),
    glow(0.0, 2.0),
    glow(0.0, -2.0),
    glow(1.5, 1.5),
    glow(-1.5, 1.5),
    glow(1.5, -1.5),
    glow(-1.5, -1.5),
    edge(1.0, 0.0),
    edge(-1.0, 0.0),
    edge(0.0, 1.0),
    edge(0.0, -1.0),
    edge(1.0, 2.0),
];

/// The dusk ground laid behind what floats over the world, `opacity` of the way from clear to
/// solid, for text that stays whole whatever the plate under it is set to.
pub fn plate(painter: &Painter, rect: Rect, opacity: f32) {
    painter.add(plate_shape(rect, opacity));
}

pub fn plate_shape(rect: Rect, opacity: f32) -> Shape {
    Shape::rect_filled(rect, RADIUS, FILL.gamma_multiply(opacity.clamp(0.0, 1.0)))
}

/// Lettering over the world: `galley` at `at` with a copy of itself as each of `under` lays it
/// beneath. `shift` moves and scales the drawing while the space it takes stays put.
pub fn lettering(
    painter: &Painter,
    at: Pos2,
    galley: Arc<Galley>,
    under: &[TextShadow],
    shift: TSTransform,
) {
    for shadow in under {
        let mut copy = TextShape::new(at + shadow.offset, galley.clone(), shadow.color);
        copy.override_text_color = Some(shadow.color);
        copy.transform(shift);
        painter.add(Shape::Text(copy));
    }
    let mut over = TextShape::new(at, galley, Color32::PLACEHOLDER);
    over.transform(shift);
    painter.add(Shape::Text(over));
}

/// A line of lettering over the world, as a label. `shift` moves and scales the drawing about
/// the text's centre while the space it takes stays put, so a moving title never reflows what is
/// around it.
pub fn label(
    ui: &mut Ui,
    text: &str,
    font: FontId,
    color: Color32,
    under: &[TextShadow],
    shift: TSTransform,
) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if ui.is_rect_visible(rect) {
        let centre = rect.center().to_vec2();
        let around =
            TSTransform::from_translation(centre) * shift * TSTransform::from_translation(-centre);
        lettering(ui.painter(), rect.min, galley, under, around);
    }
    response
}

pub fn frame() -> Frame {
    Frame::new()
        .fill(FILL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(8))
        .shadow(SHADOW)
}

const fn glow(right: f32, down: f32) -> TextShadow {
    TextShadow {
        offset: Vec2::new(right, down),
        color: Color32::from_black_alpha(70),
    }
}

const fn edge(right: f32, down: f32) -> TextShadow {
    TextShadow {
        offset: Vec2::new(right, down),
        color: Color32::BLACK,
    }
}
