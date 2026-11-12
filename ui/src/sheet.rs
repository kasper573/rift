use std::sync::OnceLock;

use egui::epaint::{Mesh, Shadow, Vertex, WHITE_UV};
use egui::layers::ShapeIdx;
use egui::{
    Align2, Color32, ColorImage, CornerRadius, Id, InnerResponse, Margin, Painter, Pos2, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureId, TextureOptions, Ui, Vec2,
    WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::{controls, ink};

const RULE_INSET: f32 = 7.0;
const RULE_GAP: f32 = 8.0;
const SMALL_RULE_INSET: f32 = 4.0;
const TITLE_BAND: f32 = 36.0;
pub(crate) const CROSS: f32 = 24.0;
const SMALL_CROSS: f32 = 16.0;
pub(crate) const CURL: f32 = 24.0;
const FILLER: f32 = 34.0;
const SMALL_SHEET: f32 = 160.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    Front,
    Behind,
}

pub struct Sheet<'a> {
    pub title: Option<&'a str>,
    pub closable: bool,
    pub curl: bool,
    pub depth: Depth,
}

pub struct SheetParts {
    pub band: Rect,
    pub cross: Option<Rect>,
    pub curl: Option<Rect>,
    pub body: Rect,
}

impl Sheet<'_> {
    pub fn paint(&self, painter: &Painter, rect: Rect) -> SheetParts {
        self.paint_at(painter, rect, shadow(self.depth), Color32::WHITE)
    }

    pub fn paint_at(
        &self,
        painter: &Painter,
        rect: Rect,
        shadow: Shadow,
        tint: Color32,
    ) -> SheetParts {
        let small = is_small(rect);
        painter.add(shadow.as_shape(rect, CornerRadius::ZERO));
        vellum(painter, rect);
        let inset = if small { SMALL_RULE_INSET } else { RULE_INSET };
        rule(painter, rect, inset);
        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TITLE_BAND));
        let cross_size = if small { SMALL_CROSS } else { CROSS };
        let cross = self.closable.then(|| {
            Rect::from_min_size(
                Pos2::new(rect.max.x - 10.0 - cross_size, rect.min.y + 6.0),
                Vec2::splat(cross_size),
            )
        });
        if let Some(title) = self.title {
            title_band(painter, title_room(band, small), title, !small);
        }
        let curl = self
            .curl
            .then(|| Rect::from_min_max(rect.max - Vec2::splat(CURL), rect.max));
        if let Some(curl) = curl {
            paint_curl(painter, curl);
        }
        let top = if self.title.is_some() {
            band.max.y
        } else {
            rect.min.y + inset + 4.0
        };
        let body = Rect::from_min_max(
            Pos2::new(rect.min.x + inset + 4.0, top),
            rect.max - Vec2::splat(inset + 4.0),
        );
        if tint != Color32::WHITE {
            painter.rect_filled(rect, CornerRadius::ZERO, tint);
        }
        SheetParts {
            band,
            cross,
            curl,
            body,
        }
    }
}

pub fn body(rect: Rect, titled: bool) -> Rect {
    let small = is_small(rect);
    let margin = if small { SMALL_RULE_INSET } else { RULE_INSET } + 4.0;
    let top = if titled {
        rect.min.y + TITLE_BAND
    } else {
        rect.min.y + margin
    };
    Rect::from_min_max(
        Pos2::new(rect.min.x + margin, top),
        rect.max - Vec2::splat(margin),
    )
    .intersect(rect)
}

pub fn chrome(titled: bool) -> Vec2 {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0 * SMALL_SHEET));
    rect.size() - body(rect, titled).size()
}

pub const fn shadow(depth: Depth) -> Shadow {
    match depth {
        Depth::Front => Shadow {
            offset: [0, 12],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(128),
        },
        Depth::Behind => Shadow {
            offset: [0, 6],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(90),
        },
    }
}

const SHEET_CONTACT_SHADOW: Shadow = Shadow {
    offset: [0, 2],
    blur: 3,
    spread: 0,
    color: Color32::from_black_alpha(90),
};

pub fn vellum(painter: &Painter, rect: Rect) {
    painter.add(vellum_shape(painter.ctx(), rect));
}

pub fn vellum_shape(ctx: &egui::Context, rect: Rect) -> Shape {
    let grain = grain(ctx);
    let tile = grain.size_vec2();
    Shape::mesh(vignette(rect, grain.id(), |offset| {
        (offset / tile).to_pos2()
    }))
}

/// Vellum without its grain, for a box too small to show it: the grain's texture would cost the box
/// a draw call of its own.
pub fn radial(painter: &Painter, rect: Rect) {
    painter.add(Shape::mesh(vignette(rect, TextureId::default(), |_| {
        WHITE_UV
    })));
}

