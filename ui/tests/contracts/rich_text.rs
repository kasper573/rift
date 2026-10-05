use std::time::Duration;

use ui::{RichPiece, RichSpan, TypewriterSpeed, reveal_duration, revealed};

const TEN_PER_SECOND: TypewriterSpeed = TypewriterSpeed(Some(10.0));

fn span(text: &'static str) -> RichPiece {
    RichSpan::plain(text).into()
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn characters_appear_at_the_typing_speed() {
    let line = [span("abcd")];
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(0)), 0);
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(250)), 2);
    assert_eq!(reveal_duration(&line, TEN_PER_SECOND), ms(400));
}

#[test]
fn a_pause_holds_back_what_follows() {
    let line = [span("ab"), RichPiece::Pause(ms(500)), span("cd")];
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(600)), 2);
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(800)), 3);
    assert_eq!(reveal_duration(&line, TEN_PER_SECOND), ms(900));
}

#[test]
fn slow_spans_take_longer_than_plain_ones() {
    let plain = [span("abcd")];
    let slow = [RichSpan::plain("abcd").slow().into()];
    assert!(reveal_duration(&slow, TEN_PER_SECOND) > reveal_duration(&plain, TEN_PER_SECOND));
}

#[test]
fn instant_speed_shows_everything_at_once() {
    let line = [span("ab"), RichPiece::Pause(ms(500)), span("cd")];
    let instant = TypewriterSpeed(None);
    assert_eq!(revealed(&line, instant, ms(0)), 4);
    assert_eq!(reveal_duration(&line, instant), ms(0));
}
