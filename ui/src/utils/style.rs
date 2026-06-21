use std::sync::Arc;

use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityWorldMut;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_text::TextColor;
use bevy_ui::{BackgroundColor, BorderColor, Checked, Node, Pressed, UiTransform};

use crate::motion::{Motion, Paint as MotionPaint, Timing, Transform2d};
use crate::theme::{ColorVar, Theme};
use bevy_opacity::Opacity;

type Op = Arc<dyn Fn(&mut EntityWorldMut) + Send + Sync>;

#[derive(Clone, Copy)]
pub enum Paint {
    Var(ColorVar),
    Literal(Color),
}

impl From<ColorVar> for Paint {
    fn from(var: ColorVar) -> Paint {
        Paint::Var(var)
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Paint {
        Paint::Literal(color)
    }
}

impl Paint {
    fn resolve(self, theme: &Theme) -> Color {
        match self {
            Paint::Literal(color) => color,
            Paint::Var(var) => var.resolve(theme),
        }
    }
}

#[derive(Component, Clone, Default)]
pub struct Style {
    ops: Vec<Op>,
    background: Option<Paint>,
    border: Option<Paint>,
    text: Option<Paint>,
    transform: Option<Transform2d>,
    enter: Option<Transform2d>,
    opacity: Option<f32>,
    enter_opacity: Option<f32>,
    spin: Option<f32>,
    transition: Option<Timing>,
    hover: Option<Box<Style>>,
    active: Option<Box<Style>>,
    checked: Option<Box<Style>>,
}

impl Style {
    pub fn new() -> Style {
        Style::default()
    }

    pub fn node<F>(self, patch: F) -> Style
    where
        F: Fn(&mut Node) + Send + Sync + 'static,
    {
        self.op(move |entity| {
            if let Some(mut node) = entity.get_mut::<Node>() {
                patch(&mut node);
            }
        })
    }

    pub fn insert<B>(self, bundle: B) -> Style
    where
        B: Bundle + Clone,
    {
        self.op(move |entity| {
            entity.insert(bundle.clone());
        })
    }

    pub fn background(mut self, paint: impl Into<Paint>) -> Style {
        self.background = Some(paint.into());
        self
    }

    pub fn border_color(mut self, paint: impl Into<Paint>) -> Style {
        self.border = Some(paint.into());
        self
    }

    pub fn text_color(mut self, paint: impl Into<Paint>) -> Style {
        self.text = Some(paint.into());
        self
    }

    pub fn translate(mut self, offset: Vec2) -> Style {
        self.transform
            .get_or_insert(Transform2d::IDENTITY)
            .translation = offset;
        self
    }

    pub fn scale(mut self, scale: Vec2) -> Style {
        self.transform.get_or_insert(Transform2d::IDENTITY).scale = scale;
        self
    }

    pub fn rotate(mut self, radians: f32) -> Style {
        self.transform.get_or_insert(Transform2d::IDENTITY).rotation = radians;
        self
    }

    pub fn enter(mut self, from: Transform2d) -> Style {
        self.enter = Some(from);
        self
    }

    // bevy_ui has no subtree opacity; `apply_opacity` propagates this down the tree.
    pub fn opacity(mut self, opacity: f32) -> Style {
        self.opacity = Some(opacity);
        self
    }

    pub fn enter_opacity(mut self, from: f32) -> Style {
        self.enter_opacity = Some(from);
        self
    }

    pub fn spin(mut self, radians_per_second: f32) -> Style {
        self.spin = Some(radians_per_second);
        self
    }

    pub fn transition(mut self, timing: Timing) -> Style {
        self.transition = Some(timing);
        self
    }

    pub fn hover(mut self, style: Style) -> Style {
        self.hover = Some(Box::new(style));
        self
    }

    pub fn active(mut self, style: Style) -> Style {
        self.active = Some(Box::new(style));
        self
    }

    pub fn checked(mut self, style: Style) -> Style {
        self.checked = Some(Box::new(style));
        self
    }

    pub(crate) fn op<F>(mut self, op: F) -> Style
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        self.ops.push(Arc::new(op));
        self
    }

    pub fn merge(mut self, other: Style) -> Style {
        self.ops.extend(other.ops);
        self.background = other.background.or(self.background);
        self.border = other.border.or(self.border);
        self.text = other.text.or(self.text);
        self.transform = other.transform.or(self.transform);
        self.enter = other.enter.or(self.enter);
        self.opacity = other.opacity.or(self.opacity);
        self.enter_opacity = other.enter_opacity.or(self.enter_opacity);
        self.spin = other.spin.or(self.spin);
        self.transition = other.transition.or(self.transition);
        self.hover = compose(self.hover, other.hover);
        self.active = compose(self.active, other.active);
        self.checked = compose(self.checked, other.checked);
        self
    }

