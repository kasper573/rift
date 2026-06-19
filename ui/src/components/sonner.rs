//! `Toaster`: a sonner-style toast stack, modelled on shadcn's sonner. Controlled — the app owns the
//! list of live toasts (their ids, content, and when they appear, expire and leave). The toaster pins
//! the stack to one of eight positions, slides each new toast in from that edge (a fixed 100px straight
//! line, scaling) and out the same way, keeps the newest in front with the older ones scaled down
//! behind, and — while `expanded` (the app sets this on hover) — fans the stack into a list. A toast's
//! content is app-composed; dropping a [`SonnerClose`] into it gives the user a dismiss control.

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::Pickable;
use bevy_ui::{BorderRadius, FlexDirection, Node, PositionType, UiRect, Val};
use bevy_view::{View, context, each, node, provide};

use crate::controlled::OnChange;
use crate::motion::Transform2d;
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, spacing};

const CARD_WIDTH: f32 = 356.0;
const CARD_HEIGHT: f32 = 76.0;
/// The straight-line distance a toast travels on enter and exit — fixed, so the motion reads the same
/// whether the stack is collapsed or expanded.
const TRAVEL: f32 = 100.0;
/// How far each card behind the front is nudged toward the centre when collapsed (the peek).
const PEEK: f32 = 16.0;
const PEEK_SCALE: f32 = 0.05;
const GAP: f32 = 14.0;
const MAX_VISIBLE: usize = 3;
/// The hover region's height — covers the fully fanned-out stack.
const STACK_HEIGHT: f32 = MAX_VISIBLE as f32 * (CARD_HEIGHT + GAP);
const EDGE: f32 = 24.0;

/// Where the stack pins. The abstraction allows for the eight sonner positions; only `BottomRight`
/// is implemented — add the others by extending [`SonnerPosition::place`] and the stack/travel
/// vectors in [`card`].
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SonnerPosition {
    #[default]
    BottomRight,
}

impl SonnerPosition {
    /// The unit vector the stack grows and fans out along — toward the screen centre.
    fn grow(self) -> Vec2 {
        Vec2::new(0.0, -1.0)
    }

    /// The unit vector a toast travels in on enter/exit — straight off the nearest edge.
    fn travel(self) -> Vec2 {
        Vec2::new(0.0, 1.0)
    }

    /// Pins a card's box to the corner of the hover region (the rest is done with transforms).
    fn place(self, node: &mut Node) {
        node.position_type = PositionType::Absolute;
        node.bottom = Val::Px(0.0);
        node.right = Val::Px(0.0);
    }

    /// Pins the hover region over the stack at the viewport edge — wide enough for a card, tall enough to
    /// cover the fanned-out list so moving between cards never leaves it.
    fn place_region(self, node: &mut Node) {
        node.position_type = PositionType::Absolute;
        node.bottom = Val::Px(EDGE);
        node.right = Val::Px(EDGE);
        node.width = Val::Px(CARD_WIDTH);
        node.height = Val::Px(STACK_HEIGHT);
    }
}

/// One live toast: a stable `id` and its app-composed content (title, body, and any [`SonnerClose`]).
#[derive(Clone)]
pub struct Toast {
    id: u64,
    content: Arc<dyn Fn() -> View + Send + Sync>,
    leaving: bool,
}

impl Toast {
    pub fn new<F>(id: u64, content: F) -> Toast
    where
        F: Fn() -> View + Send + Sync + 'static,
    {
        Toast {
            id,
            content: Arc::new(content),
            leaving: false,
        }
    }

    /// Marks the toast as exiting, so the toaster slides it back off the edge.
    pub fn leaving(mut self, leaving: bool) -> Toast {
        self.leaving = leaving;
        self
    }
}

/// The toast stack. Pass the live toasts oldest-first; the last is the newest and sits in front. Set
/// `expanded` (on hover) to fan the stack into a list.
#[derive(Default)]
pub struct Toaster {
    position: Option<SonnerPosition>,
    expanded: bool,
    toasts: Vec<Toast>,
    on_dismiss: Option<OnChange<u64>>,
    on_expand_change: Option<OnChange<bool>>,
}

impl Toaster {
    pub fn position(mut self, position: SonnerPosition) -> Toaster {
        self.position = Some(position);
        self
    }
    pub fn expanded(mut self, expanded: bool) -> Toaster {
        self.expanded = expanded;
        self
    }
    pub fn toasts(mut self, toasts: Vec<Toast>) -> Toaster {
        self.toasts = toasts;
        self
    }
    pub fn on_dismiss<F>(mut self, handler: F) -> Toaster
    where
        F: Fn(&mut World, u64) + Send + Sync + 'static,
    {
        self.on_dismiss = Some(Arc::new(handler));
        self
    }

    /// Requests the expanded state to change — the toaster reports a hover over the stack as `true` and a
    /// leave as `false`, so the app can fan the toasts out into a list while pointed at.
    pub fn on_expand_change<F>(mut self, handler: F) -> Toaster
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_expand_change = Some(Arc::new(handler));
        self
    }
}

