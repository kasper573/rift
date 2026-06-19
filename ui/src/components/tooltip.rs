//! `Tooltip`: a hover-triggered, non-interactive floating label. Controlled — `open` is a prop; the
//! trigger records a hover, and once the [`TooltipProvider`]'s delay has elapsed [`open_due_tooltips`]
//! requests `open=true` through `on_open_change`. Leaving the trigger requests `false`. A re-hover within
//! the provider's skip window opens immediately.

use std::sync::Arc;
use std::time::Duration;

use bevy_ecs::prelude::*;
use bevy_time::Time;
use bevy_view::{InstanceId, PortalKind, View, context, node, provide};

use crate::controlled::{OnChange, controlled, noop};
use crate::{Align, Overlays, Side, overlay_root, register_anchor, set_overlay};

/// The reserved portal destination for tooltips; place a [`TooltipOutlet`] where they should paint.
const TOOLTIP_OUTLET: PortalKind = PortalKind(0x70_0000_0000_0070);

/// Shared tooltip timing, supplied by a [`TooltipProvider`] and read via context.
#[derive(Clone, Copy)]
pub struct TooltipConfig {
    pub delay: Duration,
    pub skip_delay: Duration,
}

impl Default for TooltipConfig {
    fn default() -> TooltipConfig {
        TooltipConfig {
            delay: Duration::from_millis(400),
            skip_delay: Duration::from_millis(300),
        }
    }
}

/// A hover-triggered, non-interactive tooltip. Wrap a [`TooltipTrigger`] and a [`TooltipContent`].
#[derive(Default)]
pub struct Tooltip {
    open: bool,
    on_open_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl Tooltip {
    pub fn open(mut self, open: bool) -> Tooltip {
        self.open = open;
        self
    }

    pub fn on_open_change<F>(mut self, handler: F) -> Tooltip
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_open_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Tooltip);

/// Shares tooltip timing (`delay`, `skip_delay`) with the tooltips it wraps.
#[derive(Default)]
pub struct TooltipProvider {
    config: TooltipConfig,
    children: Vec<View>,
}

impl TooltipProvider {
    pub fn delay(mut self, delay: Duration) -> TooltipProvider {
        self.config.delay = delay;
        self
    }
    pub fn skip_delay(mut self, skip_delay: Duration) -> TooltipProvider {
        self.config.skip_delay = skip_delay;
        self
    }
}

children_builder!(TooltipProvider);

/// Wraps the element a tooltip describes; hovering it opens the tooltip after the delay.
#[derive(Default)]
pub struct TooltipTrigger {
    children: Vec<View>,
}

children_builder!(TooltipTrigger);

/// The floating tooltip body.
#[derive(Default)]
pub struct TooltipContent {
    side: Side,
    align: Align,
    offset: f32,
    children: Vec<View>,
}

crate::popper::placement_props!(TooltipContent);
children_builder!(TooltipContent);

/// Where tooltips render.
#[derive(Default)]
pub struct TooltipOutlet;

impl From<Tooltip> for View {
    fn from(tooltip: Tooltip) -> View {
        overlay_root(
            tooltip.open,
            tooltip.on_open_change.unwrap_or_else(noop),
            tooltip.children,
        )
    }
}

impl From<TooltipProvider> for View {
    fn from(provider: TooltipProvider) -> View {
        node()
            .bind(provide(provider.config))
            .children(provider.children)
            .into()
    }
}

impl From<TooltipTrigger> for View {
    fn from(trigger: TooltipTrigger) -> View {
        node()
            .on_over_with(hover)
            .on_out_with(leave)
            .children(trigger.children)
            .into()
    }
}

impl From<TooltipContent> for View {
    fn from(content: TooltipContent) -> View {
        // No appearance: the tooltip just floats whatever is composed inside it (bare text, or a `Card`
        // for a surface). It ignores picking (click-through) and isn't dismissed by an outside press.
        crate::popper::content(
            TOOLTIP_OUTLET,
            content.side,
            content.align,
            content.offset,
            true,
            false,
            content.children,
        )
    }
}

impl From<TooltipOutlet> for View {
    fn from(_: TooltipOutlet) -> View {
        crate::overlay_outlet(TOOLTIP_OUTLET).into()
    }
}

/// Opens each hovered tooltip whose delay has elapsed, requesting `open=true` once (then forgetting the
/// hover, so it fires a single time). Runs every frame; available for scripted activation in tests.
pub fn open_due_tooltips(world: &mut World) {
    let Some(now) = world.get_resource::<Time>().map(|time| time.elapsed()) else {
        return;
    };
    let due: Vec<InstanceId> = world
        .resource::<Overlays>()
        .states
        .iter()
        .filter_map(|(id, overlay)| {
            let since = overlay.hover_at?;
            (now.saturating_sub(since) >= overlay.delay).then_some(*id)
        })
        .collect();
    for id in due {
        let open = {
            let mut overlays = world.resource_mut::<Overlays>();
            match overlays.states.get_mut(&id) {
                Some(overlay) => {
                    overlay.hover_at = None;
                    overlay.on_open_change.clone()
                }
                None => None,
            }
        };
        if let Some(open) = open {
            open(world, true);
        }
    }
}

fn hover(world: &mut World, entity: Entity) {
    register_anchor(world, entity);
    let config = context::<TooltipConfig>(world, entity)
        .copied()
        .unwrap_or_default();
    let now = elapsed(world);
    let skip = world
        .resource::<Overlays>()
        .last_tooltip_closed
        .is_some_and(|last| now.saturating_sub(last) < config.skip_delay);
    if skip {
        set_overlay(world, entity, |overlay| overlay.hover_at = None);
        request(world, entity, true);
    } else {
        set_overlay(world, entity, move |overlay| {
            overlay.hover_at = Some(now);
            overlay.delay = config.delay;
        });
    }
}

fn leave(world: &mut World, entity: Entity) {
    let now = elapsed(world);
    let was_open = controlled::<bool>(world, entity).is_some_and(|control| control.value);
    set_overlay(world, entity, |overlay| overlay.hover_at = None);
    if was_open && let Some(mut overlays) = world.get_resource_mut::<Overlays>() {
        overlays.last_tooltip_closed = Some(now);
    }
    request(world, entity, false);
}

fn request(world: &mut World, entity: Entity, open: bool) {
    if let Some(control) = controlled::<bool>(world, entity) {
        control.request(world, open);
    }
}

fn elapsed(world: &World) -> Duration {
    world
        .get_resource::<Time>()
        .map(|time| time.elapsed())
        .unwrap_or_default()
}
