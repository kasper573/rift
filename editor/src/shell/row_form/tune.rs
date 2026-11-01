use std::ops::RangeInclusive;
use ui::chip;
use ui::form;
use ui::input::SliderField;
use ui::theme;

use engine::core::audio::playback::{SfxDef, SfxScalar};
use ui::egui::Ui;

/// A field that tunes how a sound plays, and as far as the editor lets it go: twice the clip's
/// level, or an octave either way, is as far as a mix needs.
#[derive(Debug)]
pub struct SfxTuneField {
    pub name: &'static str,
    pub range: RangeInclusive<f32>,
    pub of: fn(&mut SfxDef) -> &mut SfxScalar,
}

const SFX_TUNE_FIELDS: [SfxTuneField; 2] = [
    SfxTuneField {
        name: "volume",
        range: RangeInclusive::new(0.0, 2.0),
        of: |sound| &mut sound.volume,
    },
    SfxTuneField {
        name: "pitch",
        range: RangeInclusive::new(0.5, 2.0),
        of: |sound| &mut sound.pitch,
    },
];

pub(crate) fn sfx_tune_field(name: &str) -> Option<&'static SfxTuneField> {
    SFX_TUNE_FIELDS.iter().find(|field| field.name == name)
}

/// The sound's tune as sliders. Gives the name of the field changed.
pub fn show(ui: &mut Ui, sound: &mut SfxDef) -> Option<&'static str> {
    let mut changed = None;
    for field in &SFX_TUNE_FIELDS {
        form::kv(ui, field.name, |ui| {
            if scalar(ui, (field.of)(sound), &field.range) {
                changed = Some(field.name);
            }
        });
    }
    changed
}

/// A value fixed or drawn at random from a span, each end on a slider within `range`. Values
/// outside it that the file already holds stay as they are until moved.
pub(crate) fn scalar(ui: &mut Ui, scalar: &mut SfxScalar, range: &RangeInclusive<f32>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let random = matches!(scalar, SfxScalar::Random { .. });
        if chip::mode(ui, "fixed", !random).clicked() && random {
            *scalar = SfxScalar::Fixed(middle(scalar));
            changed = true;
        }
        if chip::mode(ui, "at random", random).clicked() && !random {
            let at = middle(scalar);
            *scalar = SfxScalar::Random { min: at, max: at };
            changed = true;
        }
        match scalar {
            SfxScalar::Fixed(value) => changed |= slider(ui, value, range),
            SfxScalar::Random { min, max } => {
                if slider(ui, min, range) {
                    *max = max.max(*min);
                    changed = true;
                }
                ui.label(theme::muted("to"));
                if slider(ui, max, range) {
                    *min = min.min(*max);
                    changed = true;
                }
            }
        }
    });
    changed
}

fn slider(ui: &mut Ui, value: &mut f32, range: &RangeInclusive<f32>) -> bool {
    SliderField::new(value, range.clone()).show(ui)
}

fn middle(scalar: &SfxScalar) -> f32 {
    match *scalar {
        SfxScalar::Fixed(value) => value,
        SfxScalar::Random { min, max } => (min + max) / 2.0,
    }
}
