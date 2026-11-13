use egui::epaint::{Mesh, Shadow};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Margin, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextureId, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::glyph::{self, CardKind, Mark};
use crate::sheet::{self, Depth};
use crate::{controls, ink};

pub const WIDTH: f32 = 300.0;
pub const DOCKED_WIDTH: f32 = 480.0;
pub const MAX_HEIGHT: f32 = 420.0;
pub const MARGIN: f32 = 6.0;
const CORNER: f32 = 10.0;
const FRAME_STROKE: f32 = 2.5;
const FRAME_RADIUS: f32 = 6.0;
const GLYPH: f32 = 18.0;
pub const KIND_BAND: f32 = 26.0;
pub const NAME_BAND: f32 = 34.0;
pub const ART: f32 = 118.0;
pub const ART_QUEST: f32 = 80.0;
pub const ART_SHORT: f32 = 70.0;
const PLATE_PADDING: Margin = Margin::symmetric(12, 10);
pub const LINE_GAP: f32 = 6.0;
pub const GRANT_ICON: f32 = 22.0;
const CROSS_AT: Vec2 = Vec2::new(8.0, 7.0);
const BAND_TEXT_INSET: f32 = 28.0;
const PANE_GAP: f32 = 10.0;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Art {
    pub icon: Option<TextureId>,
    pub height: f32,
}

impl Art {
    fn icon_size(self) -> f32 {
        if self.height >= ART { 96.0 } else { 60.0 }
    }
}

pub struct Card<'a> {
    pub kind: CardKind,
    pub band: &'a str,
    pub numeral: Option<&'a str>,
    pub name: &'a str,
    pub art: Art,
    pub width: f32,
    pub max_height: f32,
    pub closable: bool,
    pub stamp: Option<&'a str>,
    pub depth: Depth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plate {
    /// The name overran its band and was cut: the plate's first line writes it in full.
    pub name_cut: bool,
}

pub struct CardResponse {
    pub rect: Rect,
    pub frame: Rect,
    pub band: Response,
    pub close: Option<Response>,
}

