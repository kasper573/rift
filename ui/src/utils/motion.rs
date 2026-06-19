//! Time-based motion: tweens and looping animations, the analog of CSS
//! `transition`s and keyframe animations. A [`Motion`] component holds animated channels — the paint
//! colors, the [`UiTransform`], and an optional continuous spin — each easing from where it is toward a
//! target that styling sets every frame. [`advance_motion`] integrates them against real time and
//! writes the underlying `bevy_ui` components, so a recipe only ever sets *targets* and the smoothing
//! is automatic. Durations and easings come from the library's motion tokens.

use std::time::Duration;

use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Alpha, Color, Mix};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::{Rot2, Vec2};
use bevy_text::TextColor;
use bevy_time::Time;
use bevy_ui::widget::ImageNode;
use bevy_ui::{BackgroundColor, BorderColor, Node, UiTransform, Val2};

/// A cubic-bezier easing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Easing {
    Linear,
    Standard,
    StandardDecelerate,
    StandardAccelerate,
    Emphasized,
    EmphasizedDecelerate,
    EmphasizedAccelerate,
}

impl Easing {
    /// The `(x1, y1, x2, y2)` control points of the unit cubic bezier (`P0=(0,0)`, `P3=(1,1)`).
    const fn control(self) -> [f32; 4] {
        match self {
            Easing::Linear => [0.0, 0.0, 1.0, 1.0],
            Easing::Standard | Easing::Emphasized => [0.2, 0.0, 0.0, 1.0],
            Easing::StandardDecelerate => [0.0, 0.0, 0.0, 1.0],
            Easing::StandardAccelerate => [0.3, 0.0, 1.0, 1.0],
            Easing::EmphasizedDecelerate => [0.05, 0.7, 0.1, 1.0],
            Easing::EmphasizedAccelerate => [0.3, 0.0, 0.8, 0.15],
        }
    }

    /// Eased output for linear progress `t` in `0..=1`.
    fn eval(self, t: f32) -> f32 {
        let [x1, y1, x2, y2] = self.control();
        // Solve the bezier's x(s) = t for the parameter s (Newton, then a couple of bisection
        // fallbacks), then read y(s). Good enough for per-frame UI easing.
        let bezier = |a: f32, b: f32, s: f32| {
            let u = 1.0 - s;
            3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
        };
        let mut s = t;
        for _ in 0..6 {
            let x = bezier(x1, x2, s) - t;
            let dx = 3.0 * (1.0 - s) * (1.0 - s) * x1
                + 6.0 * (1.0 - s) * s * (x2 - x1)
                + 3.0 * s * s * (1.0 - x2);
            if dx.abs() < 1e-5 {
                break;
            }
            s = (s - x / dx).clamp(0.0, 1.0);
        }
        bezier(y1, y2, s)
    }
}

/// A duration paired with an easing — a `Transition`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Timing {
    pub duration: Duration,
    pub easing: Easing,
}

impl Timing {
    pub const fn new(millis: u64, easing: Easing) -> Timing {
        Timing {
            duration: Duration::from_millis(millis),
            easing,
        }
    }
}

/// The named transitions. Reach for [`STANDARD_ENTER`] for the
/// ubiquitous hover/press appearance change; the rest cover overlay enter/exit motion.
pub mod transition {
    use super::{Easing, Timing};

    /// `standard.enter` — the default appearance transition (hover/press color changes).
    pub const STANDARD_ENTER: Timing = Timing::new(250, Easing::StandardDecelerate);
    /// `standard.exit`.
    pub const STANDARD_EXIT: Timing = Timing::new(200, Easing::StandardAccelerate);
    /// `emphasized.enter` — overlay entrances.
    pub const EMPHASIZED_ENTER: Timing = Timing::new(400, Easing::EmphasizedDecelerate);
    /// `emphasized.exit`.
    pub const EMPHASIZED_EXIT: Timing = Timing::new(200, Easing::EmphasizedAccelerate);
}

/// A 2D UI transform as plain numbers (logical-pixel translation, unitless scale, radian rotation) so
/// it interpolates cleanly; written out as a [`UiTransform`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform2d {
    pub translation: Vec2,
    pub scale: Vec2,
    pub rotation: f32,
}

impl Transform2d {
    pub const IDENTITY: Transform2d = Transform2d {
        translation: Vec2::ZERO,
        scale: Vec2::ONE,
        rotation: 0.0,
    };

    fn lerp(self, other: Transform2d, t: f32) -> Transform2d {
        Transform2d {
            translation: self.translation.lerp(other.translation, t),
            scale: self.scale.lerp(other.scale, t),
            rotation: self.rotation + (other.rotation - self.rotation) * t,
        }
    }

    pub(crate) fn to_ui(self) -> UiTransform {
        UiTransform {
            translation: Val2::px(self.translation.x, self.translation.y),
            scale: self.scale,
            rotation: Rot2::radians(self.rotation),
        }
    }
}

/// One animated value easing from `from` to `target` over `timing`.
#[derive(Clone, Copy)]
struct Tween<T> {
    from: T,
    target: T,
    elapsed: Duration,
    timing: Timing,
}