    fn for_state(&self, hovered: bool, pressed: bool, checked: bool) -> Style {
        let mut style = self.flat();
        // When checked, the `checked` sub-style overrides the base paints AND supplies the
        // hover/active for the checked state — so a checked+hovered control uses e.g. `primary_hover`
        // rather than the base's unchecked hover (which would wash the fill away).
        let (hover, active) = if checked && let Some(checked) = &self.checked {
            style = style.merge(checked.flat());
            (
                checked.hover.clone().or_else(|| self.hover.clone()),
                checked.active.clone().or_else(|| self.active.clone()),
            )
        } else {
            (self.hover.clone(), self.active.clone())
        };
        if (hovered || pressed)
            && let Some(hover) = &hover
        {
            style = style.merge(hover.flat());
        }
        if pressed && let Some(active) = &active {
            style = style.merge(active.flat());
        }
        style
    }

    fn flat(&self) -> Style {
        Style {
            hover: None,
            active: None,
            checked: None,
            ..self.clone()
        }
    }

    fn stateful(&self) -> bool {
        self.hover.is_some() || self.active.is_some() || self.checked.is_some()
    }

    fn write(&self, entity: &mut EntityWorldMut, theme: &Theme) {
        for op in &self.ops {
            op(entity);
        }

        let background = self.background.map(|paint| paint.resolve(theme));
        let border = self.border.map(|paint| paint.resolve(theme));
        let text = self.text.map(|paint| paint.resolve(theme));

        if self.transition.is_none() && self.spin.is_none() {
            if let Some(color) = background {
                entity.insert(BackgroundColor(color));
            }
            if let Some(color) = border {
                entity.insert(BorderColor::all(color));
            }
            if let Some(color) = text {
                entity.insert(TextColor(color));
            }
            if let Some(transform) = self.transform {
                entity.insert(transform.to_ui());
            }
            if let Some(opacity) = self.opacity {
                entity.insert(Opacity::new(opacity));
            }
            return;
        }

        if entity.get::<Motion>().is_none() {
            entity.insert(Motion::default());
        }
        let timing = self.transition;
        let transformed = self.transform.is_some() || self.enter.is_some();
        // Aim each channel and read back this frame's value, so the paint is right immediately (no
        // first-frame flash) while `advance_motion` keeps easing it on later frames.
        let (background, border, text, transform, opacity) = {
            let mut motion = entity.get_mut::<Motion>().expect("just inserted");
            let background =
                background.map(|color| motion.aim_color(MotionPaint::Background, color, timing));
            let border = border.map(|color| motion.aim_color(MotionPaint::Border, color, timing));
            let text = text.map(|color| motion.aim_color(MotionPaint::Text, color, timing));
            let target = self.transform.unwrap_or(Transform2d::IDENTITY);
            let transform = transformed
                .then(|| motion.aim_transform(self.enter.unwrap_or(target), target, timing));
            let opacity = self.opacity.map(|target| {
                motion.aim_opacity(self.enter_opacity.unwrap_or(target), target, timing)
            });
            if let Some(speed) = self.spin {
                motion.set_spin(speed);
            }
            (background, border, text, transform, opacity)
        };
        if let Some(color) = background {
            entity.insert(BackgroundColor(color));
        }
        if let Some(color) = border {
            entity.insert(BorderColor::all(color));
        }
        if let Some(color) = text {
            entity.insert(TextColor(color));
        }
        if let Some(transform) = transform {
            entity.insert(transform.to_ui());
        } else if self.spin.is_some() && entity.get::<UiTransform>().is_none() {
            entity.insert(UiTransform::default());
        }
        if let Some(value) = opacity {
            entity.insert(Opacity::new(value));
        }
    }
}

fn compose(base: Option<Box<Style>>, over: Option<Box<Style>>) -> Option<Box<Style>> {
    match (base, over) {
        (Some(base), Some(over)) => Some(Box::new(base.merge(*over))),
        (base, over) => over.or(base),
    }
}

pub(crate) fn apply_styles(world: &mut World) {
    let theme = *world.resource::<Theme>();
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Style>>()
        .iter(world)
        .collect();
    for entity in entities {
        let hovered = world.get::<Hovered>(entity).is_some_and(Hovered::get);
        let pressed = world.get::<Pressed>(entity).is_some();
        let checked = world.get::<Checked>(entity).is_some();
        let Some(style) = world.get::<Style>(entity).cloned() else {
            continue;
        };
        let mut entity = world.entity_mut(entity);
        // Track for picking so a hover change repaints the entity.
        if style.stateful() && entity.get::<Hovered>().is_none() {
            entity.insert(Hovered(false));
        }
        style
            .for_state(hovered, pressed, checked)
            .write(&mut entity, &theme);
    }
}
