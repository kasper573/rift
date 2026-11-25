//! The dark ground of notices: what floats over the world to tell the player something stays apart
//! from the paper of windows and cards.

use std::sync::Arc;

use egui::emath::TSTransform;
use egui::epaint::{Shadow, TextShape};
use egui::{
    Color32, CornerRadius, FontId, Frame, Galley, Margin, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::glyph::Mark;
use crate::ink::{Ground, Prose};
use crate::paint::{self, TextShadow};

pub const FILL: Color32 = Color32::from_rgb(38, 38, 43);
pub const BORDER: Color32 = Color32::from_rgb(50, 50, 57);
pub const HOVER: Color32 = Color32::from_rgb(48, 48, 54);
pub const TEXT: Color32 = Color32::from_rgb(228, 228, 232);
pub const DANGER: Color32 = Prose::Danger.on(Ground::Dusk);
/// A character's name floating over the world.
pub const CHARACTER: Color32 = Prose::Place.on(Ground::Dusk);
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

/// Under large text set straight over the world, whatever the scene behind it: a crisp black
/// outline round every letter and a shadow below, so the letter keeps its own colour on the
/// brightest ground.
pub const OUTLINED: &[TextShadow] = &[
    TextShadow {
        offset: Vec2::new(0.0, 2.5),
        color: Color32::from_black_alpha(160),
    },
    edge(1.6, 0.0),
    edge(1.39, 0.8),
    edge(0.8, 1.39),
    edge(0.0, 1.6),
    edge(-0.8, 1.39),
    edge(-1.39, 0.8),
    edge(-1.6, 0.0),
    edge(-1.39, -0.8),
    edge(-0.8, -1.39),
    edge(0.0, -1.6),
    edge(0.8, -1.39),
    edge(1.39, -0.8),
];

/// How tall the letters `OUTLINED` is set for stand: it reaches further round larger letters, and
/// less far round smaller ones.
pub const OUTLINED_FOR: f32 = 28.0;

/// How far `OUTLINED` reaches round letters `size` tall, as a share of how far it is set to.
pub fn outline_reach(size: f32) -> f32 {
    size / OUTLINED_FOR
}

/// `under`, reaching `reach` of how far it is set to.
pub fn reaching(under: &[TextShadow], reach: f32) -> Vec<TextShadow> {
    under
        .iter()
        .map(|shadow| TextShadow {
            offset: shadow.offset * reach,
            color: shadow.color,
        })
        .collect()
}

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

/// `mark` drawn over the world in `rect`, with a copy of itself as each of `under` lays it beneath,
/// as `lettering` sets a letter.
pub fn mark(painter: &Painter, mark: Mark, rect: Rect, color: Color32, under: &[TextShadow]) {
    for shadow in under {
        mark.paint(painter, rect.translate(shadow.offset), shadow.color);
    }
    mark.paint(painter, rect, color);
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

/// What a tag of a trade says of an item: that the player gives it and holds enough of it, gives
/// it and holds too little, or gets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tally {
    Gives,
    Lacks,
    Gets,
}

impl Tally {
    /// The sign the tag's count is led by.
    pub fn mark(self) -> Mark {
        match self {
            Tally::Gives | Tally::Lacks => Mark::Minus,
            Tally::Gets => Mark::Plus,
        }
    }

    /// The colour of the tag's sign and words.
    pub fn ink(self) -> Color32 {
        match self {
            Tally::Gives => Color32::from_rgb(236, 228, 212),
            Tally::Lacks => Color32::from_rgb(255, 172, 160),
            Tally::Gets => Color32::from_rgb(144, 214, 122),
        }
    }

    fn fill(self) -> Color32 {
        match self {
            Tally::Gives => Color32::from_rgba_unmultiplied(44, 32, 28, 215),
            Tally::Lacks => Color32::from_rgba_unmultiplied(92, 24, 20, 225),
            Tally::Gets => Color32::from_rgba_unmultiplied(30, 64, 34, 225),
        }
    }

    fn edge(self) -> Color32 {
        match self {
            Tally::Gives => Color32::from_rgb(150, 120, 104),
            Tally::Lacks => DANGER,
            Tally::Gets => Color32::from_rgb(120, 196, 104),
        }
    }
}

/// A chip of a trade's tally over the world, round what `add` lays in it, for text `size` tall:
/// the tally's fill within one even edge, as tall as the text, so a row holding it grows no
/// taller.
pub fn tally<R>(
    ui: &mut Ui,
    tally: Tally,
    size: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let across = (size * TALLY_PADDING).round() as i8;
    Frame::new()
        .fill(tally.fill())
        .stroke(Stroke::new(1.0, tally.edge()))
        .corner_radius(CornerRadius::same((size * TALLY_CORNER).round() as u8))
        .inner_margin(Margin::symmetric(across, 0))
        .show(ui, add)
}

/// How far a tally's words keep from its edges along the row, and how round its corners are, in
/// the height of its text.
const TALLY_PADDING: f32 = 0.35;
const TALLY_CORNER: f32 = 0.3;

/// A moment worth marking, set straight over the world: what `add` lays out (giving back the
/// rect it drew in), centred on a dark band that fades out to both sides between two thin rules
/// of `rule`. Gives back the band's rect.
pub fn banner(ui: &mut Ui, rule: Color32, add: impl FnOnce(&mut Ui) -> Rect) -> Rect {
    let under = ui.painter().add(Shape::Noop);
    ui.add_space(BANNER_PADDING);
    let words = add(ui);
    ui.add_space(BANNER_PADDING);
    let reach = words.width() * 0.5 + BANNER_REACH;
    let band = Rect::from_x_y_ranges(
        words.center().x - reach..=words.center().x + reach,
        words.top() - BANNER_PADDING..=words.bottom() + BANNER_PADDING,
    );
    let ruled = band.shrink2(Vec2::new(band.width() * (1.0 - BANNER_RULED) * 0.5, 0.0));
    let rule_at = |y: f32| Rect::from_x_y_ranges(ruled.x_range(), y - 0.5..=y + 0.5);
    let ends = |along: f32| (along.min(1.0 - along) / BANNER_RULE_FADE).clamp(0.0, 1.0);
    ui.painter().set(
        under,
        Shape::Vec(vec![
            paint::band(band, |along| {
                let middle = 1.0 - (2.0 * along - 1.0).abs();
                BANNER_GROUND.gamma_multiply(middle.powf(BANNER_FALLOFF))
            }),
            paint::band(rule_at(band.top() + BANNER_RULE_INSET), |along| {
                rule.gamma_multiply(ends(along))
            }),
            paint::band(rule_at(band.bottom() - BANNER_RULE_INSET), |along| {
                rule.gamma_multiply(ends(along))
            }),
        ]),
    );
    band
}

const BANNER_GROUND: Color32 = Color32::from_rgba_premultiplied(9, 7, 5, 170);
/// How sharply the ground fades out from the middle to its ends: under 1, it stays dark across
/// most of its width.
const BANNER_FALLOFF: f32 = 0.7;
/// How far the band reaches past the words on each side, and how far it keeps from them above
/// and below.
const BANNER_REACH: f32 = 220.0;
const BANNER_PADDING: f32 = 8.0;
/// How much of the band's width the rules span, how much of their length fades out at each end,
/// and how far in from the band's edges they run.
const BANNER_RULED: f32 = 0.8;
const BANNER_RULE_FADE: f32 = 0.25;
const BANNER_RULE_INSET: f32 = 2.0;

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
