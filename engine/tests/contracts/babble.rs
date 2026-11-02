use std::time::Duration;

use bevy::asset::io::AssetSourceId;
use bevy::asset::{AssetApp, AssetPlugin};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use engine::core::assets::SyncedFiles;
use engine::core::audio::loader::SoundClip;
use engine::core::audio::playback::{HushVoice, PlayClip, SfxPlace};
use engine::core::babble::{BabbleId, BabblePlugin, BabbleRank, Babbling};
use engine::core::content::Content;
use engine::core::text::{LettersPerSecond, StyledLine, StyledTextPlugin, Typewriter, TypingPace};
use testkit::{Declared, Game};

const FRAME: Duration = Duration::from_millis(50);
const TEN_PER_SECOND: TypingPace = TypingPace {
    letters: LettersPerSecond(10.0),
    slow_by: 2.5,
};

#[test]
fn a_babbling_line_speaks_a_clip_per_letter_as_it_types() {
    let mut app = voice_app();
    let babble = bram(&app);
    app.world_mut().spawn((
        Typewriter::default(),
        StyledLine::plain_text("The quick brown fox"),
        Babbling {
            babble,
            place: SfxPlace::Interface,
            rank: BabbleRank::Foremost,
        },
    ));

    for _ in 0..80 {
        app.update();
    }

    let heard = app.world().resource::<Heard>().0;
    assert_eq!(heard, 16, "a clip for each of the sixteen letters");
}

#[test]
fn a_line_babbles_the_same_letters_at_the_same_moments_whatever_the_frame_rate() {
    let sixty = spoken_at(
        Duration::from_millis(16),
        "The quick brown fox jumps over the lazy dog!",
    );
    let ten = spoken_at(
        Duration::from_millis(100),
        "The quick brown fox jumps over the lazy dog!",
    );
    assert!(sixty.len() > 4, "{} clips", sixty.len());
    assert_eq!(sixty, ten);
    let moments: Vec<u32> = sixty.iter().map(|(_, _, due)| *due).collect();
    assert!(
        moments.windows(2).all(|pair| pair[0] < pair[1]),
        "one letter at a time, each on its own moment: {moments:?}"
    );
}

#[test]
fn a_line_cut_short_is_hushed_and_a_line_spoken_out_is_not() {
    let mut app = voice_app();
    let babble = bram(&app);
    let line = app
        .world_mut()
        .spawn((
            Typewriter::default(),
            StyledLine::plain_text("The quick brown fox"),
            Babbling {
                babble,
                place: SfxPlace::Interface,
                rank: BabbleRank::Foremost,
            },
        ))
        .id();
    for _ in 0..80 {
        app.update();
    }
    app.world_mut().despawn(line);
    app.update();
    assert_eq!(app.world().resource::<Hushed>().0, 0, "a line spoken out");

    let line = app
        .world_mut()
        .spawn((
            Typewriter::default(),
            StyledLine::plain_text("The quick brown fox"),
            Babbling {
                babble,
                place: SfxPlace::Interface,
                rank: BabbleRank::Foremost,
            },
        ))
        .id();
    for _ in 0..4 {
        app.update();
    }
    app.world_mut()
        .get_mut::<Typewriter>(line)
        .unwrap()
        .finish();
    app.update();
    assert_eq!(app.world().resource::<Hushed>().0, 1, "a line cut short");
}

/// The clips a line is spoken with when every frame lasts `frame`: each clip's file, pitch and
/// the millisecond it is due.
fn spoken_at(frame: Duration, line: &str) -> Vec<(String, u32, u32)> {
    let mut app = voice_app();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame))
        .insert_resource(testkit::typing().pace_for(None))
        .init_resource::<Spoken>()
        .add_systems(Last, note_clips);
    let babble = bram(&app);
    app.world_mut().spawn((
        Typewriter::default(),
        StyledLine::plain_text(line.to_owned()),
        Babbling {
            babble,
            place: SfxPlace::Interface,
            rank: BabbleRank::Foremost,
        },
    ));
    let frames = (Duration::from_secs(4).as_secs_f32() / frame.as_secs_f32()) as usize;
    for _ in 0..frames {
        app.update();
    }
    app.world().resource::<Spoken>().0.clone()
}

#[derive(Resource, Default)]
struct Spoken(Vec<(String, u32, u32)>);

fn note_clips(mut clips: MessageReader<PlayClip>, mut spoken: ResMut<Spoken>) {
    for clip in clips.read() {
        let file = clip
            .clip
            .path()
            .map(|path| path.to_string())
            .unwrap_or_default();
        spoken.0.push((
            file,
            (clip.tune.pitch.0 * 1000.0).round() as u32,
            clip.due.as_millis() as u32,
        ));
    }
}

#[derive(Resource, Default)]
struct Heard(usize);

#[derive(Resource, Default)]
struct Hushed(usize);

fn voice_app() -> App {
    let mut app = App::new();
    let files = SyncedFiles::default();
    app.insert_resource(files.clone());
    AssetApp::register_asset_source(&mut app, AssetSourceId::Default, files.source());
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        StyledTextPlugin,
        BabblePlugin,
    ))
    .init_asset::<SoundClip>()
    .add_message::<PlayClip>()
    .add_message::<HushVoice>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME))
    .insert_resource(TEN_PER_SECOND)
    .insert_resource(testkit::speech())
    .insert_resource(voices())
    .init_resource::<Heard>()
    .init_resource::<Hushed>()
    .add_systems(Last, (count_clips, count_hushes));
    app
}

fn count_clips(mut clips: MessageReader<PlayClip>, mut heard: ResMut<Heard>) {
    heard.0 += clips.read().count();
}

fn count_hushes(mut hushes: MessageReader<HushVoice>, mut hushed: ResMut<Hushed>) {
    hushed.0 += hushes.read().count();
}

fn voices() -> Content {
    Game::new().babble("bram").load()
}

fn bram(app: &App) -> BabbleId {
    app.world()
        .resource::<Content>()
        .declared("bram")
        .expect("bram babbles")
}
