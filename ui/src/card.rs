use std::f32::consts::{FRAC_PI_2, PI};

use egui::epaint::{Mesh, Shadow};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextureId, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::glyph::{self, CardKind, Mark};
use crate::sheet::{self, Depth};
use crate::{controls, ink, paint};

/// How wide and tall every card is, wherever it is shown and whatever it shows: a crest and a
/// ribbon may reach past it, and text that overruns it scrolls.
pub const WIDTH: f32 = 300.0;
pub const HEIGHT: f32 = 420.0;
pub const MARGIN: f32 = 6.0;
pub const CORNER: f32 = 10.0;
const FRAME_STROKE: f32 = 2.5;
/// How far in from a card's edge its border may reach before it covers what the card holds.
pub const BORDER_ROOM: f32 = MARGIN + FRAME_STROKE;
/// The largest corner cut or notched that stays clear of what the card holds.
pub const CORNER_ROOM: f32 = MARGIN;
/// The largest radius of a round corner that stays clear of what the card holds.
pub const ROUND_ROOM: f32 = 20.0;
const FRAME_RADIUS: f32 = 6.0;
const GLYPH: f32 = 18.0;
pub const KIND_BAND: f32 = 26.0;
pub const NAME_BAND: f32 = 34.0;
/// How tall the picture's part of every card is.
pub const ART: f32 = 128.0;
const PICTURE: f32 = 96.0;
const PLATE_PADDING: Margin = Margin::symmetric(12, 10);
pub const LINE_GAP: f32 = 6.0;
pub const GRANT_ICON: f32 = 22.0;
const CROSS_AT: Vec2 = Vec2::new(8.0, 7.0);
const BAND_TEXT_INSET: f32 = 28.0;
const ACTION: f32 = 18.0;
const ACTION_GAP: f32 = 4.0;
/// How much of its ink a card's action shows while the pointer is elsewhere.
const ACTION_AT_REST: f32 = 0.55;
const PANE_GAP: f32 = 10.0;
const STRIP: f32 = 18.0;
/// How far in from the frame's lower right corner a seal is stamped, clear of the kind's mark
/// there.
const SEAL_IN: Vec2 = Vec2::new(80.0, 26.0);
const BANNER: f32 = 16.0;
const CREST_SUNK: f32 = 4.0;
const BAND_ICON: f32 = 16.0;
const BAND_ICON_GAP: f32 = 6.0;
const INSET_STROKE: f32 = 1.5;
/// How far the picture's ground reaches under the rules and the stroke that border it.
const ART_BLEED: f32 = 1.0;
const ARC_STEPS: usize = 6;
const WATERMARK: f32 = 96.0;
const RULE_ICON: f32 = 10.0;
const RULE_ROW: f32 = 10.0;
/// How many times over a tilt closes all but a third of what is left within its settling time:
/// after it, under a fiftieth remains.
const SETTLING: f32 = 4.0;
/// How near its goal, in degrees, a tilt is there: closing in for ever, a card would never lie
/// still, and what shows it would never be done drawing it.
const AT_REST: f32 = 0.01;
const RULE_FADE: f32 = 0.25;

pub const SHADOW: Shadow = Shadow {
    offset: [0, 10],
    blur: 14,
    spread: 0,
    color: Color32::from_black_alpha(140),
};
const BEHIND_SHADOW: Shadow = Shadow {
    offset: [0, 5],
    blur: 8,
    spread: 0,
    color: Color32::from_black_alpha(90),
};
const CARD_CONTACT_SHADOW: Shadow = Shadow {
    offset: [0, 1],
    blur: 2,
    spread: 0,
    color: Color32::from_black_alpha(80),
};

/// How a card is dressed: the colour behind its picture, the two colours, the icon and the label
/// its looks paint with, and the look each of its parts takes. The picture, the words in its
/// bands and its text are the card's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardDress<'a> {
    pub ground: [Color32; 2],
    pub gradient: [Color32; 2],
    pub icon: Option<TextureId>,
    pub label: Option<&'a str>,
    pub border: BorderLook,
    pub corners: CornerLook,
    pub crest: CrestLook,
    pub name_band: NameBandLook,
    pub label_line: LabelLineLook,
    pub art: ArtLook,
    pub paper: PaperLook,
    pub dividers: DividerLook,
    pub halo: HaloLook,
}

