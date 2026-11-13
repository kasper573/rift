use std::sync::{Arc, OnceLock};

use egui::epaint::{Mesh, Shadow, Vertex};
use egui::layers::ShapeIdx;
use egui::{
    Align2, Color32, ColorImage, CornerRadius, Galley, Id, InnerResponse, Margin, Painter, Pos2,
    Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureId, TextureOptions, Ui,
    Vec2, WidgetInfo, WidgetType,
};

use crate::face::TextRole;
use crate::{controls, float, ink};

const RULE_INSET: f32 = 7.0;
const RULE_GAP: f32 = 8.0;
const SMALL_RULE_INSET: f32 = 4.0;
const TITLE_BAND: f32 = 36.0;
pub const CROSS: f32 = 24.0;
pub const CROSS_MARGIN: f32 = 8.0;
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
        let bleeds = self.title.is_none() && !small;
        let cross = self.closable.then(|| {
            let at = if bleeds {
                let from_rule = inset + CROSS_MARGIN;
                Pos2::new(rect.max.x - from_rule - cross_size, rect.min.y + from_rule)
            } else {
                Pos2::new(rect.max.x - 10.0 - cross_size, rect.min.y + 6.0)
            };
            Rect::from_min_size(at, Vec2::splat(cross_size))
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
        let body = if bleeds {
            rect.shrink(inset)
        } else {
            Rect::from_min_max(
                Pos2::new(rect.min.x + inset + 4.0, top),
                rect.max - Vec2::splat(inset + 4.0),
            )
        };
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

pub const GUTTER: f32 = 16.0;
const BODY_INSET: f32 = 4.0;
pub const PAGE_MARGIN: f32 = GUTTER - BODY_INSET;

pub fn page(body: Rect) -> Rect {
    body.shrink2(Vec2::new(PAGE_MARGIN, 0.0))
}

pub fn rule_stroke() -> Stroke {
    Stroke::new(1.0, ink::ink_wash(0.35))
}

/// The room a pane keeps clear around its content.
pub const PANE_PAD: Vec2 = Vec2::new(20.0, 16.0);

/// One of the panes a sheet's body parts into: all of it, rule to rule, and the room its
/// content keeps within its padding.
#[derive(Clone, Copy, Debug)]
pub struct Pane {
    pub outer: Rect,
    pub inner: Rect,
}

impl Pane {
    /// A pane that is only its content, its bar standing just beside it.
    pub fn bare(rect: Rect) -> Pane {
        Pane {
            outer: Rect::from_min_max(rect.min, rect.max + Vec2::new(BAR_LANE, 0.0)),
            inner: rect,
        }
    }

    /// The pane above a row `foot` tall kept at its bottom.
    pub fn above_foot(self, foot: f32) -> Pane {
        let bottom = self.inner.max.y - foot;
        Pane {
            outer: Rect::from_min_max(self.outer.min, Pos2::new(self.outer.max.x, bottom)),
            inner: Rect::from_min_max(self.inner.min, Pos2::new(self.inner.max.x, bottom)),
        }
    }
}

const BAR: f32 = 6.0;
const BAR_INSET: f32 = 2.0;
const BAR_LANE: f32 = BAR + BAR_INSET;

pub fn spaced_columns<const N: usize>(room: Rect, widths: [f32; N]) -> ([Rect; N], Rect) {
    let mut left = room.min.x;
    let columns = widths.map(|width| {
        let column = Rect::from_min_max(
            Pos2::new(left, room.min.y),
            Pos2::new((left + width).min(room.max.x), room.max.y),
        );
        left = column.max.x + GUTTER;
        column
    });
    let rest = Rect::from_min_max(Pos2::new(left.min(room.max.x), room.min.y), room.max);
    (columns, rest)
}

/// `room` parted from its left into panes that hold `widths` of content, and the pane left
/// beside them, a rule between each two running the whole height of the room.
pub fn columns<const N: usize>(ui: &Ui, room: Rect, widths: [f32; N]) -> ([Pane; N], Pane) {
    let pad = PANE_PAD;
    let mut left = room.min.x;
    let columns = widths.map(|width| {
        let outer = Rect::from_min_max(
            Pos2::new(left, room.min.y),
            Pos2::new((left + width + 2.0 * pad.x).min(room.max.x), room.max.y),
        );
        left = outer.max.x;
        ui.painter().vline(left, room.y_range(), rule_stroke());
        Pane {
            outer,
            inner: outer.shrink2(pad),
        }
    });
    let rest = Rect::from_min_max(Pos2::new(left.min(room.max.x), room.min.y), room.max);
    (
        columns,
        Pane {
            outer: rest,
            inner: rest.shrink2(pad),
        },
    )
}

fn bars(ui: &mut Ui) {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.bar_width = BAR;
    scroll.bar_inner_margin = 0.0;
    scroll.bar_outer_margin = BAR_INSET;
}

pub fn scroll_page<R>(
    ui: &mut Ui,
    page: Rect,
    least: Vec2,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let lane = |needed: bool| if needed { BAR_LANE } else { 0.0 };
    let view = Vec2::new(
        page.width() - lane(page.height() < least.y),
        page.height() - lane(page.width() < least.x),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(page), |ui| {
        bars(ui);
        scroll(ui, area.auto_shrink(false), |ui| {
            let (rect, _) = ui.allocate_exact_size(view.max(least), Sense::hover());
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add)
                .inner
        })
        .inner
    })
    .inner
}

/// What `pane` holds, scrolled up and down by `area` when it outgrows the pane: it scrolls from
/// rule to rule, its padding scrolling with it, and its bar stands at the pane's edge.
pub fn scroll_pane<R>(
    ui: &mut Ui,
    pane: Pane,
    area: egui::ScrollArea,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let Pane { outer, inner } = pane;
    ui.scope_builder(egui::UiBuilder::new().max_rect(outer), |ui| {
        bars(ui);
        scroll(ui, area.auto_shrink(false), |ui| {
            ui.add_space(inner.min.y - outer.min.y);
            let top = ui.cursor().min.y;
            let room = Rect::from_min_max(
                Pos2::new(inner.min.x, top),
                Pos2::new(inner.max.x, f32::INFINITY),
            );
            let made = ui
                .scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    ui.set_width(inner.width());
                    add(ui)
                })
                .inner;
            ui.add_space(outer.max.y - inner.max.y);
            made
        })
        .inner
    })
    .inner
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabSize {
    Main,
    Sub,
}