impl<T: Copy + PartialEq> Tween<T> {
    fn settled(value: T) -> Tween<T> {
        Tween {
            from: value,
            target: value,
            elapsed: Duration::ZERO,
            timing: Timing::new(0, Easing::Linear),
        }
    }

    /// Aims the tween at `target`: snaps when `timing` is `None` or already there, otherwise restarts
    /// from the current eased value so an in-flight tween redirects smoothly.
    fn retarget(&mut self, current: T, target: T, timing: Option<Timing>) {
        if self.target == target {
            return;
        }
        match timing {
            Some(timing) => {
                self.from = current;
                self.target = target;
                self.elapsed = Duration::ZERO;
                self.timing = timing;
            }
            None => {
                self.from = target;
                self.target = target;
                self.elapsed = self.timing.duration; // fully settled
            }
        }
    }

    fn fraction(&self) -> f32 {
        if self.timing.duration.is_zero() {
            return 1.0;
        }
        let raw = (self.elapsed.as_secs_f32() / self.timing.duration.as_secs_f32()).clamp(0.0, 1.0);
        self.timing.easing.eval(raw)
    }
}

/// Per-element animation state. Styling sets each channel's target every frame; [`advance_motion`]
/// eases the stored value toward it and writes the matching `bevy_ui` component. An absent channel is
/// left to whatever set the component directly.
#[derive(Component, Default)]
pub struct Motion {
    background: Option<Tween<Color>>,
    border: Option<Tween<Color>>,
    text: Option<Tween<Color>>,
    transform: Option<Tween<Transform2d>>,
    opacity: Option<Tween<f32>>,
    /// Continuous clockwise spin in radians per second, composed on top of `transform`.
    spin: Option<f32>,
    spun: f32,
}

/// A node's own opacity in `0..=1`, written by [`advance_motion`] from the opacity tween and multiplied
/// into the alpha of this node and its descendants by [`apply_opacity`] — bevy_ui has no built-in
/// subtree opacity, so this gives one (the basis for overlay show/hide cross-fades).
#[derive(Component, Clone, Copy)]
pub(crate) struct Opacity(pub f32);

/// Which paint a color channel drives.
#[derive(Clone, Copy)]
pub(crate) enum Paint {
    Background,
    Border,
    Text,
}

impl Motion {
    /// Aims a color channel at `target`, returning the value to show this frame: a new channel settles
    /// at `target` (no mount fade-in), while an existing one redirects from its live value and eases.
    pub(crate) fn aim_color(
        &mut self,
        paint: Paint,
        target: Color,
        timing: Option<Timing>,
    ) -> Color {
        let slot = match paint {
            Paint::Background => &mut self.background,
            Paint::Border => &mut self.border,
            Paint::Text => &mut self.text,
        };
        match slot {
            Some(tween) => {
                let current = value_of(tween);
                tween.retarget(current, target, timing);
                current
            }
            None => {
                *slot = Some(Tween::settled(target));
                target
            }
        }
    }

    /// Aims the transform at `target`; a new channel eases from `enter` (its entrance start, usually
    /// equal to `target` for a plain element). Returns the value to show this frame.
    pub(crate) fn aim_transform(
        &mut self,
        enter: Transform2d,
        target: Transform2d,
        timing: Option<Timing>,
    ) -> Transform2d {
        match &mut self.transform {
            Some(tween) => {
                let current = value_of(tween);
                tween.retarget(current, target, timing);
                current
            }
            None => {
                let tween = match timing {
                    Some(timing) => Tween {
                        from: enter,
                        target,
                        elapsed: Duration::ZERO,
                        timing,
                    },
                    None => Tween::settled(target),
                };
                let current = value_of(&tween);
                self.transform = Some(tween);
                current
            }
        }
    }

    /// Aims the opacity at `target`; a new channel eases from `enter` (its mount start). Returns the
    /// value to show this frame.
    pub(crate) fn aim_opacity(&mut self, enter: f32, target: f32, timing: Option<Timing>) -> f32 {
        match &mut self.opacity {
            Some(tween) => {
                let current = value_of(tween);
                tween.retarget(current, target, timing);
                current
            }
            None => {
                let tween = match timing {
                    Some(timing) => Tween {
                        from: enter,
                        target,
                        elapsed: Duration::ZERO,
                        timing,
                    },
                    None => Tween::settled(target),
                };
                let current = value_of(&tween);
                self.opacity = Some(tween);
                current
            }
        }
    }

    pub(crate) fn set_spin(&mut self, radians_per_second: f32) {
        self.spin = Some(radians_per_second);
    }
}

/// The paints and transform a [`Motion`] writes into — all optional, since an element animates only
/// the channels its styling set.
type Painted = (
    &'static mut Motion,
    Option<&'static mut BackgroundColor>,
    Option<&'static mut BorderColor>,
    Option<&'static mut TextColor>,
    Option<&'static mut UiTransform>,
    Option<&'static mut Opacity>,
);

