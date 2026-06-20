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
const TRAVEL: f32 = 100.0;
const PEEK: f32 = 16.0;
const PEEK_SCALE: f32 = 0.05;
const GAP: f32 = 14.0;
const MAX_VISIBLE: usize = 3;
const STACK_HEIGHT: f32 = MAX_VISIBLE as f32 * (CARD_HEIGHT + GAP);
const EDGE: f32 = 24.0;

/// Where the stack pins. Only `BottomRight` implemented; extend [`SonnerPosition::place`] for others.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SonnerPosition {
    #[default]
    BottomRight,
}

impl SonnerPosition {
    fn grow(self) -> Vec2 {
        Vec2::new(0.0, -1.0)
    }

    fn travel(self) -> Vec2 {
        Vec2::new(0.0, 1.0)
    }

    fn place(self, node: &mut Node) {
        node.position_type = PositionType::Absolute;
        node.bottom = Val::Px(0.0);
        node.right = Val::Px(0.0);
    }

    fn place_region(self, node: &mut Node) {
        node.position_type = PositionType::Absolute;
        node.bottom = Val::Px(EDGE);
        node.right = Val::Px(EDGE);
        node.width = Val::Px(CARD_WIDTH);
        node.height = Val::Px(STACK_HEIGHT);
    }
}

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

    pub fn leaving(mut self, leaving: bool) -> Toast {
        self.leaving = leaving;
        self
    }
}

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

    pub fn on_expand_change<F>(mut self, handler: F) -> Toaster
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_expand_change = Some(Arc::new(handler));
        self
    }
}

#[derive(Clone, Copy)]
struct ToastId(u64);

#[derive(Clone)]
struct Dismiss(OnChange<u64>);

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
        // Depth counts only live toasts (newest = 0), so leaving toasts free slots immediately.
        // Visit newest-first to assign depth; reverse so front card paints on top.
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

        // Hover region pinned over the stack; cards sit inside. Full-screen layer ignores picking.
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