impl CardDress<'static> {
    /// A card that picks no look, on a ground of plain ink.
    pub const PLAIN: CardDress<'static> = CardDress {
        ground: [ink::INK3, ink::INK2],
        gradient: [ink::INK3, ink::INK2],
        icon: None,
        label: None,
        border: BorderLook::Ink,
        corners: CornerLook::Round { radius: CORNER },
        crest: CrestLook::None,
        name_band: NameBandLook::Plain { icon: false },
        label_line: LabelLineLook::None,
        art: ArtLook::Flush,
        paper: PaperLook::Cream,
        dividers: DividerLook::Line,
        halo: HaloLook::Shadow,
    };
}

/// The stroke round a card: ink, a stroke in the dress's gradient along its edge, that stroke with
/// one of ink a gap inside it, or an image tiled along the edge and tinted by the gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BorderLook {
    Ink,
    Gradient { width: f32 },
    Double { width: f32, gap: f32 },
    Patterned { image: TextureId, width: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CornerLook {
    Round { radius: f32 },
    Cut { size: f32 },
    Notched { size: f32 },
}

/// What rises over a card's top edge: nothing, a tab holding the dress's icon, or a banner
/// carrying its label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CrestLook {
    None,
    Icon { size: f32 },
    Banner,
}

/// How the band under the picture is filled, and whether the dress's icon stands before the
/// name: plain, washed in the gradient at both ends, filled with it under a light name, or a
/// ribbon reaching past the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NameBandLook {
    Plain { icon: bool },
    Washed { strength: f32, icon: bool },
    Filled { icon: bool },
    Ribbon { overhang: f32, icon: bool },
}

/// Where a card writes the dress's label: nowhere, on a strip under the name band, or as a seal
/// stamped on its paper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelLineLook {
    None,
    Strip,
    Seal,
}

/// How the picture is framed within its part of the card: flush with the frame, or set in with
/// a margin and a border of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArtLook {
    Flush,
    Inset { margin: f32 },
}

/// What lies under a card's text: cream, cream tinted with the gradient, or cream watermarked
/// with the dress's icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaperLook {
    Cream,
    Tinted { strength: f32 },
    Watermarked { strength: f32 },
}

/// The rule beside the heading of each section of a card's text: a plain line, a line in the
/// gradient that fades at both ends, or a line broken by the dress's icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DividerLook {
    Line,
    Gradient,
    Ornament,
}

/// What surrounds a card as it lies on the screen: its shadow, or a glow in the gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HaloLook {
    Shadow,
    Glow { spread: f32 },
}

pub struct Card<'a> {
    pub kind: CardKind,
    pub band: &'a str,
    /// What the band counts at its far end: a rank, or how many are held.
    pub tally: Option<&'a str>,
    pub name: &'a str,
    pub picture: Option<TextureId>,
    /// The cross that closes the card, by the name a screen reader says, where it has one.
    pub close: Option<&'a str>,
    pub action: Option<CardAction<'a>>,
    pub depth: Depth,
    pub dress: CardDress<'a>,
}

/// Something a reader can do with what a card shows that the card does not ask for: a small
/// button at the end of its band, quiet until the pointer is on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardAction<'a> {
    pub mark: Mark,
    pub label: &'a str,
}

/// How far a card is turned toward the pointer, in degrees: `aside` about its upright axis and
/// `above` about its level one, so the part under the pointer sinks back. Eased from frame to
/// frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CardTilt {
    pub aside: f32,
    pub above: f32,
}

impl CardTilt {
    pub const FLAT: CardTilt = CardTilt {
        aside: 0.0,
        above: 0.0,
    };

    /// A step of `dt` seconds toward facing `pointer`, or toward lying flat when it is not on
    /// the card. The card faces the pointer flat at the middle of `flat`, where it lies when it
    /// is not turned, and turned by `most` at its edge, and is as good as there after
    /// `settles_in` seconds.
    pub fn toward(
        self,
        pointer: Option<Pos2>,
        flat: Rect,
        most: f32,
        settles_in: f32,
        dt: f32,
    ) -> CardTilt {
        let goal = pointer
            .filter(|pointer| flat.contains(*pointer) && flat.is_positive())
            .map_or(CardTilt::FLAT, |pointer| {
                let off = (pointer - flat.center()) / (flat.size() / 2.0);
                CardTilt {
                    aside: off.x * most,
                    above: off.y * most,
                }
            });
        let covered = if settles_in > 0.0 {
            1.0 - (-SETTLING * dt / settles_in).exp()
        } else {
            1.0
        };
        let turned = CardTilt {
            aside: self.aside + (goal.aside - self.aside) * covered,
            above: self.above + (goal.above - self.above) * covered,
        };
        let there = |turned: f32, goal: f32| (goal - turned).abs() < AT_REST;
        if there(turned.aside, goal.aside) && there(turned.above, goal.above) {
            goal
        } else {
            turned
        }
    }
}

