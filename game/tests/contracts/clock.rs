use game::core::time::{GameDay, Seconds, UnixMillis, UtcHour};

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;

fn hour(value: u8) -> UtcHour {
    UtcHour::try_from(value).expect("valid hour")
}

#[test]
fn the_day_rolls_over_exactly_at_the_reset_hour() {
    let reset = hour(4);
    let reset_instant = UnixMillis(10 * DAY + 4 * HOUR);
    let before = GameDay::of(UnixMillis(reset_instant.0 - 1), reset);
    let at = GameDay::of(reset_instant, reset);
    assert_eq!(at.0, before.0 + 1);
}

#[test]
fn the_day_does_not_roll_over_at_midnight_unless_that_is_the_reset() {
    let midnight = UnixMillis(10 * DAY);
    let just_before = UnixMillis(10 * DAY - 1);
    assert_eq!(
        GameDay::of(just_before, hour(4)),
        GameDay::of(midnight, hour(4))
    );
    assert_ne!(
        GameDay::of(just_before, UtcHour::MIDNIGHT),
        GameDay::of(midnight, UtcHour::MIDNIGHT)
    );
}

#[test]
fn hours_outside_a_day_are_rejected() {
    assert!(UtcHour::try_from(23).is_ok());
    assert!(UtcHour::try_from(24).is_err());
    assert!(serde_json::from_str::<UtcHour>("24").is_err());
    assert_eq!(serde_json::from_str::<UtcHour>("23").ok(), Some(hour(23)));
}

#[test]
fn durations_round_trip_through_wall_time() {
    let start = UnixMillis(1_000);
    let later = start.after(Seconds(2.5));
    assert_eq!(later, UnixMillis(3_500));
    assert_eq!(later.since(start), Seconds(2.5));
    assert_eq!(start.since(later), Seconds(0.0));
}