/// The id of the toast a piece of content belongs to, shared with any [`SonnerClose`] in it.
#[derive(Clone, Copy)]
struct ToastId(u64);

/// The toaster's dismiss callback, shared with any [`SonnerClose`].
#[derive(Clone)]
struct Dismiss(OnChange<u64>);

/// A dismiss control placed inside a toast's content (typically wrapping a button); clicking it asks the
/// toaster to remove that toast.
#[derive(Default)]
pub struct SonnerClose {
    children: Vec<View>,
}

children_builder!(SonnerClose);

impl From<Toaster> for View {
    fn from(toaster: Toaster) -> View {
        let position = toaster.position.unwrap_or(SonnerPosition::BottomRight);
        let expanded = toaster.expanded;
        let dismiss = toaster.on_dismiss.map(Dismiss);
        // Depth counts only the live toasts (newest = 0), so a leaving toast frees its slot immediately:
        // the toasts behind it ease forward to their new depth while it eases out. Visited newest-first
        // to assign depth, then reversed so the front card paints on top.
        let mut depth_counter = 0;
        let mut stacked: Vec<(Toast, usize)> = toaster
            .toasts
            .into_iter()
            .rev()
            .filter_map(|toast| {
                let depth = depth_counter;
                if !toast.leaving {
                    depth_counter += 1;
                }
                (depth < MAX_VISIBLE || toast.leaving || expanded).then_some((toast, depth))
            })
            .collect();
        stacked.reverse();

        let cards = each(
            move |_| stacked.clone(),
            |(toast, _)| toast.id,
            move |(toast, depth)| card(toast, *depth, position, expanded, dismiss.clone()),
        );

        // A region pinned over the stack receives the hover so the app can expand it; the cards sit
        // inside it. The full-screen layer itself ignores picking so the rest of the app stays clickable.
        let on_expand = toaster.on_expand_change;
        let over = on_expand.clone();
        let out = on_expand;
        let region = node()
            .attr(move |entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    position.place_region(&mut node);
                }
            })
            .on_over_with(move |world, _| {
                if let Some(handler) = &over {
                    handler(world, true);
                }
            })
            .on_out_with(move |world, _| {
                if let Some(handler) = &out {
                    handler(world, false);
                }
            })
            .child(cards);

        node()
            .attr(fill)
            .insert(Pickable::IGNORE)
            .child(region)
            .into()
    }
}

impl From<SonnerClose> for View {
    fn from(close: SonnerClose) -> View {
        node()
            .on_click_with(dismiss_toast)
            .children(close.children)
            .into()
    }
}

fn dismiss_toast(world: &mut World, entity: Entity) {
    let id = context::<ToastId>(world, entity).map(|toast| toast.0);
    let dismiss = context::<Dismiss>(world, entity).map(|d| d.0.clone());
    if let (Some(id), Some(dismiss)) = (id, dismiss) {
        dismiss(world, id);
    }
}

fn fill(entity: &mut bevy_ecs::world::EntityWorldMut) {
    if let Some(mut node) = entity.get_mut::<Node>() {
        node.position_type = PositionType::Absolute;
        node.top = Val::Px(0.0);
        node.left = Val::Px(0.0);
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
    }
}

fn card(
    toast: &Toast,
    depth: usize,
    position: SonnerPosition,
    expanded: bool,
    dismiss: Option<Dismiss>,
) -> View {
    let grow = position.grow();
    let depth = depth as f32;
    // The card's resting translation: its place in the stack (collapsed peek or expanded list).
    let rest = if expanded {
        grow * depth * (CARD_HEIGHT + GAP)
    } else {
        grow * depth * PEEK
    };
    let scale = if expanded {
        1.0
    } else {
        1.0 - depth * PEEK_SCALE
    };
    // Enter from / exit to a point a fixed distance straight off the edge — a straight line through the
    // anchor, scaling down.
    let off = position.travel() * TRAVEL;
    let (translate, scale) = if toast.leaving {
        (rest + off, scale * 0.9)
    } else {
        (rest, scale)
    };

    let style = card_style(position)
        .translate(translate)
        .scale(Vec2::splat(scale))
        .opacity(if toast.leaving { 0.0 } else { 1.0 })
        .enter_opacity(0.0)
        .enter(Transform2d {
            translation: rest + off,
            scale: Vec2::splat(0.9),
            rotation: 0.0,
        })
        .transition(STANDARD_ENTER);

    let mut card = node().style(style);
    if let Some(dismiss) = dismiss {
        card = card.bind(provide(ToastId(toast.id))).bind(provide(dismiss));
    }
    card.child((toast.content)()).into()
}

fn card_style(position: SonnerPosition) -> Style {
    Style::new()
        .background(color::surface_elevated_base)
        .border_color(color::surface_elevated_border)
        .node(move |node| {
            position.place(node);
            node.width = Val::Px(CARD_WIDTH);
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::S);
            node.padding = UiRect::all(Val::Px(spacing::XL));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}
