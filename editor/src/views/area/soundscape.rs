use engine::core::audio::mix::AudioCategory;
use engine::core::content::{AssetKind, AudioRef};
use engine::systems::area::{SoundChannel, SoundscapeDef};
use ui::egui::{Align, Layout, Ui};
use ui::form;
use ui::glyph::{self, Mark};
use ui::sheet;
use ui::theme;

use crate::forms::{self, FormCx};
use crate::picker;

/// The channels of `soundscape`, each edited as the game reads it.
pub(super) fn soundscape_extras(
    ui: &mut Ui,
    cx: &mut FormCx,
    soundscape: &mut SoundscapeDef,
    path: &str,
) -> bool {
    let mut changed = false;
    sheet::section(ui, "Channels");
    let mut remove = None;
    for (index, channel) in soundscape.channels.iter_mut().enumerate() {
        let channel_path = format!("{path}/channels/{index}");
        ui.horizontal(|ui| {
            ui.label(theme::strong(format!("channel {}", channel.channel)).monospace());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if glyph::button(ui, Mark::Cross, 11.0, theme::FAINT, "Remove")
                    .on_hover_text("Remove the channel")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        });
        changed |= forms::of_type(ui, cx, channel, &channel_path).is_some();
    }
    if let Some(index) = remove {
        soundscape.channels.remove(index);
        changed = true;
    }
    form::kv(ui, "+ channel", |ui| {
        let mut file = String::new();
        let new_path = format!("{path}/channels/{}", soundscape.channels.len());
        if picker::asset_of(ui, cx, AssetKind::Audio, &mut file, &new_path) && !file.is_empty() {
            let next = soundscape
                .channels
                .iter()
                .map(|channel| channel.channel)
                .max()
                .unwrap_or(0)
                + 1;
            soundscape.channels.push(SoundChannel {
                channel: next,
                file: AudioRef::new(file),
                volume: 1.0,
                pan: 0.0,
                category: AudioCategory::Ambience,
            });
            changed = true;
        }
    });
    changed
}
