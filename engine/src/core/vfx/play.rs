//! Where a layer is in its run and what its tracks and colours hold there, worked out the same
//! wherever a vfx is drawn.

use bevy::color::{ColorToComponents, LinearRgba};

use super::{ColorFill, ParamValue, VfxColor, VfxDef, VfxEase, VfxLayer, VfxParams, VfxTrack};
use crate::core::units::Seconds;

/// A layer caught mid-run: the seconds since it began and how far through its duration it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerTime {
    pub local: Seconds,
    pub progress: f32,
}

impl VfxLayer {
    /// Where the layer is at `elapsed` seconds into its instance, or nothing while it waits for its
    /// delay or once it is over.
    pub fn at(&self, elapsed: Seconds) -> Option<LayerTime> {
        let run = self.run();
        let local = elapsed - run.delay;
        (local.0 >= 0.0 && local <= run.duration).then(|| LayerTime {
            local,
            progress: (local.0 / run.duration.0.max(0.001)).clamp(0.0, 1.0),
        })
    }
}

impl VfxEase {
    /// How far between two keys a value stands `t` of the way from one to the next.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            VfxEase::Linear => t,
            VfxEase::In => t * t * t,
            VfxEase::Out => 1.0 - (1.0 - t).powi(3),
            VfxEase::InOut => t * t * (3.0 - 2.0 * t),
            VfxEase::Back => {
                let overshoot = 1.70158;
                let u = t - 1.0;
                1.0 + (overshoot + 1.0) * u * u * u + overshoot * u * u
            }
        }
    }
}

/// A value a track eases between.
pub trait Blendable: Copy {
    fn blend(self, other: Self, t: f32) -> Self;
}

impl Blendable for f32 {
    fn blend(self, other: f32, t: f32) -> f32 {
        self + (other - self) * t
    }
}

impl<const N: usize> Blendable for [f32; N] {
    fn blend(self, other: [f32; N], t: f32) -> [f32; N] {
        std::array::from_fn(|i| self[i].blend(other[i], t))
    }
}

impl Blendable for LinearRgba {
    fn blend(self, other: LinearRgba, t: f32) -> LinearRgba {
        LinearRgba::from_f32_array(self.to_f32_array().blend(other.to_f32_array(), t))
    }
}

impl<T> VfxTrack<T> {
    /// The value at `progress` through the run, each key read through `value`.
    pub fn at<V: Blendable>(&self, progress: f32, value: impl Fn(&T) -> V) -> V {
        let first = &self.keys[0];
        if progress <= first.0 {
            return value(&first.1);
        }
        for pair in self.keys.windows(2) {
            let ((from, a), (to, b)) = (&pair[0], &pair[1]);
            if progress <= *to {
                let t = (progress - from) / (to - from).max(1e-6);
                return value(a).blend(value(b), self.ease.apply(t));
            }
        }
        value(&self.keys[self.keys.len() - 1].1)
    }
}

impl VfxColor {
    /// The colour in linear light, above 1 glowing, as the row's parameters `given` fill it.
    pub fn linear(&self, def: &VfxDef, given: &VfxParams) -> LinearRgba {
        let hue = match &self.color {
            ColorFill::Value(color) => *color,
            ColorFill::Param { param } => match def.param(param, given) {
                Some(ParamValue::Color(color)) => color,
                _ => crate::core::math::Rgba::WHITE,
            },
        };
        let linear = hue.color().to_linear();
        LinearRgba::new(
            linear.red * self.brightness,
            linear.green * self.brightness,
            linear.blue * self.brightness,
            linear.alpha * self.alpha,
        )
    }
}