pub fn note<R>(ui: &mut Ui, padding: Margin, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    note_shadowed(ui, padding, shadow(Depth::Front), add)
}

/// A note casting `cast` rather than the shadow of a note at the front, as one rising or settling
/// does.
pub fn note_shadowed<R>(
    ui: &mut Ui,
    padding: Margin,
    cast: Shadow,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let under = ui.painter().add(Shape::Noop);
    let framed = egui::Frame::new().inner_margin(padding).show(ui, add);
    paint_note(ui.painter(), under, framed.response.rect, cast);
    framed
}

pub(crate) fn paint_note(painter: &Painter, under: ShapeIdx, rect: Rect, cast: Shadow) {
    let mut shapes = vec![
        Shape::Rect(cast.as_shape(rect, CornerRadius::ZERO)),
        Shape::Rect(SHEET_CONTACT_SHADOW.as_shape(rect, CornerRadius::ZERO)),
    ];
    shapes.push(vellum_shape(painter.ctx(), rect));
    painter.set(under, Shape::Vec(shapes));
    rule(painter, rect, RULE_INSET);
}

pub fn rule(painter: &Painter, rect: Rect, inset: f32) {
    painter.rect_stroke(
        rect.shrink(inset),
        CornerRadius::ZERO,
        Stroke::new(1.0, ink::ink_wash(0.5)),
        StrokeKind::Inside,
    );
    painter.rect_stroke(
        rect.shrink(inset + 2.0),
        CornerRadius::ZERO,
        Stroke::new(1.0, ink::ink_wash(0.16)),
        StrokeKind::Inside,
    );
}

/// An upright line pressed into the paper: a shadowed edge with a lit one beside it.
pub fn groove(painter: &Painter, top: Pos2, bottom: Pos2) {
    let x = top.x.round() - 0.5;
    let line = |x: f32, color: Color32| {
        painter.line_segment(
            [Pos2::new(x, top.y), Pos2::new(x, bottom.y)],
            Stroke::new(1.0, color),
        );
    };
    line(x, ink::ink_wash(0.42));
    line(x + 1.0, ink::white_wash(0.55));
}

/// Where the title of a sheet standing in `rect` starts, as its band draws it.
pub fn title_origin(ctx: &egui::Context, rect: Rect, title: &str) -> Pos2 {
    let small = is_small(rect);
    let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TITLE_BAND));
    let galley = ctx.fonts_mut(|fonts| fonts.layout_job(TextRole::Title.job(title, ink::RUBRIC)));
    title_at(title_room(band, small), galley.size())
}

fn title_band(painter: &Painter, band: Rect, title: &str, fillers: bool) {
    let galley = painter.layout_job(TextRole::Title.job(title, ink::RUBRIC));
    let width = galley.size().x.min(band.width());
    let at = title_at(band, galley.size());
    if galley.size().x > band.width() {
        painter
            .with_clip_rect(band.intersect(painter.clip_rect()))
            .galley(at, galley, ink::RUBRIC);
        return;
    }
    painter.galley(at, galley, ink::RUBRIC);
    if !fillers {
        return;
    }
    let y = band.center().y;
    let gap = 10.0;
    let left = Pos2::new(at.x - gap, y);
    let right = Pos2::new(at.x + width + gap, y);
    let reach = (left.x - band.min.x).clamp(0.0, FILLER);
    fading_line(
        painter,
        left,
        left - Vec2::new(reach, 0.0),
        ink::RUBRIC,
        1.0,
    );
    fading_line(
        painter,
        right,
        right + Vec2::new(reach, 0.0),
        ink::RUBRIC,
        1.0,
    );
}

