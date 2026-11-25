//! What floats over the interface: tooltips, what follows the pointer, toasts, and areas that
//! keep their place as they change size.

use std::hash::{Hash, Hasher};

use egui::{
    Align2, Area, Context, Id, InnerResponse, LayerId, Order, Pos2, Rect, Response, Ui, Vec2,
};

use geo::ConvexHull;

use crate::slip;

/// A widget's tooltip opens once the pointer has been on it a moment, however the pointer moves or
/// clicks there, and at once while another has only just closed. egui's own waits for the pointer to
/// rest, and after a click to move again, so it never opened on a widget that is clicked.
pub fn tooltip(response: &Response, add: impl FnOnce(&mut Ui)) {
    let gui = &response.ctx;
    let key = response.id.with("tooltip");
    if !response.contains_pointer() || gui.dragged_id().is_some() || egui::Popup::is_any_open(gui) {
        gui.data_mut(|data| data.remove::<Hovered>(key));
        return;
    }
    let (delay, grace) = {
        let style = gui.global_style();
        (
            style.interaction.tooltip_delay,
            style.interaction.tooltip_grace_time,
        )
    };
    let (now, pass) = (gui.input(|input| input.time), gui.cumulative_pass_nr());
    let since = gui.data_mut(|data| {
        let since = data
            .get_temp::<Hovered>(key)
            .filter(|pointed| pointed.pass + 1 >= pass)
            .map_or(now, |pointed| pointed.since);
        data.insert_temp(key, Hovered { since, pass });
        since
    });
    let follows_another = egui::Tooltip::seconds_since_last_tooltip(gui) < grace;
    if follows_another || now - since >= f64::from(delay) {
        let sheet = egui::Tooltip::next_tooltip_id(gui, response.id);
        let before = gui.memory(|memory| memory.area_rect(sheet));
        let mut tooltip = egui::Tooltip::for_widget(response);
        tooltip.popup = tooltip
            .popup
            .anchor(with_pointer(response))
            .align(egui::RectAlign::TOP_START);
        let shown = tooltip.show(|ui| {
            // egui lays a tooltip out within the size it opened at, which would wrap whatever it
            // comes to say while it stays open.
            let frame = egui::Frame::popup(ui.style()).total_margin().sum().x;
            ui.set_max_width(ui.spacing().tooltip_width - frame);
            add(ui);
        });
        let resized = before
            .zip(shown)
            .is_some_and(|(before, shown)| before.size() != shown.response.rect.size());
        if resized {
            gui.request_discard("a tooltip changed size");
        }
    }
}

pub trait Hinted {
    fn hinted(self, text: &str) -> Self;
}

impl Hinted for Response {
    fn hinted(self, text: &str) -> Response {
        tooltip(&self, |ui| {
            ui.label(text);
        });
        self
    }
}

/// How far the pointer's picture reaches from the point it points with, for tooltips to stand
/// clear of; `None` for the system's own arrow.
pub fn set_pointer_body(gui: &Context, body: Option<Rect>) {
    gui.data_mut(|data| match body {
        Some(body) => {
            data.insert_temp(Id::new(POINTER_BODY), body);
        }
        None => data.remove::<Rect>(Id::new(POINTER_BODY)),
    });
}

const POINTER_BODY: &str = "pointer body";
const SYSTEM_ARROW: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(12.0, 20.0));

/// A tooltip is placed about the widget and the pointer's picture together, so whichever side it
/// takes, the pointer never lies over it.
fn with_pointer(response: &Response) -> Rect {
    let gui = &response.ctx;
    let mut widget = response.interact_rect;
    if let Some(to_global) = gui.layer_transform_to_global(response.layer_id) {
        widget = to_global * widget;
    }
    let body = gui
        .data(|data| data.get_temp::<Rect>(Id::new(POINTER_BODY)))
        .unwrap_or(SYSTEM_ARROW);
    match gui.pointer_latest_pos() {
        Some(at) => widget.union(body.translate(at.to_vec2())),
        None => widget,
    }
}

/// egui places an area by the size it had the pass before, so one held by any point but its top
/// left shows out of place on the pass its size changes, and that pass is drawn again.
pub fn anchored<R>(gui: &Context, area: Area, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let before = gui.memory(|memory| memory.area_rect(area.layer().id));
    let shown = area.show(gui, add);
    if before.is_some_and(|before| before.size() != shown.response.rect.size()) {
        gui.request_discard("an anchored area changed size");
    }
    shown
}