/// What a card hands the text written on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    /// The name overran its band and was cut: the plate's first line writes it in full.
    pub name_cut: bool,
    /// How the rule beside each section's heading is drawn, and what with.
    pub rule: SectionRule,
}

/// The rule beside a section's heading on a card, in the card's dress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionRule {
    pub look: DividerLook,
    pub gradient: [Color32; 2],
    pub icon: Option<TextureId>,
}

pub struct CardResponse {
    pub rect: Rect,
    pub frame: Rect,
    pub band: Response,
    pub close: Option<Response>,
    pub action: Option<Response>,
}

impl<'a> Card<'a> {
    pub fn new(kind: CardKind, band: &'a str, name: &'a str) -> Card<'a> {
        Card {
            kind,
            band,
            tally: None,
            name,
            picture: None,
            close: None,
            action: None,
            depth: Depth::Front,
            dress: CardDress::PLAIN,
        }
    }

    pub fn show(self, ui: &mut Ui, plate: impl FnOnce(&mut Ui, Plate)) -> CardResponse {
        let crest = self.crest_size(ui.painter());
        let risen = crest.map_or(0.0, |crest| crest.y - CREST_SUNK);
        let outer_min = ui.cursor().min + Vec2::new(0.0, risen);
        let frame_width = WIDTH - 2.0 * MARGIN;
        let under = ui.painter().add(Shape::Noop);
        let frame_min = outer_min + Vec2::splat(MARGIN);
        let frame = Rect::from_min_size(frame_min, Vec2::new(frame_width, HEIGHT - 2.0 * MARGIN));
        let mut inner = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    frame_min,
                    Vec2::new(frame_width, f32::INFINITY),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        inner.set_clip_rect(frame.intersect(ui.clip_rect()));
        inner.spacing_mut().item_spacing = Vec2::ZERO;
        let (band_rect, band) =
            inner.allocate_exact_size(Vec2::new(frame_width, KIND_BAND), Sense::click_and_drag());
        band.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, self.band));
        let (art_rect, _) = inner.allocate_exact_size(Vec2::new(frame_width, ART), Sense::hover());
        let (name_rect, _) =
            inner.allocate_exact_size(Vec2::new(frame_width, NAME_BAND), Sense::hover());
        inner.put(
            name_rect,
            egui::Label::new(
                egui::RichText::new(self.name)
                    .size(0.1)
                    .color(Color32::TRANSPARENT),
            )
            .selectable(false),
        );
        let strip = (self.dress.label_line == LabelLineLook::Strip).then(|| {
            inner
                .allocate_exact_size(Vec2::new(frame_width, STRIP), Sense::hover())
                .0
        });
        let painter = inner.painter().clone();
        self.paint_art(&painter, art_rect);
        self.paint_kind_band(&painter, band_rect);
        if let NameBandLook::Ribbon { overhang, .. } = self.dress.name_band {
            self.paint_ribbon(ui.painter(), name_rect, overhang);
        }
        let name_cut = self.paint_name_band(&painter, name_rect);
        if let Some(strip) = strip {
            self.paint_strip(&painter, strip);
        }
        let close = self.close.map(|said| {
            let rect = Rect::from_min_size(
                Pos2::new(
                    outer_min.x + WIDTH - CROSS_AT.x - sheet::CROSS,
                    outer_min.y + CROSS_AT.y,
                ),
                Vec2::splat(sheet::CROSS),
            );
            let id = inner.id();
            crate::panel::close_button(&mut inner, id, rect, said)
        });
        let action = self.action.map(|action| {
            let right = band_rect.right() - self.cross_room() - ACTION_GAP;
            let rect = Rect::from_center_size(
                Pos2::new(right - ACTION / 2.0, band_rect.center().y),
                Vec2::splat(ACTION),
            );
            let response = inner.interact(rect, inner.id().with("action"), Sense::click());
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, inner.is_enabled(), action.label)
            });
            let ink = if response.hovered() {
                painter.rect_filled(rect, 3.0, ink::ink_wash(0.1));
                ink::INK
            } else {
                ink::ink_wash(ACTION_AT_REST)
            };
            action.mark.paint(&painter, rect.shrink(2.0), ink);
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        });
        let written = Rect::from_min_max(Pos2::new(frame.min.x, inner.cursor().min.y), frame.max);
        painter.extend(self.paper(written));
        let rule = SectionRule {
            look: self.dress.dividers,
            gradient: self.dress.gradient,
            icon: self.dress.icon,
        };
        let write = |ui: &mut Ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, LINE_GAP);
            egui::Frame::new()
                .inner_margin(PLATE_PADDING)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    plate(ui, Plate { name_cut, rule });
                });
        };
        sheet::bars(&mut inner);
        sheet::scroll(
            &mut inner,
            egui::ScrollArea::vertical()
                .max_height(written.height())
                .auto_shrink([false, false])
                .id_salt(("card plate", self.name)),
            write,
        );
        let outer = frame.expand(MARGIN);
        let taken = Rect::from_min_max(outer.min - Vec2::new(0.0, risen), outer.max);
        ui.allocate_rect(taken, Sense::hover());
        ui.painter().set(
            under,
            Shape::Vec(self.board(ui.painter(), outer, frame, crest)),
        );
        self.paint_over(ui.painter(), outer, frame);
        CardResponse {
            rect: taken,
            frame,
            band,
            close,
            action,
        }
    }

    /// How much of the far end of the band the cross that closes the card takes.
    fn cross_room(&self) -> f32 {
        if self.close.is_some() {
            sheet::CROSS + 2.0
        } else {
            0.0
        }
    }

    /// How much of the far end of the band its buttons take.
    fn buttons_room(&self) -> f32 {
        let action = self.action.map_or(0.0, |_| ACTION + ACTION_GAP);
        self.cross_room() + action
    }

    /// How wide and tall the crest over the top edge is, when the card has one.
    fn crest_size(&self, painter: &Painter) -> Option<Vec2> {
        match self.dress.crest {
            CrestLook::None => None,
            CrestLook::Icon { size } => Some(Vec2::new(size + 16.0, size + 6.0 + CREST_SUNK)),
            CrestLook::Banner => {
                let label = self.dress.label.unwrap_or_default();
                let galley = painter.layout_job(TextRole::Section.job(label, ink::CREAM));
                Some(Vec2::new(galley.size().x + 20.0, BANNER + CREST_SUNK))
            }
        }
    }

    fn board(
        &self,
        painter: &Painter,
        outer: Rect,
        frame: Rect,
        crest: Option<Vec2>,
    ) -> Vec<Shape> {
        let [from, to] = self.dress.gradient;
        let radius = match self.dress.corners {
            CornerLook::Round { radius } => radius,
            CornerLook::Cut { .. } | CornerLook::Notched { .. } => 0.0,
        };
        let mut shapes = match self.dress.halo {
            HaloLook::Shadow => {
                let shadow = match self.depth {
                    Depth::Front => SHADOW,
                    Depth::Behind => BEHIND_SHADOW,
                };
                vec![
                    Shape::Rect(shadow.as_shape(outer, radius)),
                    Shape::Rect(CARD_CONTACT_SHADOW.as_shape(outer, radius)),
                ]
            }
            HaloLook::Glow { spread } => {
                let glow = Shadow {
                    offset: [0, 0],
                    blur: spread.clamp(0.0, 255.0) as u8,
                    spread: (spread / 4.0).clamp(0.0, 255.0) as u8,
                    color: ink::wash(from, 0.7),
                };
                vec![Shape::Rect(glow.as_shape(outer, radius))]
            }
        };
        if let Some(crest) = crest {
            let tab = Rect::from_center_size(
                Pos2::new(outer.center().x, outer.top() + CREST_SUNK - crest.y / 2.0),
                crest,
            );
            let top = CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            };
            shapes.push(Shape::rect_filled(tab, top, to));
            let middle = Pos2::new(tab.center().x, tab.top() + (crest.y - CREST_SUNK) / 2.0);
            match (self.dress.crest, self.dress.icon, self.dress.label) {
                (CrestLook::Icon { size }, Some(icon), _) => shapes.push(Shape::image(
                    icon,
                    Rect::from_center_size(middle, Vec2::splat(size)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                )),
                (CrestLook::Banner, _, Some(label)) => {
                    let galley = painter.layout_job(TextRole::Section.job(label, ink::CREAM));
                    shapes.push(Shape::galley(
                        middle - galley.size() / 2.0,
                        galley,
                        ink::CREAM,
                    ));
                }
                _ => {}
            }
        }
        match self.dress.corners {
            CornerLook::Round { radius } => {
                shapes.push(Shape::rect_filled(outer, radius, ink::CREAM));
            }
            corners => shapes.push(filled(&outline(outer, corners), ink::CREAM)),
        }
        match (self.dress.border, self.dress.corners) {
            (BorderLook::Ink, CornerLook::Round { .. }) => shapes.extend([
                Shape::rect_stroke(
                    frame.shrink(FRAME_STROKE),
                    FRAME_RADIUS - FRAME_STROKE,
                    Stroke::new(1.0, ink::LIGHT),
                    StrokeKind::Outside,
                ),
                Shape::rect_stroke(
                    frame,
                    FRAME_RADIUS,
                    Stroke::new(FRAME_STROKE, ink::FRAME),
                    StrokeKind::Inside,
                ),
            ]),
            (BorderLook::Ink, corners) => {
                shapes.push(ring(outer, corners, FRAME_STROKE, [ink::FRAME; 2]));
            }
            (BorderLook::Gradient { width }, corners) => {
                shapes.push(ring(outer, corners, width, [from, to]));
            }
            (BorderLook::Double { width, gap }, corners) => shapes.extend([
                ring(outer, corners, width, [from, to]),
                ring(
                    outer.shrink(width + gap),
                    within(corners, width + gap),
                    width,
                    [ink::FRAME; 2],
                ),
            ]),
            (BorderLook::Patterned { image, width }, _) => {
                shapes.push(tiled(outer, image, width, [from, to]));
            }
        }
        shapes
    }

    fn paper(&self, written: Rect) -> Vec<Shape> {
        let [from, _] = self.dress.gradient;
        match (self.dress.paper, self.dress.icon) {
            (PaperLook::Tinted { strength }, _) => {
                vec![Shape::rect_filled(written, 0.0, ink::wash(from, strength))]
            }
            (PaperLook::Watermarked { strength }, Some(icon)) => {
                let side = WATERMARK.min(written.height()).min(written.width());
                vec![Shape::image(
                    icon,
                    Rect::from_min_size(
                        written.max - Vec2::splat(side) - Vec2::splat(6.0),
                        Vec2::splat(side),
                    ),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE.gamma_multiply(strength),
                )]
            }
            (PaperLook::Cream | PaperLook::Watermarked { .. }, _) => Vec::new(),
        }
    }

    fn paint_kind_band(&self, painter: &Painter, band: Rect) {
        painter.rect_filled(band, CornerRadius::ZERO, ink::white_wash(0.22));
        painter.line_segment(
            [
                Pos2::new(band.min.x, band.max.y - 1.0),
                Pos2::new(band.max.x, band.max.y - 1.0),
            ],
            Stroke::new(2.0, ink::INK),
        );
        sheet::text(
            painter,
            Pos2::new(band.min.x + BAND_TEXT_INSET, band.center().y),
            Align2::LEFT_CENTER,
            TextRole::Band,
            self.band,
            ink::INK,
        );
        if let Some(tally) = self.tally {
            let buttons = self.buttons_room();
            let right = band.max.x - buttons - if buttons > 0.0 { 8.0 } else { 10.0 };
            sheet::text(
                painter,
                Pos2::new(right, band.center().y),
                Align2::RIGHT_CENTER,
                TextRole::Band,
                tally,
                ink::INK,
            );
        }
    }

    fn paint_art(&self, painter: &Painter, art: Rect) {
        let [top, bottom] = self.dress.ground;
        let picture = match self.dress.art {
            ArtLook::Flush => art,
            ArtLook::Inset { margin } => art.shrink(margin),
        };
        // What borders the picture is painted over its edge. A card turned on the stage would
        // show its paper between a ground that ends where a soft-edged rule begins.
        let ground = picture.expand(ART_BLEED);
        painter.add(shaded(
            &[
                ground.left_top(),
                ground.right_top(),
                ground.right_bottom(),
                ground.left_bottom(),
            ],
            picture,
            [top, bottom],
        ));
        if let ArtLook::Inset { .. } = self.dress.art {
            painter.rect_stroke(
                picture,
                0.0,
                Stroke::new(INSET_STROKE, self.dress.gradient[1]),
                StrokeKind::Outside,
            );
        }
        match self.picture {
            Some(icon) => {
                let size = Vec2::splat(PICTURE.min(picture.height()));
                let rect = Rect::from_center_size(picture.center(), size);
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                for offset in [
                    Vec2::new(-1.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.0, -1.0),
                    Vec2::new(0.0, 1.0),
                ] {
                    painter.add(Shape::image(icon, rect.translate(offset), uv, ink::CREAM));
                }
                painter.add(Shape::image(icon, rect, uv, Color32::WHITE));
            }
            None => {
                let rect = Rect::from_center_size(
                    picture.center(),
                    Vec2::splat(72.0_f32.min(picture.height() - 8.0)),
                );
                self.kind
                    .paint(painter, rect, ink::wash(ink::CREAM, 0.85), false);
            }
        }
    }

    /// The ribbon a name band reaches past the frame as, with the fold that tucks each end
    /// back under the card.
    fn paint_ribbon(&self, painter: &Painter, band: Rect, overhang: f32) {
        let [from, to] = self.dress.gradient;
        let reach = MARGIN + overhang;
        let ribbon = band.expand2(Vec2::new(reach, 0.0));
        for (tip, inward) in [(ribbon.left_bottom(), 1.0), (ribbon.right_bottom(), -1.0)] {
            painter.add(Shape::convex_polygon(
                vec![
                    tip,
                    tip + Vec2::new(inward * overhang, 0.0),
                    tip + Vec2::new(inward * overhang, overhang.min(6.0)),
                ],
                ink::on(to, Color32::BLACK, 0.45),
                Stroke::NONE,
            ));
        }
        painter.rect_filled(ribbon, 0.0, ink::CREAM);
        paint::band(painter, ribbon, |along| {
            ink::wash(from.lerp_to_gamma(to, along), 0.45)
        });
        painter.rect_stroke(
            ribbon,
            0.0,
            Stroke::new(1.0, ink::on(to, Color32::BLACK, 0.3)),
            StrokeKind::Inside,
        );
    }

    fn paint_name_band(&self, painter: &Painter, band: Rect) -> bool {
        let [from, to] = self.dress.gradient;
        let (ink, with_icon, ruled) = match self.dress.name_band {
            NameBandLook::Plain { icon } => {
                painter.rect_filled(band, CornerRadius::ZERO, ink::white_wash(0.25));
                (ink::INK, icon, true)
            }
            NameBandLook::Washed { strength, icon } => {
                painter.rect_filled(band, CornerRadius::ZERO, ink::white_wash(0.25));
                paint::band(painter, band, |along| {
                    let ends = (2.0 * along - 1.0).abs();
                    ink::wash(from.lerp_to_gamma(to, along), strength * ends * ends)
                });
                (ink::INK, icon, true)
            }
            NameBandLook::Filled { icon } => {
                paint::band(painter, band, |along| from.lerp_to_gamma(to, along));
                (ink::CREAM, icon, true)
            }
            NameBandLook::Ribbon { icon, .. } => (ink::INK, icon, false),
        };
        if ruled {
            for y in [band.min.y + 1.0, band.max.y - 1.0] {
                painter.line_segment(
                    [Pos2::new(band.min.x, y), Pos2::new(band.max.x, y)],
                    Stroke::new(2.0, ink::INK),
                );
            }
        }
        let icon = self.dress.icon.filter(|_| with_icon);
        let lead = if icon.is_some() {
            BAND_ICON + BAND_ICON_GAP
        } else {
            0.0
        };
        let room = band.width() - 16.0 - lead;
        let mut job = TextRole::Name.job(self.name, ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let galley = painter.layout_job(job);
        let cut = galley.elided;
        let left = band.center().x - (lead + galley.size().x) / 2.0;
        if let Some(icon) = icon {
            painter.image(
                icon,
                Rect::from_center_size(
                    Pos2::new(left + BAND_ICON / 2.0, band.center().y),
                    Vec2::splat(BAND_ICON),
                ),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.galley(
            Pos2::new(left + lead, band.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
        cut
    }

    /// The strip under the name band that carries the dress's label.
    fn paint_strip(&self, painter: &Painter, strip: Rect) {
        let [from, to] = self.dress.gradient;
        paint::band(painter, strip, |along| {
            ink::wash(from.lerp_to_gamma(to, along), 0.35)
        });
        painter.line_segment(
            [strip.left_bottom(), strip.right_bottom()],
            Stroke::new(1.0, ink::ink_wash(0.5)),
        );
        sheet::text(
            painter,
            strip.center(),
            Align2::CENTER_CENTER,
            TextRole::Section,
            self.dress.label.unwrap_or_default(),
            ink::INK,
        );
    }

    fn paint_over(&self, painter: &Painter, outer: Rect, frame: Rect) {
        let top = Rect::from_min_size(outer.min + Vec2::new(9.0, 10.0), Vec2::splat(GLYPH));
        let bottom = Rect::from_min_size(
            outer.max - Vec2::new(9.0, 10.0) - Vec2::splat(GLYPH),
            Vec2::splat(GLYPH),
        );
        self.kind.paint(painter, top, ink::INK, false);
        self.kind.paint(painter, bottom, ink::INK, true);
        if let (LabelLineLook::Seal, Some(label)) = (self.dress.label_line, self.dress.label) {
            controls::stamp(painter, frame.max - SEAL_IN, label, self.dress.gradient[1]);
        }
    }
}

/// The corners of an edge drawn `inset` inside one whose corners are `corners`, so the two run
/// side by side.
fn within(corners: CornerLook, inset: f32) -> CornerLook {
    match corners {
        CornerLook::Round { radius } => CornerLook::Round {
            radius: (radius - inset).max(0.0),
        },
        cut => cut,
    }
}

/// The edge of `rect` with its corners as `corners` shapes them, clockwise from the top left.
/// Every look gives as many points, so two edges of one look pair up point for point.
fn outline(rect: Rect, corners: CornerLook) -> Vec<Pos2> {
    let turns = [
        (rect.left_top(), Vec2::new(1.0, 1.0), PI),
        (rect.right_top(), Vec2::new(-1.0, 1.0), 1.5 * PI),
        (rect.right_bottom(), Vec2::new(-1.0, -1.0), 0.0),
        (rect.left_bottom(), Vec2::new(1.0, -1.0), FRAC_PI_2),
    ];
    let most = rect.width().min(rect.height()) / 2.0;
    turns
        .into_iter()
        .flat_map(|(corner, inward, start)| -> Vec<Pos2> {
            match corners {
                CornerLook::Round { radius } => {
                    let radius = radius.clamp(0.0, most);
                    let center = corner + inward * radius;
                    (0..=ARC_STEPS)
                        .map(|step| {
                            let angle = start + FRAC_PI_2 * step as f32 / ARC_STEPS as f32;
                            center + Vec2::angled(angle) * radius
                        })
                        .collect()
                }
                CornerLook::Cut { size } | CornerLook::Notched { size } => {
                    let size = size.clamp(0.0, most);
                    let along = Vec2::new(inward.x * size, 0.0);
                    let down = Vec2::new(0.0, inward.y * size);
                    let (first, last) = if inward.x * inward.y > 0.0 {
                        (corner + down, corner + along)
                    } else {
                        (corner + along, corner + down)
                    };
                    match corners {
                        CornerLook::Notched { .. } => vec![first, corner + along + down, last],
                        _ => vec![first, last],
                    }
                }
            }
        })
        .collect()
}

/// A shape filled in one colour, its edge the `points` round its middle.
fn filled(points: &[Pos2], color: Color32) -> Shape {
    let bounds = Rect::from_points(points);
    shaded(points, bounds, [color, color])
}

/// A shape shaded from the first colour at the top of `within` to the second at its bottom,
/// its edge the `points` round its middle.
fn shaded(points: &[Pos2], within: Rect, [top, bottom]: [Color32; 2]) -> Shape {
    let at = |point: Pos2| {
        let down = ((point.y - within.top()) / within.height().max(f32::EPSILON)).clamp(0.0, 1.0);
        top.lerp_to_gamma(bottom, down)
    };
    let middle = Rect::from_points(points).center();
    let mut mesh = Mesh::default();
    mesh.colored_vertex(middle, at(middle));
    for point in points {
        mesh.colored_vertex(*point, at(*point));
    }
    let count = points.len() as u32;
    for index in 0..count {
        mesh.add_triangle(0, 1 + index, 1 + (index + 1) % count);
    }
    Shape::mesh(mesh)
}

/// A stroke `width` wide inside the edge of `rect` as `corners` shape it, shaded from the first
/// colour at its top to the second at its bottom.
fn ring(rect: Rect, corners: CornerLook, width: f32, [top, bottom]: [Color32; 2]) -> Shape {
    let outer = outline(rect, corners);
    let inner = outline(rect.shrink(width), within(corners, width));
    let at = |point: Pos2| {
        let down = ((point.y - rect.top()) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
        top.lerp_to_gamma(bottom, down)
    };
    let mut mesh = Mesh::default();
    for (outside, inside) in outer.iter().zip(&inner) {
        mesh.colored_vertex(*outside, at(*outside));
        mesh.colored_vertex(*inside, at(*inside));
    }
    let count = outer.len() as u32;
    for index in 0..count {
        let (here, next) = (2 * index, 2 * ((index + 1) % count));
        mesh.add_triangle(here, next, here + 1);
        mesh.add_triangle(next, next + 1, here + 1);
    }
    Shape::mesh(mesh)
}

/// `image` tiled along the inside of the edge of `rect` in squares `width` across, tinted from
/// the first colour at the top to the second at the bottom.
fn tiled(rect: Rect, image: TextureId, width: f32, [top, bottom]: [Color32; 2]) -> Shape {
    let width = width.max(1.0);
    let mut mesh = Mesh::with_texture(image);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let mut tile = |min: Pos2| {
        let down = ((min.y - rect.top()) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
        let square = Rect::from_min_size(min, Vec2::splat(width)).intersect(rect);
        mesh.add_rect_with_uv(square, uv, top.lerp_to_gamma(bottom, down));
    };
    let across = (rect.width() / width).ceil() as u32;
    let down = (rect.height() / width).ceil() as u32;
    for step in 0..across {
        let x = rect.left() + step as f32 * width;
        tile(Pos2::new(x, rect.top()));
        tile(Pos2::new(x, rect.bottom() - width));
    }
    for step in 1..down.saturating_sub(1) {
        let y = rect.top() + step as f32 * width;
        tile(Pos2::new(rect.left(), y));
        tile(Pos2::new(rect.right() - width, y));
    }
    Shape::mesh(mesh)
}

/// Takes the same room whatever it holds and when it holds nothing, so nothing beside it moves as
/// the selection changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardPane {
    pub size: Vec2,
    pub foot: f32,
}

impl CardPane {
    pub fn show(
        self,
        ui: &mut Ui,
        card: impl FnOnce(&mut Ui),
        actions: impl FnOnce(&mut Ui),
    ) -> Response {
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::hover());
        let foot = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - self.foot), rect.max);
        let room = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, foot.min.y - PANE_GAP));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(room)
                .layout(Layout::top_down(Align::Min)),
            card,
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(foot)
                .layout(Layout::right_to_left(Align::Center)),
            actions,
        );
        response
    }
}

