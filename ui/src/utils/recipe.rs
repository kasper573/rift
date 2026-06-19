//! The styling core. A [`Style`] is the one appearance primitive every component reaches for: it
//! collects instant layout (`Node` patches and component inserts), the tweenable paints
//! (`background`/`border`/`text`), a [transform](Style::translate), optional hover/press
//! [state](Style::hover) overlays, and an optional [`transition`](Style::transition) — then `apply`s
//! itself to one element, resolving everything in one place: theme variables against the active
//! [theme](crate::theme), hover/press against the element's [`PointerState`], and easing through
//! [`Motion`](crate::motion). Attach one with [`Styled::style`].
//!
//! A [`Recipe`] is the only thing layered on top: it composes Styles by variant — a `bevy_ui` take on
//! [vanilla-extract's recipes](https://vanilla-extract.style/documentation/packages/recipes/) — and
//! `resolve`s a (partial) selection into a single `Style`. Nothing else styles elements directly, so
//! adding a paint, a state, or a transition is the same gesture everywhere.

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

/// One instant styling operation: a `Node` patch or a component insert, run against the entity.
type Op = Arc<dyn Fn(&mut EntityWorldMut) + Send + Sync>;

/// A color source for a tweenable paint: a [theme variable](ColorVar) resolved against the active
/// theme, or a literal color.
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

/// One element's complete appearance: instant layout, tweenable paints, a transform, hover/press
/// overlays, and a transition. Cheaply cloned and [merged](Style::merge); a later tweenable field wins
/// over an earlier one, instant operations accumulate in order, and the state overlays compose.
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

    /// Patches `Node` fields in place, leaving the rest — and any runtime-owned fields like a
    /// draggable's `left`/`top` — untouched.
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

    /// Inserts a component immediately (a marker, or a paint the recipe doesn't tween itself).
    pub fn insert<B>(self, bundle: B) -> Style
    where
        B: Bundle + Clone,
    {
        self.op(move |entity| {
            entity.insert(bundle.clone());
        })
    }

    /// The background paint — a [`ColorVar`] or a literal [`Color`] (e.g. `Color::NONE`).
    pub fn background(mut self, paint: impl Into<Paint>) -> Style {
        self.background = Some(paint.into());
        self
    }

    /// The border paint (set a `Node.border` width to make it show).
    pub fn border_color(mut self, paint: impl Into<Paint>) -> Style {
        self.border = Some(paint.into());
        self
    }

    /// A text element's foreground paint.
    pub fn text_color(mut self, paint: impl Into<Paint>) -> Style {
        self.text = Some(paint.into());
        self
    }

    /// Offsets the element by a logical-pixel translation.
    pub fn translate(mut self, offset: Vec2) -> Style {
        self.transform
            .get_or_insert(Transform2d::IDENTITY)
            .translation = offset;
        self
    }

    /// Scales the element (1.0 = natural size).
    pub fn scale(mut self, scale: Vec2) -> Style {
        self.transform.get_or_insert(Transform2d::IDENTITY).scale = scale;
        self
    }

    /// Rotates the element clockwise by `radians`.
    pub fn rotate(mut self, radians: f32) -> Style {
        self.transform.get_or_insert(Transform2d::IDENTITY).rotation = radians;
        self
    }

    /// The transform the element animates *from* when it first mounts — an entrance (e.g. a smaller
    /// scale, or an offset, that eases to rest). Needs a [`transition`](Style::transition).
    pub fn enter(mut self, from: Transform2d) -> Style {
        self.enter = Some(from);
        self
    }

    /// The element's opacity in `0..=1`, faded into this node *and its descendants* — bevy_ui has no
    /// subtree opacity, so this provides it (the basis for overlay cross-fades).
    pub fn opacity(mut self, opacity: f32) -> Style {
        self.opacity = Some(opacity);
        self
    }

    /// The opacity the element animates *from* on mount (e.g. `0.0` to fade in). Needs a
    /// [`transition`](Style::transition).
    pub fn enter_opacity(mut self, from: f32) -> Style {
        self.enter_opacity = Some(from);
        self
    }

    /// Spins the element continuously, `radians` per second (composed over any transform).
    pub fn spin(mut self, radians_per_second: f32) -> Style {
        self.spin = Some(radians_per_second);
        self
    }

    /// Eases this style's tweenable paints and transform over `timing` instead of snapping — including
    /// the changes its [`hover`](Style::hover)/[`active`](Style::active) overlays bring.
    pub fn transition(mut self, timing: Timing) -> Style {
        self.transition = Some(timing);
        self
    }

    /// Overlays `style` while the pointer is over the element (and while pressed, since a press implies
    /// hover) — the analog of CSS `:hover`. Layered over the base; pair with a [`transition`].
    pub fn hover(mut self, style: Style) -> Style {
        self.hover = Some(Box::new(style));
        self
    }

    /// Overlays `style` while the element is pressed — the analog of CSS `:active`.
    pub fn active(mut self, style: Style) -> Style {
        self.active = Some(Box::new(style));
        self
    }

    /// Pushes an arbitrary instant operation against the element's entity.
    pub(crate) fn op<F>(mut self, op: F) -> Style
    where
        F: Fn(&mut EntityWorldMut) + Send + Sync + 'static,
    {
        self.ops.push(Arc::new(op));
        self
    }

    /// Appends `other`: its instant ops run after this style's, each tweenable field it sets wins, and
    /// their state overlays compose.
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

    /// Applies the style to `entity`: resolves hover/press against the element's [`PointerState`] (which
    /// it installs so the observers track it), then writes the resulting look.
    pub fn apply(&self, entity: &mut EntityWorldMut) {
        if (self.hover.is_some() || self.active.is_some()) && entity.get::<PointerState>().is_none()
        {
            entity.insert(PointerState::default());
        }
        let pointer = entity.get::<PointerState>().copied().unwrap_or_default();
        self.for_state(pointer).write(entity);
    }

    /// The flattened style for `pointer`: this base with its hover overlay merged while hovered/pressed
    /// and its active overlay merged while pressed (so a press shows both, as CSS `:active` implies
    /// `:hover`).
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

    /// Writes this (already state-flattened) style: instant ops, then the paints/transform — directly
    /// when there's no transition, or aimed at via [`Motion`] so they ease over time.
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

/// A composed style: a base plus named variant dimensions, optional compound overrides, and default
/// selections. [`resolve`](Recipe::resolve) turns a partial selection into the final [`Style`].
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

    /// The style applied before any variant — the component's default look.
    pub fn base(mut self, style: Style) -> Recipe {
        self.base = style;
        self
    }

    /// Declares a variant dimension `name` with its `(option, style)` choices.
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

    /// A style applied only when every `(dimension, option)` pair in `selection` is chosen — the
    /// equivalent of vanilla-extract's `compoundVariants`.
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

    /// The option chosen for `dimension` when a resolved selection omits it.
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