/// Draws what follows the pointer, its top left `from` the pointer, in the frame the pointer is read:
/// never kept on screen or faded in, as egui does to its areas, which would part it from the pointer.
pub fn at_pointer<R>(
    gui: &Context,
    id: Id,
    order: Order,
    from: Vec2,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let at = gui.pointer_latest_pos()?;
    let shown = Area::new(id)
        .order(order)
        .fixed_pos(at + from)
        .interactable(false)
        .constrain(false)
        .fade_in(false)
        .show(gui, add);
    Some(shown.inner)
}

/// A short word at the bottom of the screen on what just happened; `which` tells one toast from the
/// next, so each lays itself out anew rather than keep the size the last one took.
pub fn toast(gui: &Context, text: &str, which: impl Hash) {
    let id = Id::new("toast");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    which.hash(&mut hasher);
    let which = hasher.finish();
    let fresh = gui.data_mut(|data| {
        let laid_out = data.get_temp::<u64>(id);
        data.insert_temp(id, which);
        laid_out != Some(which)
    });
    Area::new(id)
        .sizing_pass(fresh)
        .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
        .order(Order::Foreground)
        .show(gui, |ui| {
            slip::show(ui, |ui| {
                ui.label(text);
            });
        });
}

/// Lays labels that float over the scene apart, so none overlaps another: each, in the order
/// given, keeps its place unless it comes nearer than `apart` to one laid before it, and then
/// rises until it is `apart` above every one it came near. The first keep their places.
pub fn lay_apart(rects: &mut [Rect], apart: f32) {
    for at in 0..rects.len() {
        let (laid, rest) = rects.split_at_mut(at);
        let rect = &mut rest[0];
        let mut below = laid.to_vec();
        loop {
            let (near, clear): (Vec<Rect>, Vec<Rect>) = below
                .into_iter()
                .partition(|other| comes_near(*other, *rect, apart));
            below = clear;
            // Rising over the highest it comes near puts it over all of them, so each one it
            // rises over is passed for good and the rise ends.
            let Some(top) = near.iter().map(|other| other.top()).reduce(f32::min) else {
                break;
            };
            *rect = rect.translate(Vec2::new(0.0, top - apart - rect.bottom()));
        }
    }
}

/// What one set down by `least_penalized_spot` had rather not cover: the outline all of some
/// parts take, each part the convex outline round its points, grown a margin out each way, and
/// counted once where parts overlap, so the room between them stays free. Covering all of the one
/// set down with it costs `weight`.
#[derive(Clone, Debug)]
pub struct CoverPenalty {
    rings: Vec<Ring>,
    weight: f32,
}

impl CoverPenalty {
    pub fn over<Part: IntoIterator<Item = Pos2>>(
        parts: impl IntoIterator<Item = Part>,
        margin: f32,
        weight: f32,
    ) -> CoverPenalty {
        let hulls: Vec<geo::Polygon<f64>> = parts
            .into_iter()
            .map(|part| {
                let grown: Vec<geo::Coord<f64>> = part
                    .into_iter()
                    .flat_map(|point| {
                        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| {
                            geo::coord! {
                                x: f64::from(point.x + x * margin),
                                y: f64::from(point.y + y * margin),
                            }
                        })
                    })
                    .collect();
                geo::MultiPoint::from(grown).convex_hull()
            })
            .collect();
        let rings = geo::unary_union(&hulls)
            .into_iter()
            .flat_map(|polygon| {
                let (exterior, interiors) = polygon.into_inner();
                std::iter::once(Ring::of(&exterior, 1.0))
                    .chain(interiors.iter().map(|hole| Ring::of(hole, -1.0)))
                    .collect::<Vec<_>>()
            })
            .collect();
        CoverPenalty { rings, weight }
    }

    fn covered(&self, rect: Rect) -> f32 {
        self.rings.iter().map(|ring| ring.covered(rect)).sum()
    }
}

/// The spot for a rect of `size` that is penalized least, its penalties added up from how far its
/// centre stands from `toward`, as a share of half the diagonal of `screen`; how much of it covers
/// each of `covers`, as a share of its own area times that one's weight; and how much of it lies off
/// `screen`, as a share of its own area times `OFF_SCREEN_WEIGHT`.
pub fn least_penalized_spot(
    size: Vec2,
    screen: Rect,
    covers: &[CoverPenalty],
    toward: Pos2,
) -> Pos2 {
    let area = size.x * size.y;
    let reach = screen.size().length() / 2.0;
    let penalty = |at: Pos2| {
        let placed = Rect::from_min_size(at, size);
        let off_centre = placed.center().distance(toward) / reach;
        let covering: f32 = covers
            .iter()
            .map(|cover| cover.weight * cover.covered(placed))
            .sum();
        let shown = placed.intersect(screen);
        let off_screen = area - shown.width().max(0.0) * shown.height().max(0.0);
        CENTRE_WEIGHT * off_centre + (covering + OFF_SCREEN_WEIGHT * off_screen) / area
    };
    let least = |from: Pos2, to: Pos2, step: f32| {
        let steps = ((to - from) / step).floor();
        (0..=steps.x as usize)
            .flat_map(|i| (0..=steps.y as usize).map(move |j| (i, j)))
            .map(|(i, j)| from + Vec2::new(i as f32, j as f32) * step)
            .map(|at| (penalty(at), at))
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map_or(from, |(_, at)| at)
    };
    // The penalty changes gently between neighbouring spots, so a coarse sweep of every spot that
    // shows any of the rect lands beside the least penalized one, which a fine sweep around it
    // finds.
    let near = least(screen.min - size, screen.max, COARSE_STEP);
    let around = Vec2::splat(COARSE_STEP);
    least(near - around, near + around, 1.0)
}