impl<'a> Card<'a> {
    pub fn new(kind: CardKind, name: &'a str) -> Card<'a> {
        Card {
            kind,
            band: kind.word(),
            numeral: None,
            name,
            art: Art {
                icon: None,
                height: ART,
            },
            width: WIDTH,
            max_height: MAX_HEIGHT,
            closable: false,
            stamp: None,
            depth: Depth::Front,
        }
    }

    pub fn show(self, ui: &mut Ui, plate: impl FnOnce(&mut Ui, Plate)) -> CardResponse {
        let outer_min = ui.cursor().min;
        let frame_width = self.width - 2.0 * MARGIN;
        let under = ui.painter().add(Shape::Noop);
        let frame_min = outer_min + Vec2::splat(MARGIN);
        let room = Rect::from_min_size(
            frame_min,
            Vec2::new(frame_width, (self.max_height - 2.0 * MARGIN).max(0.0)),
        );
        let mut inner = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    frame_min,
                    Vec2::new(frame_width, f32::INFINITY),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        inner.set_clip_rect(room.intersect(ui.clip_rect()));
        inner.spacing_mut().item_spacing = Vec2::ZERO;
        let (band_rect, band) =
            inner.allocate_exact_size(Vec2::new(frame_width, KIND_BAND), Sense::click_and_drag());
        band.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, self.band));
        let (art_rect, _) =
            inner.allocate_exact_size(Vec2::new(frame_width, self.art.height), Sense::hover());
        let (name_rect, _) =
            inner.allocate_exact_size(Vec2::new(frame_width, NAME_BAND), Sense::hover());
        let painter = inner.painter().clone();
        self.paint_kind_band(&painter, band_rect);
        self.paint_art(&painter, art_rect);
        let name_cut = self.paint_name_band(&painter, name_rect);
        inner.put(
            name_rect,
            egui::Label::new(
                egui::RichText::new(self.name)
                    .size(0.1)
                    .color(Color32::TRANSPARENT),
            )
            .selectable(false),
        );
        let close = self.closable.then(|| {
            let rect = Rect::from_min_size(
                Pos2::new(
                    outer_min.x + self.width - CROSS_AT.x - sheet::CROSS,
                    outer_min.y + CROSS_AT.y,
                ),
                Vec2::splat(sheet::CROSS),
            );
            let response = inner.interact(rect, inner.id().with("close"), Sense::click());
            response.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, inner.is_enabled(), "Close")
            });
            sheet::cross(&painter, rect, response.hovered());
            response
        });
        let used = KIND_BAND + self.art.height + NAME_BAND;
        let plate_room = (self.max_height - 2.0 * MARGIN - used).max(0.0);
        let write = |ui: &mut Ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, LINE_GAP);
            egui::Frame::new()
                .inner_margin(PLATE_PADDING)
                .show(ui, |ui| {
                    ui.set_width(frame_width - PLATE_PADDING.sum().x);
                    plate(ui, Plate { name_cut });
                });
        };
        if self.max_height.is_finite() {
            sheet::scroll(
                &mut inner,
                egui::ScrollArea::vertical()
                    .max_height(plate_room)
                    .auto_shrink([false, true])
                    .id_salt(("card plate", self.name)),
                write,
            );
        } else {
            write(&mut inner);
        }
        let frame = Rect::from_min_max(
            frame_min,
            Pos2::new(
                frame_min.x + frame_width,
                inner.min_rect().max.y.min(room.max.y),
            ),
        );
        let outer = frame.expand(MARGIN);
        ui.allocate_rect(outer, Sense::hover());
        ui.painter()
            .set(under, Shape::Vec(self.board(outer, frame)));
        self.paint_over(ui.painter(), outer, frame, art_rect);
        CardResponse {
            rect: outer,
            frame,
            band,
            close,
        }
    }

    fn board(&self, outer: Rect, frame: Rect) -> Vec<Shape> {
        let shadow = match self.depth {
            Depth::Front => SHADOW,
            Depth::Behind => BEHIND_SHADOW,
        };
        vec![
            Shape::Rect(shadow.as_shape(outer, CORNER)),
            Shape::Rect(CARD_CONTACT_SHADOW.as_shape(outer, CORNER)),
            Shape::rect_filled(outer, CORNER, ink::CREAM),
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
        ]
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
        if let Some(numeral) = self.numeral {
            let right = if self.closable {
                band.max.x - sheet::CROSS - 10.0
            } else {
                band.max.x - 10.0
            };
            sheet::text(
                painter,
                Pos2::new(right, band.center().y),
                Align2::RIGHT_CENTER,
                TextRole::Numeral,
                numeral,
                ink::INK,
            );
        }
    }

    fn paint_art(&self, painter: &Painter, art: Rect) {
        let [top, bottom] = self.kind.ground();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(art.left_top(), top);
        mesh.colored_vertex(art.right_top(), top);
        mesh.colored_vertex(art.right_bottom(), bottom);
        mesh.colored_vertex(art.left_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        match self.art.icon {
            Some(icon) => {
                let size = Vec2::splat(self.art.icon_size());
                let rect = Rect::from_center_size(art.center(), size);
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
                    art.center(),
                    Vec2::splat(72.0_f32.min(art.height() - 8.0)),
                );
                self.kind
                    .paint(painter, rect, ink::wash(ink::CREAM, 0.85), false);
            }
        }
    }

    fn paint_name_band(&self, painter: &Painter, band: Rect) -> bool {
        painter.rect_filled(band, CornerRadius::ZERO, ink::white_wash(0.25));
        for y in [band.min.y + 1.0, band.max.y - 1.0] {
            painter.line_segment(
                [Pos2::new(band.min.x, y), Pos2::new(band.max.x, y)],
                Stroke::new(2.0, ink::INK),
            );
        }
        let room = band.width() - 16.0;
        let mut job = TextRole::Name.job(self.name, ink::INK);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let galley = painter.layout_job(job);
        let cut = galley.elided;
        painter.galley(band.center() - galley.size() / 2.0, galley, ink::INK);
        cut
    }

    fn paint_over(&self, painter: &Painter, outer: Rect, frame: Rect, art: Rect) {
        let top = Rect::from_min_size(outer.min + Vec2::new(9.0, 10.0), Vec2::splat(GLYPH));
        let bottom = Rect::from_min_size(
            outer.max - Vec2::new(9.0, 10.0) - Vec2::splat(GLYPH),
            Vec2::splat(GLYPH),
        );
        self.kind.paint(painter, top, ink::INK, false);
        self.kind.paint(painter, bottom, ink::INK, true);
        if let Some(stamp) = self.stamp {
            controls::stamp(
                painter,
                Pos2::new(frame.max.x - 44.0, art.max.y + NAME_BAND + 22.0),
                stamp,
                ink::OCHRE,
            );
        }
    }
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

pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    sheet::label(ui, TextRole::Section, text, ink::RUBRIC);
}

pub fn requirement(ui: &mut Ui, met: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (mark, color) = if met {
            (Mark::Check, ink::VERDIGRIS)
        } else {
            (Mark::Cross, ink::RUBRIC)
        };
        glyph::show(ui, mark, 12.0, color);
        ui.add(egui::Label::new(TextRole::CardBody.rich(text).color(color)).wrap());
    });
}

pub fn foot(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(TextRole::Small.rich(text).color(ink::INK3)).wrap());
}
