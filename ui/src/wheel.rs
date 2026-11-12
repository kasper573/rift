//! A wheel of things to pick, laid round a point: what a held key opens round a character. It is a
//! folding fan, its ribs turning on a rivet at the point and its pleated paper opening between
//! them, round from the top until its two guards meet.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::epaint::{Mesh, Vertex};
use egui::{
    Color32, Context, Id, Image, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    TextureHandle, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::{ink, motion, sheet};

/// How wide the picture of a pick is drawn.
pub const PICTURE: f32 = 32.0;

/// How far from the wheel's middle the middle of its paper lies at the least, how wide the paper
/// is from its inner edge to its outer, and how much room each leaf takes of the round where more
/// of them need a wider one.
const REACH: f32 = 119.0;
const BAND: f32 = 66.0;
const LEAF: f32 = PICTURE * 1.5;
/// How far the paper's edge dips at each rib, and how far a leaf under the pointer lifts.
const SCALLOP: f32 = 6.0;
const LIFT: f32 = 6.0;
const RIVET: f32 = 6.0;
/// How much of the way open the fan has come by the time it is wholly seen, and how far open a
/// leaf is before its picture starts to show.
const SEEN_BY: f32 = 0.12;
const PICTURED_FROM: f32 = 0.45;
/// How many points along its edge a leaf's paper is toned at, and how far out across its width.
const ACROSS: usize = 8;
const OUT: [f32; 5] = [0.0, 0.07, 0.5, 0.93, 1.0];

/// One thing to pick off a wheel: its picture once there is one to draw, and its name, which a
/// screen reader says in place of the picture.
pub struct WheelPick<'a> {
    pub name: &'a str,
    pub image: Option<Image<'a>>,
}

/// How far open the wheel stands, from 0 folded shut to 1 round its full circle: it opens while
/// `held`, and folds, quicker, once let go.
pub fn opened(ctx: &Context, held: bool) -> f32 {
    let seconds = if held {
        motion::FAN_OPEN_SECS
    } else {
        motion::FAN_CLOSE_SECS
    };
    ctx.animate_bool_with_time(Id::new("wheel"), held, seconds)
}

/// Lays `picks` on the leaves of a fan `opened` that far round `center`, the first at the top
/// and the rest clockwise. Returns the place of the one clicked: a fan takes clicks only once it
/// stands wholly open.
pub fn wheel(ui: &mut Ui, center: Pos2, picks: Vec<WheelPick>, opened: f32) -> Option<usize> {
    if picks.is_empty() || opened <= 0.0 {
        return None;
    }
    let fan = Fan::new(center, picks.len(), motion::ease_out(opened));
    let open = opened >= 1.0;
    let sense = if open { Sense::click() } else { Sense::hover() };
    let round = ui.allocate_rect(fan.bounds(), sense);
    let leaves: Vec<Response> = if open {
        picks
            .iter()
            .enumerate()
            .map(|(leaf, pick)| {
                let room = Rect::from_center_size(fan.middle(leaf, 0.0), Vec2::splat(LEAF));
                let response = ui.interact(room, round.id.with(leaf), Sense::click());
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, pick.name));
                response
            })
            .collect()
    } else {
        Vec::new()
    };
    let over = round.contains_pointer() || leaves.iter().any(Response::contains_pointer);
    let pointed = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|_| open && over)
        .and_then(|pointer| fan.leaf_at(pointer));
    let clicked = round.clicked() || leaves.iter().any(Response::clicked);

    ui.set_opacity(sheet::smooth(0.0, SEEN_BY, opened));
    let painter = ui.painter().clone();
    let paper = sheet::grain(ui.ctx());
    for edge in 0..=fan.leaves {
        fan.rib(&painter, edge, fan.inner + 4.0, Stroke::new(2.5, ink::INK2));
    }
    let pictured = sheet::smooth(PICTURED_FROM, 1.0, fan.open);
    for (leaf, pick) in picks.into_iter().enumerate() {
        let lit = pointed == Some(leaf);
        let lift = if lit { LIFT } else { 0.0 };
        fan.leaf(&painter, &paper, leaf, lift, lit);
        if let Some(image) = pick.image {
            let size = PICTURE * (0.4 + 0.6 * pictured) * if lit { 1.15 } else { 1.0 };
            image
                .tint(Color32::from_white_alpha((pictured * 255.0) as u8))
                .paint_at(
                    ui,
                    Rect::from_center_size(fan.middle(leaf, lift), Vec2::splat(size)),
                );
        }
    }
    let guard = Stroke::new(4.0, ink::FRAME);
    fan.rib(&painter, 0, fan.outer - 4.0, guard);
    fan.rib(&painter, fan.leaves, fan.outer - 4.0, guard);
    painter.circle(center, RIVET, ink::GOLD, Stroke::new(1.5, ink::FRAME));
    pointed.filter(|_| clicked)
}