fn fading_line(painter: &Painter, from: Pos2, to: Pos2, color: Color32, width: f32) {
    let along = to - from;
    if along.length() <= f32::EPSILON {
        return;
    }
    let side = Vec2::new(-along.y, along.x).normalized() * (width / 2.0);
    let clear = ink::wash(color, 0.0);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(from - side, color);
    mesh.colored_vertex(from + side, color);
    mesh.colored_vertex(to + side, clear);
    mesh.colored_vertex(to - side, clear);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

pub fn cross(painter: &Painter, rect: Rect, hovered: bool) {
    if hovered {
        painter.rect_filled(rect, 3.0, ink::ink_wash(0.1));
    }
    let bar = rect.width() * (16.0 / 24.0) / 2.0;
    let width = if rect.width() < CROSS { 1.5 } else { 2.0 };
    let centre = rect.center();
    let arm = Vec2::splat(bar * std::f32::consts::FRAC_1_SQRT_2);
    let stroke = Stroke::new(width, ink::INK);
    painter.line_segment([centre - arm, centre + arm], stroke);
    painter.line_segment(
        [
            centre + Vec2::new(-arm.x, arm.y),
            centre + Vec2::new(arm.x, -arm.y),
        ],
        stroke,
    );
}

pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, tabs: &[(T, String)], active: &mut T) {
    let baseline = ui.painter().add(Shape::Noop);
    let mut lip = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (tab, label) in tabs {
            let selected = *tab == *active;
            let color = if selected { ink::RUBRIC } else { ink::INK2 };
            let galley = ui.painter().layout_job(TextRole::Tab.job(label, color));
            let size = galley.size() + Vec2::new(24.0, 10.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label)
            });
            let radius = CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            };
            let fill = if selected {
                ink::LIGHT
            } else if response.hovered() {
                ink::gold_wash(0.2)
            } else {
                ink::ink_wash(0.05)
            };
            let border = ink::ink_wash(if selected { 0.5 } else { 0.35 });
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, color);
            if response.has_focus() {
                controls::focus_ring(ui.painter(), rect, radius);
            }
            if selected {
                lip = Some(rect);
            }
            if response.clicked() {
                *active = *tab;
            }
        }
    });
    let y = row.response.rect.bottom() - 0.5;
    let span = ui.max_rect().x_range();
    ui.painter().set(
        baseline,
        Shape::line_segment(
            [Pos2::new(span.min, y), Pos2::new(span.max, y)],
            Stroke::new(1.0, ink::ink_wash(0.35)),
        ),
    );
    if let Some(lip) = lip {
        ui.painter().line_segment(
            [Pos2::new(lip.min.x + 1.0, y), Pos2::new(lip.max.x - 1.0, y)],
            Stroke::new(1.0, ink::LIGHT),
        );
    }
    ui.add_space(6.0);
}

pub fn section(ui: &mut Ui, text: &str) {
    section_with(ui, text, |_| ());
}

/// A section heading with `actions` set at its right end, the heading's rule running between the
/// two and stopping short of the actions.
pub fn section_with<R>(ui: &mut Ui, text: &str, actions: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(6.0);
    let galley = ui
        .painter()
        .layout_job(TextRole::Section.job(text, ink::RUBRIC));
    let width = ui.available_width();
    let band = Rect::from_min_size(ui.cursor().min, Vec2::new(width, controls::CONTROL_HEIGHT));
    let mut tools = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(band)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let made = actions(&mut tools);
    let used = tools.min_rect();
    let height = if used.width() > 0.0 {
        band.height()
    } else {
        galley.size().y + 2.0
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    let at = Pos2::new(rect.min.x, rect.center().y - galley.size().y / 2.0);
    let text_width = galley.size().x;
    ui.painter().galley(at, galley, ink::RUBRIC);
    let end = if used.width() > 0.0 {
        used.min.x - RULE_GAP
    } else {
        rect.max.x
    };
    let y = rect.center().y + 0.5;
    let start = rect.min.x + text_width + RULE_GAP;
    if end > start {
        fading_line(
            ui.painter(),
            Pos2::new(start, y),
            Pos2::new(end, y),
            ink::RUBRIC,
            1.0,
        );
    }
    ui.add_space(4.0);
    made
}

pub fn scroll<R>(
    ui: &mut Ui,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let kept = ui.visuals().clone();
    let visuals = ui.visuals_mut();
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.bg_fill = ink::wash(ink::RUBRIC, 0.85);
    }
    visuals.extreme_bg_color = ink::ink_wash(0.1);
    let shown = area.show(ui, |ui| {
        *ui.visuals_mut() = kept.clone();
        add(ui)
    });
    *ui.visuals_mut() = kept;
    shown
}

pub fn sunk(painter: &Painter, rect: Rect, radius: f32) {
    painter.rect(
        rect,
        radius,
        ink::ink_wash(0.06),
        Stroke::new(1.0, ink::ink_wash(0.55)),
        StrokeKind::Inside,
    );
}

const GRAIN: usize = 192;