/// Eases every channel toward its target and writes the underlying paints/transform.
pub(crate) fn advance_motion(time: Res<Time>, mut motions: Query<Painted>) {
    let dt = time.delta();
    for (mut motion, background, border, text, transform, opacity) in &mut motions {
        if let (Some(tween), Some(mut paint)) = (motion.background.as_mut(), background) {
            paint.0 = step_color(tween, dt);
        }
        if let (Some(tween), Some(mut paint)) = (motion.border.as_mut(), border) {
            *paint = BorderColor::all(step_color(tween, dt));
        }
        if let (Some(tween), Some(mut paint)) = (motion.text.as_mut(), text) {
            paint.0 = step_color(tween, dt);
        }
        if let (Some(tween), Some(mut opacity)) = (motion.opacity.as_mut(), opacity) {
            opacity.0 = step_f32(tween, dt);
        }

        let spin = motion.spin.map(|speed| {
            motion.spun =
                (motion.spun + speed * dt.as_secs_f32()).rem_euclid(std::f32::consts::TAU);
            motion.spun
        });
        if let Some(mut ui_transform) = transform {
            if let Some(tween) = motion.transform.as_mut() {
                let mut value = step_transform(tween, dt);
                if let Some(spin) = spin {
                    value.rotation += spin;
                }
                *ui_transform = value.to_ui();
            } else if let Some(spin) = spin {
                ui_transform.rotation = Rot2::radians(spin);
            }
        }
    }
}

fn step_color(tween: &mut Tween<Color>, dt: Duration) -> Color {
    tween.elapsed = tween.elapsed.saturating_add(dt);
    tween.from.mix(&tween.target, tween.fraction())
}

fn step_transform(tween: &mut Tween<Transform2d>, dt: Duration) -> Transform2d {
    tween.elapsed = tween.elapsed.saturating_add(dt);
    tween.from.lerp(tween.target, tween.fraction())
}

fn step_f32(tween: &mut Tween<f32>, dt: Duration) -> f32 {
    tween.elapsed = tween.elapsed.saturating_add(dt);
    let t = tween.fraction();
    tween.from + (tween.target - tween.from) * t
}

/// Propagates each [`Opacity`] down the UI tree, multiplying the cumulative value into the alpha of every
/// node's paints. Runs after [`advance_motion`] (which sets the base colors and the opacity value) and is
/// re-applied each frame, so it composes with the per-frame styling without permanently dimming anything.
/// The alpha-bearing paints [`apply_opacity`] dims on a node — all optional, since a node has only the
/// ones its styling set.
type Tinted = (
    Option<&'static mut BackgroundColor>,
    Option<&'static mut BorderColor>,
    Option<&'static mut TextColor>,
    Option<&'static mut ImageNode>,
);

fn apply_opacity(
    roots: Query<Entity, (With<Node>, Without<ChildOf>)>,
    children: Query<&Children>,
    opacities: Query<&Opacity>,
    mut paints: Query<Tinted>,
) {
    // Nothing is transparent — skip the tree walk entirely (the common case).
    if opacities.is_empty() {
        return;
    }
    let mut stack: Vec<(Entity, f32)> = roots.iter().map(|entity| (entity, 1.0)).collect();
    while let Some((entity, parent_alpha)) = stack.pop() {
        let own = opacities.get(entity).map_or(1.0, |opacity| opacity.0);
        let alpha = parent_alpha * own;
        if alpha < 0.999
            && let Ok((background, border, text, image)) = paints.get_mut(entity)
        {
            if let Some(mut background) = background {
                background.0 = fade(background.0, alpha);
            }
            if let Some(mut border) = border {
                border.top = fade(border.top, alpha);
                border.right = fade(border.right, alpha);
                border.bottom = fade(border.bottom, alpha);
                border.left = fade(border.left, alpha);
            }
            if let Some(mut text) = text {
                text.0 = fade(text.0, alpha);
            }
            if let Some(mut image) = image {
                image.color = fade(image.color, alpha);
            }
        }
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter().map(|kid| (kid, alpha)));
        }
    }
}

fn fade(color: Color, alpha: f32) -> Color {
    color.with_alpha(color.alpha() * alpha)
}

/// The current eased value of a tween mid-flight.
fn value_of<T: Lerp + Copy + PartialEq>(tween: &Tween<T>) -> T {
    tween.from.lerp_to(tween.target, tween.fraction())
}

/// Minimal interpolation so [`value_of`] reads an in-flight tween generically.
trait Lerp {
    fn lerp_to(self, other: Self, t: f32) -> Self;
}
impl Lerp for Color {
    fn lerp_to(self, other: Self, t: f32) -> Self {
        self.mix(&other, t)
    }
}
impl Lerp for Transform2d {
    fn lerp_to(self, other: Self, t: f32) -> Self {
        self.lerp(other, t)
    }
}
impl Lerp for f32 {
    fn lerp_to(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

/// Registers the motion integrator. Added by [`UiPlugin`](crate::UiPlugin).
pub(crate) struct MotionPlugin;

impl Plugin for MotionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, (advance_motion, apply_opacity).chain());
    }
}
