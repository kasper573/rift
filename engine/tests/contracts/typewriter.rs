use std::time::Duration;

use engine::core::text::{
    CharactersPerSecond, InputRef, StyledSpan, TextKey, TextPiece, TypingPace, reveal_duration,
    revealed,
};

const TEN_PER_SECOND: TypingPace = TypingPace {
    characters: CharactersPerSecond(10.0),
    slow_by: 2.5,
};

#[test]
fn characters_appear_at_the_typing_speed_a_key_as_one_slow_spans_slower_and_a_pause_holds_back_what_follows()
 {
    let line = [span("abcd")];
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(0)), 0);
    assert_eq!(revealed(&line, TEN_PER_SECOND, ms(250)), 2);
    assert_eq!(reveal_duration(&line, TEN_PER_SECOND), ms(400));

    let keyed = [span("ab"), TextPiece::Key(TextKey::Input(InputRef(0)))];
    assert_eq!(revealed(&keyed, TEN_PER_SECOND, ms(300)), 3);

    let slow = [StyledSpan::plain("abcd").slow().into()];
    assert!(reveal_duration(&slow, TEN_PER_SECOND) > reveal_duration(&line, TEN_PER_SECOND));

    let paused = [span("ab"), TextPiece::Pause(ms(500)), span("cd")];
    assert_eq!(revealed(&paused, TEN_PER_SECOND, ms(600)), 2);
    assert_eq!(revealed(&paused, TEN_PER_SECOND, ms(800)), 3);
    assert_eq!(reveal_duration(&paused, TEN_PER_SECOND), ms(900));
}

fn span(text: &'static str) -> TextPiece {
    StyledSpan::plain(text).into()
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}