/// A fan round `center` of `leaves` leaves, opened `open` of the way round from the top: its
/// paper lies between `inner` and `outer`, each leaf `width` round.
struct Fan {
    center: Pos2,
    leaves: usize,
    open: f32,
    inner: f32,
    outer: f32,
    width: f32,
}

impl Fan {
    fn new(center: Pos2, leaves: usize, open: f32) -> Fan {
        let reach = REACH.max(LEAF * leaves as f32 / TAU);
        Fan {
            center,
            leaves,
            open,
            inner: reach - BAND / 2.0,
            outer: reach + BAND / 2.0,
            width: TAU / leaves as f32 * open,
        }
    }

    fn bounds(&self) -> Rect {
        Rect::from_center_size(self.center, Vec2::splat(2.0 * (self.outer + LIFT)))
    }

    /// The angle of the rib at `edge`, the first leaf's first edge at 0: the first leaf lies
    /// across the top.
    fn edge(&self, edge: f32) -> f32 {
        -FRAC_PI_2 + (edge - 0.5) * self.width
    }

    fn at(&self, radius: f32, angle: f32) -> Pos2 {
        self.center + radius * Vec2::angled(angle)
    }

    /// The middle of `leaf`'s paper, lifted `lift` out.
    fn middle(&self, leaf: usize, lift: f32) -> Pos2 {
        self.at(
            (self.inner + self.outer + lift) / 2.0,
            self.edge(leaf as f32 + 0.5),
        )
    }

    /// The leaf whose paper lies under `point`.
    fn leaf_at(&self, point: Pos2) -> Option<usize> {
        let from = point - self.center;
        if !(self.inner..=self.outer + LIFT).contains(&from.length()) {
            return None;
        }
        let round = (from.angle() - self.edge(0.0)).rem_euclid(TAU);
        Some(((round / self.width) as usize).min(self.leaves - 1))
    }

    /// How far out the paper's edge lies `across` a leaf, from 0 at one rib to 1 at the next: it
    /// peaks on the crease between them.
    fn brim(&self, across: f32, lift: f32) -> f32 {
        self.outer + lift - SCALLOP * (2.0 * across - 1.0).abs()
    }

    fn rib(&self, painter: &Painter, edge: usize, to: f32, stroke: Stroke) {
        let angle = self.edge(edge as f32);
        painter.line_segment([self.at(RIVET, angle), self.at(to, angle)], stroke);
    }

    /// The paper of `leaf` with its crease and its edges inked, lit under the pointer.
    fn leaf(&self, painter: &Painter, paper: &TextureHandle, leaf: usize, lift: f32, lit: bool) {
        let angle = |across: f32| self.edge(leaf as f32 + across);
        let tile = paper.size_vec2();
        let mut mesh = Mesh::with_texture(paper.id());
        for step in 0..=ACROSS {
            let across = step as f32 / ACROSS as f32;
            let brim = self.brim(across, lift);
            let folded = if across > 0.5 { 0.2 } else { 0.0 } + 0.25 * (1.0 - self.open);
            for out in OUT {
                let pos = self.at(self.inner + (brim - self.inner) * out, angle(across));
                let tone = ink::on(sheet::tone(0.0, out * 2.0 - 1.0), ink::CREAM2, folded);
                mesh.vertices.push(Vertex {
                    pos,
                    uv: ((pos - self.center) / tile).to_pos2(),
                    color: if lit {
                        ink::on(tone, ink::HONEY2, 0.55)
                    } else {
                        tone
                    },
                });
            }
        }
        let rows = OUT.len() as u32;
        for column in 0..ACROSS as u32 {
            for row in 0..rows - 1 {
                let a = column * rows + row;
                let b = a + rows;
                mesh.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        painter.add(Shape::mesh(mesh));

        let brim = [0.0, 0.5, 1.0].map(|across| self.at(self.brim(across, lift), angle(across)));
        let hem: Vec<Pos2> = (0..=ACROSS)
            .map(|step| self.at(self.inner, angle(step as f32 / ACROSS as f32)))
            .collect();
        painter.line_segment(
            [self.at(self.inner, angle(0.5)), brim[1]],
            Stroke::new(1.0, ink::ink_wash(0.18)),
        );
        painter.line_segment(
            [self.at(self.inner, angle(0.0)), brim[0]],
            Stroke::new(1.0, ink::ink_wash(0.4)),
        );
        painter.add(Shape::line(brim.to_vec(), Stroke::new(1.5, ink::INK2)));
        painter.add(Shape::line(hem.clone(), Stroke::new(1.5, ink::GOLD)));
        if lit {
            let rim = hem.into_iter().rev().chain(brim).collect();
            painter.add(Shape::closed_line(rim, Stroke::new(2.0, ink::GOLD)));
        }
    }
}
