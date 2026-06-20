use std::sync::Arc;

use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ecs::world::EntityWorldMut;
use bevy_math::Vec2;
use bevy_text::TextColor;
use bevy_ui::{BackgroundColor, BorderColor, Node, UiTransform};
use bevy_view::Element;

use crate::interaction::PointerState;
use crate::motion::{Motion, Opacity, Paint as MotionPaint, Timing, Transform2d};
use crate::theme::{ColorVar, active_theme};

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
    fn resolve(self, entity: &mut EntityWorldMut) -> Color {
        match self {
            Paint::Literal(color) => color,
            Paint::Var(var) => {
                let id = entity.id();
                entity.world_scope(|world| var.resolve(active_theme(world, id)))
            }
        }
    }
}

#[derive(Clone, Default)]
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
}

impl Style {
    pub fn new() -> Style {
        Style::default()
    }

    /// Patch `Node` fields in place without touching runtime-owned fields.
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

    /// Opacity in 0..=1, applied to node and descendants (bevy_ui has no subtree opacity).
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

    /// Overlay style while pointer hovers (and while pressed, since press implies hover).
    pub fn hover(mut self, style: Style) -> Style {
        self.hover = Some(Box::new(style));
        self
    }

    pub fn active(mut self, style: Style) -> Style {
        self.active = Some(Box::new(style));
        self
    }

    pub(crate) fn op<F>(mut self, op: F) -> Style
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        self.ops.push(Arc::new(op));
        self
    }

    /// Append another style; its instant ops run after this one's, tweenable fields win, state overlays compose.
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
        self
    }

    pub fn apply(&self, entity: &mut EntityWorldMut) {
        if (self.hover.is_some() || self.active.is_some()) && entity.get::<PointerState>().is_none()
        {
            entity.insert(PointerState::default());
        }
        let pointer = entity.get::<PointerState>().copied().unwrap_or_default();
        self.for_state(pointer).write(entity);
    }

    fn for_state(&self, pointer: PointerState) -> Style {
        let mut style = Style {
            ops: self.ops.clone(),
            background: self.background,
            border: self.border,
            text: self.text,
            transform: self.transform,
            enter: self.enter,
            opacity: self.opacity,
            enter_opacity: self.enter_opacity,
            spin: self.spin,
            transition: self.transition,
            hover: None,
            active: None,
        };
        if (pointer.hovered || pointer.pressed)
            && let Some(hover) = &self.hover
        {
            style = style.merge((**hover).clone());
        }
        if pointer.pressed
            && let Some(active) = &self.active
        {
            style = style.merge((**active).clone());
        }
        style
    }

    fn write(&self, entity: &mut EntityWorldMut) {
        for op in &self.ops {
            op(entity);
        }

        let background = self.background.map(|paint| paint.resolve(entity));
        let border = self.border.map(|paint| paint.resolve(entity));
        let text = self.text.map(|paint| paint.resolve(entity));

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
                entity.insert(Opacity(opacity));
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
            entity.insert(Opacity(value));
        }
    }
}

fn compose(base: Option<Box<Style>>, over: Option<Box<Style>>) -> Option<Box<Style>> {
    match (base, over) {
        (Some(base), Some(over)) => Some(Box::new(base.merge(*over))),
        (base, over) => over.or(base),
    }
}

/// Attaching a [`Style`] to an element — the single seam between styling and the view tree. The style
/// re-applies every render, so it tracks theme and pointer state live.
pub trait Styled {
    fn style(self, style: Style) -> Self;
}

impl Styled for Element {
    fn style(self, style: Style) -> Element {
        self.attr(move |entity| style.apply(entity))
    }
}

#[derive(Default)]
pub struct Recipe {
    base: Style,
    variants: Vec<Dimension>,
    compounds: Vec<Compound>,
    defaults: Vec<(&'static str, &'static str)>,
}

impl Recipe {
    pub fn new() -> Recipe {
        Recipe::default()
    }

    pub fn base(mut self, style: Style) -> Recipe {
        self.base = style;
        self
    }

    pub fn variant<I>(mut self, name: &'static str, options: I) -> Recipe
    where
        I: IntoIterator<Item = (&'static str, Style)>,
    {
        self.variants.push(Dimension {
            name,
            options: options.into_iter().collect(),
        });
        self
    }

    pub fn compound<I>(mut self, selection: I, style: Style) -> Recipe
    where
        I: IntoIterator<Item = (&'static str, &'static str)>,
    {
        self.compounds.push(Compound {
            selection: selection.into_iter().collect(),
            style,
        });
        self
    }

    pub fn default_variant(mut self, dimension: &'static str, option: &'static str) -> Recipe {
        self.defaults.push((dimension, option));
        self
    }

    /// Merges the base, then each selected variant option (an explicit `selection` entry overriding the
    /// [`default_variant`](Recipe::default_variant)), then every matching compound — later styles win.
    pub fn resolve(&self, selection: &[(&str, &str)]) -> Style {
        let chosen = |dimension: &str| -> Option<&str> {
            selection
                .iter()
                .find(|(name, _)| *name == dimension)
                .map(|(_, option)| *option)
                .or_else(|| {
                    self.defaults
                        .iter()
                        .find(|(name, _)| *name == dimension)
                        .map(|(_, option)| *option)
                })
        };
        let mut style = self.base.clone();
        for dimension in &self.variants {
            let Some(option) = chosen(dimension.name) else {
                continue;
            };
            match dimension.options.iter().find(|(name, _)| *name == option) {
                Some((_, variant)) => style = style.merge(variant.clone()),
                None => debug_assert!(
                    false,
                    "no option `{option}` for variant `{}`",
                    dimension.name
                ),
            }
        }
        for compound in &self.compounds {
            if compound
                .selection
                .iter()
                .all(|(name, option)| chosen(name) == Some(option))
            {
                style = style.merge(compound.style.clone());
            }
        }
        style
    }
}

struct Dimension {
    name: &'static str,
    options: Vec<(&'static str, Style)>,
}

struct Compound {
    selection: Vec<(&'static str, &'static str)>,
    style: Style,
}
