use driver::Player;
use engine::core::math::Pos;
use engine::core::units::Meters;
use engine::systems::probe::{ProbeSoundZone, ProbeVoice};

use crate::chapters::Chapter;
use crate::shoot::Shoot;

pub const CHAPTER: Chapter = Chapter {
    name: "soundscape",
    title: "Soundscape",
    summary: "Music and ambience laid over the map, mixed by where you stand",
    play,
};

fn play(shoot: &mut Shoot) {
    let player = shoot.join_fresh();
    let mut me = shoot.roll(player);
    me.show_mixer();
    let seen = me.snapshot();
    let zone = |track: &str| {
        seen.soundscape
            .zones
            .iter()
            .find(|zone| zone.tracks.iter().any(|played| played.src.ends_with(track)))
            .unwrap_or_else(|| panic!("no zone plays {track}"))
            .clone()
    };
    let (town, shore, jetty) = (
        zone("harbour.flac"),
        zone("lagoon.flac"),
        zone("timbers.flac"),
    );
    let home = me.come_to_rest();

    me.caption("Soundscape zones lay music and ambience over the map, each track on a numbered channel, fading out past their edges");
    mixed(&mut me);

    me.caption("Where two zones overlap they mix: both themes share channel 1, and the surf both play pools");
    listen_from(&mut me, Pos::new(town.bounds.origin.x + 1.0, home.y));

    me.caption("Down at the shore the lagoon theme has channel 1 to itself, and the jungle chorus joins on channel 3");
    listen_from(&mut me, shore.bounds.center());

    me.caption("Back in town, the harbour theme resumes where it would be had it never stopped");
    listen_from(&mut me, home);

    me.caption("The jetty emits further than most: its timbers and lapping water carry well past its edges");
    listen_from(
        &mut me,
        Pos::new(
            jetty.bounds.origin.x + jetty.bounds.size.width + jetty.reach.0 / 2.0,
            jetty.bounds.center().y,
        ),
    );

    me.caption("On the jetty nothing claims channel 1, so the music fades out");
    listen_from(&mut me, jetty.bounds.center());

    me.caption("Out on the beach beyond every zone's reach, each channel fades to silence");
    listen_from(&mut me, beyond(&jetty));

    me.caption("And walking back, the harbour fades in again");
    listen_from(&mut me, home);
}

fn listen_from(me: &mut Player, spot: Pos<Meters>) {
    me.walk_near(spot);
    mixed(me);
}

fn mixed(me: &mut Player) {
    let mut last: Vec<(u32, String, f32)> = Vec::new();
    me.wait_until("the soundscape to settle", |snapshot| {
        let now: Vec<(u32, String, f32)> = snapshot
            .soundscape
            .voices
            .iter()
            .map(
                |ProbeVoice {
                     channel,
                     src,
                     heard,
                     ..
                 }| (*channel, src.clone(), *heard),
            )
            .collect();
        let settled = now == last;
        last = now;
        settled
    });
}

// The sand west of the jetty, past where its sounds reach.
fn beyond(zone: &ProbeSoundZone) -> Pos<Meters> {
    Pos::new(
        zone.bounds.origin.x - zone.reach.0 - 1.0,
        zone.bounds.center().y,
    )
}