const CENTRE_WEIGHT: f32 = 0.5;
const OFF_SCREEN_WEIGHT: f32 = 4.0;
const COARSE_STEP: f32 = 8.0;

/// The layer drawn at `at`, leaving out the root layer egui keeps under the whole screen for panels
/// to catch clicks on.
pub fn layer_at(gui: &Context, at: Pos2) -> Option<LayerId> {
    gui.layer_id_at(at)
        .filter(|&layer| layer != LayerId::background())
}

/// What a layer drew in its last pass, kept to be drawn again after what it showed is gone, so a
/// sheet whose content has ended can still lift off the table.
#[derive(Clone, Default)]
pub struct Afterimage(Vec<egui::epaint::ClippedShape>);

impl Afterimage {
    pub fn of(gui: &Context, layer: LayerId) -> Afterimage {
        Afterimage(gui.graphics(|graphics| {
            graphics
                .get(layer)
                .map(|list| list.all_entries().cloned().collect())
                .unwrap_or_default()
        }))
    }

    pub fn paint(&self, painter: &egui::Painter) {
        for shape in &self.0 {
            painter
                .with_clip_rect(shape.clip_rect)
                .add(shape.shape.clone());
        }
    }
}

fn comes_near(a: Rect, b: Rect, apart: f32) -> bool {
    a.left() - apart < b.right()
        && b.left() - apart < a.right()
        && a.top() - apart < b.bottom()
        && b.top() - apart < a.bottom()
}

#[derive(Clone, Copy)]
struct Hovered {
    since: f64,
    pass: u64,
}

/// A ring of an outline: one that bounds it, counted as `sign` 1, or a hole in it, as -1.
#[derive(Clone, Debug)]
struct Ring {
    points: Vec<Pos2>,
    bounds: Rect,
    area: f32,
    sign: f32,
}

impl Ring {
    fn of(ring: &geo::LineString<f64>, sign: f32) -> Ring {
        let mut points: Vec<Pos2> = ring
            .coords()
            .map(|coord| Pos2::new(coord.x as f32, coord.y as f32))
            .collect();
        points.pop();
        Ring {
            bounds: Rect::from_points(&points),
            area: shoelace(&points),
            points,
            sign,
        }
    }

    fn covered(&self, rect: Rect) -> f32 {
        let area = if !self.bounds.intersects(rect) {
            0.0
        } else if rect.contains_rect(self.bounds) {
            self.area
        } else {
            shoelace(&clipped(&self.points, rect))
        };
        self.sign * area
    }
}

/// The part of the outline through `points` within `rect`, cut by each of its sides in turn:
/// where the outline bends inward the cut may run along an edge both ways, which adds no area.
fn clipped(points: &[Pos2], rect: Rect) -> Vec<Pos2> {
    let sides = [
        (Vec2::X, rect.min.x),
        (-Vec2::X, -rect.max.x),
        (Vec2::Y, rect.min.y),
        (-Vec2::Y, -rect.max.y),
    ];
    sides
        .into_iter()
        .fold(points.to_vec(), |kept, (inward, edge)| {
            let inside = |at: Pos2| at.to_vec2().dot(inward) - edge;
            let mut cut = Vec::with_capacity(kept.len() + 1);
            for (i, &at) in kept.iter().enumerate() {
                let next = kept[(i + 1) % kept.len()];
                let (here, there) = (inside(at), inside(next));
                if here >= 0.0 {
                    cut.push(at);
                }
                if (here >= 0.0) != (there >= 0.0) {
                    cut.push(at + (next - at) * (here / (here - there)));
                }
            }
            cut
        })
}

fn shoelace(outline: &[Pos2]) -> f32 {
    let twice: f32 = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum();
    twice.abs() / 2.0
}