pub fn overview(ui: &mut Ui, text: &str) {
    sheet::label(ui, TextRole::CardBody, text, ink::INK2);
}

pub fn full_name(ui: &mut Ui, name: &str) {
    ui.add(
        egui::Label::new(crate::face::bold(name).size(TextRole::CardBody.size())).selectable(false),
    );
}

pub fn flavour(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(TextRole::Flavour.rich(text).color(ink::INK2)).wrap());
}

pub fn fact(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(TextRole::CardBody.rich(text).color(ink::INK)).wrap());
}

/// The heading of a section of a card's text, under the rule the card's dress draws between
/// its sections.
pub fn heading(ui: &mut Ui, plate: Plate, text: &str) {
    let (row, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), RULE_ROW), Sense::hover());
    let line = Rect::from_center_size(row.center(), Vec2::new(row.width(), 1.0));
    let [from, to] = plate.rule.gradient;
    let painter = ui.painter();
    match (plate.rule.look, plate.rule.icon) {
        (DividerLook::Line, _) | (DividerLook::Ornament, None) => {
            painter.rect_filled(line, 0.0, ink::ink_wash(0.3));
        }
        (DividerLook::Gradient, _) => {
            paint::band(painter, line.expand2(Vec2::new(0.0, 0.5)), |along| {
                let ends = (along.min(1.0 - along) / RULE_FADE).clamp(0.0, 1.0);
                ink::wash(from.lerp_to_gamma(to, along), ends)
            });
        }
        (DividerLook::Ornament, Some(icon)) => {
            let gap = RULE_ICON + 8.0;
            let middle = line.center();
            for part in [
                Rect::from_min_max(line.min, Pos2::new(middle.x - gap / 2.0, line.max.y)),
                Rect::from_min_max(Pos2::new(middle.x + gap / 2.0, line.min.y), line.max),
            ] {
                painter.rect_filled(part, 0.0, ink::ink_wash(0.4));
            }
            painter.image(
                icon,
                Rect::from_center_size(middle, Vec2::splat(RULE_ICON)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
    sheet::label(ui, TextRole::Section, text, ink::RUBRIC);
}

pub fn requirement(ui: &mut Ui, met: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (mark, color) = if met {
            (Mark::Check, ink::VERDIGRIS)
        } else {
            (Mark::Cross, ink::VERMILION)
        };
        glyph::show(ui, mark, 12.0, color);
        ui.add(egui::Label::new(TextRole::CardBody.rich(text).color(color)).wrap());
    });
}

pub fn foot(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(TextRole::Small.rich(text).color(ink::INK3)).wrap());
}