/// The grain multiplies the vignette's tones rather than lying over them, so vellum is one mesh on
/// one texture, which egui draws in a single call.
pub(crate) fn grain(ctx: &egui::Context) -> TextureHandle {
    let id = Id::new("vellum grain");
    if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let mut pixels = Vec::with_capacity(GRAIN * GRAIN);
    for y in 0..GRAIN {
        for x in 0..GRAIN {
            let fibre = noise(x as u32, y as u32) % 24;
            let streak = noise(x as u32 / 3, y as u32 * 7) % 9;
            let inked = ink::on(ink::CREAM, ink::INK, (fibre + streak) as f32 / 255.0 * 0.6);
            let over = |inked: u8, cream: u8| (u32::from(inked) * 255 / u32::from(cream)) as u8;
            pixels.push(Color32::from_rgb(
                over(inked.r(), ink::CREAM.r()),
                over(inked.g(), ink::CREAM.g()),
                over(inked.b(), ink::CREAM.b()),
            ));
        }
    }
    let texture = ctx.load_texture(
        "vellum grain",
        ColorImage::new([GRAIN, GRAIN], pixels),
        TextureOptions::NEAREST_REPEAT,
    );
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

fn noise(x: u32, y: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

const VIGNETTE_CELLS: usize = 12;

fn vignette(rect: Rect, texture: TextureId, uv: impl Fn(Vec2) -> Pos2) -> Mesh {
    let grid = vignette_grid();
    let vertices = grid
        .points
        .iter()
        .map(|&(at, color)| {
            let offset = rect.size() * at;
            Vertex {
                pos: rect.min + offset,
                uv: uv(offset),
                color,
            }
        })
        .collect();
    Mesh {
        indices: grid.indices.clone(),
        vertices,
        texture_id: texture,
    }
}

/// The vignette's grid of tones over a unit square, worked out once, since every vellum surface
/// lays it over its rect each frame.
struct VignetteGrid {
    points: Vec<(Vec2, Color32)>,
    indices: Vec<u32>,
}

fn vignette_grid() -> &'static VignetteGrid {
    static GRID: OnceLock<VignetteGrid> = OnceLock::new();
    GRID.get_or_init(|| {
        let stride = VIGNETTE_CELLS + 1;
        let points = (0..stride * stride)
            .map(|index| {
                let at = Vec2::new(
                    (index % stride) as f32 / VIGNETTE_CELLS as f32,
                    (index / stride) as f32 / VIGNETTE_CELLS as f32,
                );
                (at, tone(at.x * 2.0 - 1.0, at.y * 2.0 - 1.0))
            })
            .collect();
        let stride = stride as u32;
        let indices = (0..VIGNETTE_CELLS as u32)
            .flat_map(|row| (0..VIGNETTE_CELLS as u32).map(move |column| row * stride + column))
            .flat_map(|a| [a, a + 1, a + stride, a + 1, a + stride + 1, a + stride])
            .collect();
        VignetteGrid { points, indices }
    })
}

pub(crate) fn tone(dx: f32, dy: f32) -> Color32 {
    let distance = (dx.powi(4) + dy.powi(4)).powf(0.25).min(1.0);
    let centre = ink::on(ink::LIGHT, ink::CREAM, smooth(0.0, 0.7, distance));
    let edge = ink::on(centre, ink::CREAM2, smooth(0.7, 1.0, distance));
    ink::on(edge, ink::HONEY, smooth(0.93, 1.0, distance) * 0.7)
}

pub(crate) fn smooth(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn paint_curl(painter: &Painter, corner: Rect) {
    let a = Pos2::new(corner.min.x, corner.max.y);
    let b = Pos2::new(corner.max.x, corner.min.y);
    let c = corner.max;
    painter.add(Shape::convex_polygon(
        vec![a, b, c],
        ink::HONEY2,
        Stroke::NONE,
    ));
    let tip = corner.min + Vec2::splat(3.0);
    painter.add(Shape::convex_polygon(
        vec![a, b, tip],
        ink::LIGHT,
        Stroke::new(1.0, ink::ink_wash(0.3)),
    ));
    painter.line_segment([a, b], Stroke::new(1.0, ink::ink_wash(0.45)));
    painter.add(Shape::convex_polygon(
        vec![a + Vec2::new(2.0, 0.0), b + Vec2::new(0.0, 2.0), c],
        ink::ink_wash(0.12),
        Stroke::NONE,
    ));
}

pub fn label(ui: &mut Ui, role: TextRole, text: &str, color: Color32) -> Response {
    ui.add(egui::Label::new(role.widget_text(text, color)).selectable(false))
}

pub fn text(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    role: TextRole,
    text: &str,
    color: Color32,
) -> Rect {
    let galley = painter.layout_job(role.job(text, color));
    let rect = anchor.anchor_size(at, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

fn is_small(rect: Rect) -> bool {
    rect.width() < SMALL_SHEET || rect.height() < SMALL_SHEET
}

fn title_room(band: Rect, small: bool) -> Rect {
    let cross = if small { SMALL_CROSS } else { CROSS };
    band.shrink2(Vec2::new(cross + 14.0, 0.0))
}

fn title_at(room: Rect, title: Vec2) -> Pos2 {
    Pos2::new(
        room.center().x - title.x.min(room.width()) / 2.0,
        room.center().y - title.y / 2.0,
    )
}
