use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use ui::egui;
use ui::face;
use ui::theme;

use bevy::asset::{AssetPath, AssetServer};
use bevy::prelude::{Entity, World};
use engine::core::assets::SyncedAssetReader;
use engine::core::audio::playback::{self, PlaySfx, SfxDef, SfxPlace};
use engine::core::audio::soundscape::{SoundscapeChannel, SoundscapeLayer, SoundscapeZone};
use engine::core::babble::{BabbleDef, BabbleRank, Babbling};
use engine::core::content::Content;
use engine::core::content::sync::SyncedAssets;
use engine::core::math::Pos;
use engine::core::text::{StyledLine, Typewriter};
use engine::core::units::Meters;
use engine::systems::GameSettings;
use engine::systems::area::{self, AreaId};
use engine::systems::script::Sky;
use ui::egui::Ui;

use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::project::ProjectFiles;
use crate::views::area::hour::ShownSky;

const BABBLE_SAMPLE: &str = "The quick brown fox jumps over the lazy dog";

pub struct ListeningPoint {
    pub area: String,
    pub at: Pos<Meters>,
    /// The sky the point was heard under.
    pub sky: Sky,
    pub heard: Vec<HeardZone>,
}

/// A soundscape zone as heard from a listening point.
pub struct HeardZone {
    pub name: String,
    pub gain: f32,
    pub outside: Meters,
    pub zone: SoundscapeZone,
}

