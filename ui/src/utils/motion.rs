use std::time::Duration;

use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::{Color, Mix};
use bevy_ecs::prelude::*;
use bevy_math::cubic_splines::CubicSegment;
use bevy_math::{Rot2, Vec2};
use bevy_text::TextColor;
use bevy_time::Time;
use bevy_ui::{BackgroundColor, BorderColor, UiTransform, Val2};

use bevy_opacity::{Opacity, OpacitySet};

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

    fn eval(self, t: f32) -> f32 {
        let [x1, y1, x2, y2] = self.control();
        CubicSegment::new_bezier_easing([x1, y1], [x2, y2]).ease(t)
    }
}

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

pub mod transition {
    use super::{Easing, Timing};

    pub const STANDARD_ENTER: Timing = Timing::new(250, Easing::StandardDecelerate);
    pub const STANDARD_EXIT: Timing = Timing::new(200, Easing::StandardAccelerate);
    pub const EMPHASIZED_ENTER: Timing = Timing::new(400, Easing::EmphasizedDecelerate);
    pub const EMPHASIZED_EXIT: Timing = Timing::new(200, Easing::EmphasizedAccelerate);
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform2d {
    pub translation: Vec2,
    pub scale: Vec2,
    pub rotation: f32,
}

impl Default for Transform2d {
    fn default() -> Transform2d {
        Transform2d::IDENTITY
    }
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
                self.elapsed = self.timing.duration;
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

#[derive(Component, Default, Clone)]
pub struct Motion {
    background: Option<Tween<Color>>,
    border: Option<Tween<Color>>,
    text: Option<Tween<Color>>,
    transform: Option<Tween<Transform2d>>,
    opacity: Option<Tween<f32>>,
    spin: Option<f32>,
    spun: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum Paint {
    Background,
    Border,
    Text,
}

impl Motion {
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

type Painted = (
    &'static mut Motion,
    Option<&'static mut BackgroundColor>,
    Option<&'static mut BorderColor>,
    Option<&'static mut TextColor>,
    Option<&'static mut UiTransform>,
    Option<&'static mut Opacity>,
);

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
            opacity.set(step_f32(tween, dt));
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

fn value_of<T: Lerp + Copy + PartialEq>(tween: &Tween<T>) -> T {
    tween.from.lerp_to(tween.target, tween.fraction())
}

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

pub(crate) struct MotionPlugin;

impl Plugin for MotionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, advance_motion.before(OpacitySet::Calculate));
    }
}
