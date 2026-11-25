//! What floats over the interface: tooltips, what follows the pointer, toasts, and areas that
//! keep their place as they change size.

use std::hash::{Hash, Hasher};

use egui::{
    Align2, Area, Context, Id, InnerResponse, LayerId, Order, Pos2, Rect, Response, Ui, Vec2,
};

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

/// Where a rect of `size` goes within `within` so it covers as little of `kept_clear` as it can,
/// none of it where it fits clear of them all, and of the spots that cover least, the nearest
/// `preferred`. A rect that covers least lies flush with an edge of `within` or `apart` off an
/// edge of one kept clear, along each axis, so those are the spots weighed.
pub fn clear_spot(
    size: Vec2,
    within: Rect,
    kept_clear: &[Rect],
    preferred: Pos2,
    apart: f32,
) -> Pos2 {
    let spots = |low: f32, high: f32, preferred: f32, edges: &mut dyn Iterator<Item = f32>| {
        let last = (high - low).max(0.0) + low;
        let mut spots: Vec<f32> = [low, last, preferred].into_iter().chain(edges).collect();
        for spot in &mut spots {
            *spot = spot.clamp(low, last);
        }
        spots
    };
    let xs = spots(
        within.left(),
        within.right() - size.x,
        preferred.x,
        &mut kept_clear
            .iter()
            .flat_map(|rect| [rect.left() - apart - size.x, rect.right() + apart]),
    );
    let ys = spots(
        within.top(),
        within.bottom() - size.y,
        preferred.y,
        &mut kept_clear
            .iter()
            .flat_map(|rect| [rect.top() - apart - size.y, rect.bottom() + apart]),
    );
    let covered = |at: Pos2| -> f32 {
        let placed = Rect::from_min_size(at, size);
        kept_clear
            .iter()
            .map(|rect| {
                let over = placed.intersect(*rect);
                if over.is_positive() { over.area() } else { 0.0 }
            })
            .sum()
    };
    xs.iter()
        .flat_map(|&x| ys.iter().map(move |&y| Pos2::new(x, y)))
        .min_by(|a, b| {
            covered(*a)
                .total_cmp(&covered(*b))
                .then(a.distance(preferred).total_cmp(&b.distance(preferred)))
        })
        .unwrap_or(preferred)
}

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