/// Hears the area `area_id` from `at` under the sky `sky` shows it under.
pub fn listen(
    editor: &mut Editor,
    content: &Content,
    area_id: AreaId,
    at: Pos<Meters>,
    sky: &ShownSky,
) {
    let area = area::load(content, area_id);
    let hearing = content.built::<GameSettings>().hearing;
    let heard = area
        .soundscapes
        .iter()
        .map(|soundscape| soundscape.zone(|when| sky.share(when)))
        .enumerate()
        .filter_map(|(index, zone)| {
            let positional = zone.heard_from(at, &hearing)?;
            let stem = zone
                .layers
                .first()
                .and_then(|layer| Path::new(layer.src.as_str()).file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some(HeardZone {
                name: format!("soundscape {} · {stem}", index + 1),
                gain: positional.proximity,
                outside: Meters((at - zone.shape.nearest(at)).length()),
                zone,
            })
        })
        .collect();
    editor.views.sound.listening = Some(ListeningPoint {
        area: area_id.path(content).to_owned(),
        at,
        sky: sky.sky(),
        heard,
    });
}

/// What a babble preview says, for the engine to speak in a babble's voice.
pub struct BabbleRequest {
    pub babble: String,
    pub text: String,
}

pub struct SoundViews {
    pub listening: Option<ListeningPoint>,
    /// Sfx rows asked to play this frame, and to stop.
    pub play: Vec<String>,
    pub stop: Vec<String>,
    /// Sfx rows still sounding, as of the last sync.
    pub sounding: HashSet<String>,
    pub babble: Option<BabbleRequest>,
    pub stop_babble: bool,
    pub babble_text: String,
    babbler: Option<Entity>,
    /// The content generation the engine's asset files and content were last taken from, with
    /// the hash of each asset file then.
    synced: Option<(u64, ProjectFiles)>,
}

impl SoundViews {
    pub fn babbling(&self) -> bool {
        self.babbler.is_some()
    }
}

impl Default for SoundViews {
    fn default() -> SoundViews {
        SoundViews {
            listening: None,
            play: Vec::new(),
            stop: Vec::new(),
            sounding: HashSet::new(),
            babble: None,
            stop_babble: false,
            babble_text: BABBLE_SAMPLE.to_owned(),
            babbler: None,
            synced: None,
        }
    }
}

/// Hands the project's assets and content to the engine's audio as they change, then plays what
/// the windows asked for through it: sfx rows as the game plays them, and a babble speaking a
/// line through the game's own typewriter.
pub fn sync(editor: &mut Editor, world: &mut World) {
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let views = &mut editor.views.sound;
    if views.synced.as_ref().map(|(generation, _)| *generation) != Some(project.generation) {
        world
            .resource::<SyncedAssetReader>()
            .replace(SyncedAssets::read_from(Arc::new(project.files.clone())));
        if let Some((_, before)) = &views.synced {
            let server = world.resource::<AssetServer>();
            for path in project.files.changed_since(before) {
                let asset = AssetPath::from(path);
                if server.get_path_id(asset.clone()).is_some() {
                    server.reload(asset);
                }
            }
        }
        views.synced = Some((project.generation, project.files.clone()));
    }
    let Some(content) = &project.content else {
        return;
    };
    for key in std::mem::take(&mut views.play) {
        if let Some(id) = content.by_path::<SfxDef>(&key) {
            playback::play(
                world,
                PlaySfx {
                    id,
                    place: SfxPlace::Interface,
                },
            );
            views.sounding.insert(key);
        }
    }
    for key in std::mem::take(&mut views.stop) {
        if let Some(id) = content.by_path::<SfxDef>(&key) {
            playback::stop(world, id);
            views.sounding.remove(&key);
        }
    }
    let still: Vec<String> = views
        .sounding
        .iter()
        .filter(|key| {
            content
                .by_path::<SfxDef>(key)
                .is_some_and(|id| playback::is_sounding(world, id))
        })
        .cloned()
        .collect();
    views.sounding = still.into_iter().collect();
    if std::mem::take(&mut views.stop_babble)
        && let Some(previous) = views.babbler.take()
    {
        world.despawn(previous);
    }
    if let Some(request) = views.babble.take()
        && let Some(babble) = content.by_path::<BabbleDef>(&request.babble)
    {
        if let Some(previous) = views.babbler.take() {
            world.despawn(previous);
        }
        let speaker = world
            .spawn((
                Typewriter::default(),
                StyledLine::plain_text(request.text),
                Babbling {
                    babble,
                    place: SfxPlace::Interface,
                    rank: BabbleRank::Foremost,
                },
            ))
            .id();
        views.babbler = Some(speaker);
    }
    if let Some(speaker) = views.babbler
        && world
            .get::<Typewriter>(speaker)
            .is_none_or(Typewriter::is_finished)
    {
        world.despawn(speaker);
        views.babbler = None;
    }
}

pub fn panel(editor: &Editor, ui: &mut Ui) {
    let Some(listening) = &editor.views.sound.listening else {
        return;
    };
    ui.label(theme::muted(format!(
        "Listening at {:.1}, {:.1} in {} · what a player there hears at the default settings · {} click moves the listener",
        listening.at.x,
        listening.at.y,
        listening.area,
        EditorKey::Listen.name()
    )));
    ui.add_space(6.0);
    if listening.heard.is_empty() {
        ui.label(theme::muted("Silence: no soundscape reaches this spot."));
        return;
    }
    let mut rows: Vec<(SoundscapeChannel, &SoundscapeLayer, &HeardZone)> = listening
        .heard
        .iter()
        .flat_map(|heard| {
            heard
                .zone
                .layers
                .iter()
                .map(move |layer| (layer.channel, layer, heard))
        })
        .collect();
    rows.sort_by_key(|(channel, ..)| *channel);
    egui::Grid::new("listen")
        .num_columns(4)
        .spacing([28.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            for head in ["ch", "plays here", "from", "heard"] {
                ui.label(theme::muted(head).monospace().size(12.0));
            }
            ui.end_row();
            for (channel, layer, heard) in rows {
                let reach = if heard.outside.0 <= 0.0 {
                    "inside".to_owned()
                } else {
                    format!("{:.1} m out · {:.0}%", heard.outside.0, heard.gain * 100.0)
                };
                ui.label(face::mono(channel.0.to_string()));
                let now = match layer.share {
                    share if share <= 0.0 => " · silent at the hour shown".to_owned(),
                    share if share < 1.0 => format!(" · {:.0}% of it now", share * 100.0),
                    _ => String::new(),
                };
                ui.label(face::mono(format!(
                    "{} · {:.2}{now}",
                    layer.src.as_str(),
                    layer.volume
                )));
                ui.label(face::mono(&heard.name));
                ui.label(face::mono(reach));
                ui.end_row();
            }
        });
}