impl TabSize {
    fn role(self) -> TextRole {
        match self {
            TabSize::Main => TextRole::MainTab,
            TabSize::Sub => TextRole::Tab,
        }
    }

    fn padding(self) -> Vec2 {
        match self {
            TabSize::Main => Vec2::new(11.0, 7.0),
            TabSize::Sub => Vec2::new(12.0, 5.0),
        }
    }
}

pub struct Tab<T> {
    pub key: T,
    pub label: String,
    /// A shut tab keeps its place in the strip, so the tabs beside it never move.
    pub shut_because: Option<String>,
    pub icon: Option<TextureId>,
    pub keys: Option<String>,
}

impl<T> Tab<T> {
    pub fn new(key: T, label: impl Into<String>) -> Tab<T> {
        Tab {
            key,
            label: label.into(),
            shut_because: None,
            icon: None,
            keys: None,
        }
    }
}

pub fn tabs<T: PartialEq + Copy>(
    ui: &mut Ui,
    size: TabSize,
    tabs: &[Tab<T>],
    active: &mut T,
    room: f32,
) -> Rect {
    let strip = tabs_inset(ui, size, tabs, active, room, 0.0);
    ui.add_space(6.0);
    strip
}

/// A strip of tabs that begins `inset` in from the left, over a rule the whole width of the room.
pub fn tabs_inset<T: PartialEq + Copy>(
    ui: &mut Ui,
    size: TabSize,
    tabs: &[Tab<T>],
    active: &mut T,
    room: f32,
    inset: f32,
) -> Rect {
    let wide = |fit: TabFit| {
        let faces: f32 = tabs
            .iter()
            .map(|tab| TabFace::of(ui, size, tab, ink::INK, fit).size(size).x)
            .sum();
        faces + TAB_SPACING * tabs.len().saturating_sub(1) as f32
    };
    let fit = [TabFit::Whole, TabFit::NoKeys]
        .into_iter()
        .find(|&fit| wide(fit) <= room)
        .unwrap_or(TabFit::IconsAlone);
    let baseline = ui.painter().add(Shape::Noop);
    let mut lip = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(inset);
        ui.spacing_mut().item_spacing.x = TAB_SPACING;
        for tab in tabs {
            let selected = tab.key == *active;
            let response = tab_button(ui, size, tab, selected, fit);
            if selected {
                lip = Some(response.rect);
            }
            if tab.shut_because.is_none() && response.clicked() {
                *active = tab.key;
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
    row.response.rect
}

const TAB_SPACING: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabFit {
    Whole,
    NoKeys,
    IconsAlone,
}

struct TabFace {
    icon: Option<TextureId>,
    label: Option<Arc<Galley>>,
    cap: Option<Arc<Galley>>,
}

impl TabFace {
    fn of<T>(ui: &Ui, size: TabSize, tab: &Tab<T>, color: Color32, fit: TabFit) -> TabFace {
        let lettered = fit != TabFit::IconsAlone || tab.icon.is_none();
        let label = lettered.then(|| ui.painter().layout_job(size.role().job(&tab.label, color)));
        let keys = tab.keys.as_deref().filter(|_| fit == TabFit::Whole);
        TabFace {
            icon: tab.icon,
            label,
            cap: keys.map(|keys| controls::key_galley(ui, keys, ink::INK3)),
        }
    }

    fn parts(&self) -> impl Iterator<Item = Vec2> {
        let icon = self.icon.map(|_| Vec2::splat(TAB_ICON));
        let label = self.label.as_ref().map(|label| label.size());
        let cap = self.cap.as_ref().map(controls::key_cap_size);
        [icon, label, cap].into_iter().flatten()
    }

    fn size(&self, size: TabSize) -> Vec2 {
        let across: f32 = self.parts().map(|part| part.x).sum();
        let gaps = TAB_GAP * self.parts().count().saturating_sub(1) as f32;
        let tall = self.label.as_ref().map_or(TAB_ICON, |label| label.size().y);
        let tall = self
            .cap
            .as_ref()
            .map_or(tall, |cap| tall.max(controls::key_cap_size(cap).y));
        Vec2::new(across + gaps, tall) + 2.0 * size.padding()
    }
}

fn tab_button<T>(
    ui: &mut Ui,
    size: TabSize,
    tab: &Tab<T>,
    selected: bool,
    fit: TabFit,
) -> Response {
    let open = tab.shut_because.is_none();
    let color = match (selected, open) {
        (_, false) => ink::INK3,
        (true, true) => ink::RUBRIC,
        (false, true) => ink::INK2,
    };
    let face = TabFace::of(ui, size, tab, color, fit);
    let padding = size.padding();
    let (rect, response) = ui.allocate_exact_size(face.size(size), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, open, selected, &tab.label)
    });
    if let Some(why) = &tab.shut_because {
        float::tooltip(&response, |ui| {
            ui.label(TextRole::Tooltip.rich(why));
        });
    } else if fit != TabFit::Whole {
        float::tooltip(&response, |ui| {
            ui.horizontal(|ui| {
                ui.label(TextRole::Tooltip.rich(&tab.label));
                if let Some(keys) = &tab.keys {
                    controls::key_hint(ui, keys);
                }
            });
        });
    }
    let radius = CornerRadius {
        nw: 4,
        ne: 4,
        sw: 0,
        se: 0,
    };
    let fill = if selected {
        ink::LIGHT
    } else if open && response.hovered() {
        ink::gold_wash(0.2)
    } else {
        ink::ink_wash(0.05)
    };
    let border = ink::ink_wash(match (selected, open) {
        (_, false) => 0.2,
        (true, true) => 0.5,
        (false, true) => 0.35,
    });
    let painter = ui.painter();
    painter.rect(
        rect,
        radius,
        fill,
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let mut left = rect.min.x + padding.x;
    let middle = rect.center().y;
    if let Some(texture) = face.icon {
        let at = Rect::from_min_size(
            Pos2::new(left, middle - TAB_ICON / 2.0),
            Vec2::splat(TAB_ICON),
        );
        let tint = if selected || !open {
            Color32::WHITE
        } else {
            ink::wash(Color32::WHITE, 0.8)
        };
        painter.image(
            texture,
            at,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            tint,
        );
        left += TAB_ICON + TAB_GAP;
    }
    if let Some(label) = face.label {
        let across = label.size().x;
        painter.galley(Pos2::new(left, middle - label.size().y / 2.0), label, color);
        left += across + TAB_GAP;
    }
    if let Some(cap) = face.cap {
        let cap_size = controls::key_cap_size(&cap);
        let at = Pos2::new(left, middle - cap_size.y / 2.0);
        controls::paint_key_cap(painter, Rect::from_min_size(at, cap_size), cap, ink::INK3);
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, radius);
    }
    response
}

const TAB_ICON: f32 = 18.0;
const TAB_GAP: f32 = 7.0;

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
        widget.bg_fill = ink::ink_wash(0.38);
    }
    visuals.extreme_bg_color = ink::ink_wash(0.05);
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
