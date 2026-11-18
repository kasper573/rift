use crate::core::units::Seconds;
use crate::systems::messages::messages;

messages! {
    /// How the game says spans of time: in the largest unit a span fills, or the two largest.
    pub struct TimeMessages {
        /// A number `{count}` of seconds.
        seconds(count: Number),
        /// A number `{count}` of minutes.
        minutes(count: Number),
        /// A number `{count}` of hours.
        hours(count: Number),
        /// A number `{count}` of days.
        days(count: Number),
        /// Two units of one span together, `{larger}` and `{smaller}`.
        joined(larger: Text, smaller: Text),
        /// How long is left of something, `{time}`.
        left(time: Text),
        /// What is left of something about to end, under a minute.
        ending,
        /// When something happened, under a minute ago.
        just_now,
        /// When something happened, `{time}` ago.
        ago(time: Text),
        /// A countdown as a clock shows it, `{minutes}` and `{seconds}` left.
        clock(minutes: Number, seconds: Number),
    }
}

impl TimeMessages {
    /// How long ago something happened, `elapsed` before now.
    pub fn since(&self, elapsed: Seconds) -> String {
        if elapsed.0 < 60.0 {
            return self.just_now().to_owned();
        }
        self.ago(self.rough(elapsed))
    }

    /// `span` in the largest unit it fills, rounded up to a whole one.
    pub fn rough(&self, span: Seconds) -> String {
        let seconds = span.0.max(0.0).ceil() as u64;
        let (unit, length) = largest(seconds);
        self.unit(unit, seconds.div_ceil(length).max(1))
    }

    /// `span` in the two largest units it fills, to the whole second, minute or hour below them.
    pub fn exact(&self, span: Seconds) -> String {
        let seconds = span.0.max(0.0).ceil() as u64;
        let (unit, length) = largest(seconds);
        let whole = seconds / length;
        let rest = seconds % length;
        let Some(&(smaller, smaller_length)) =
            UNITS.iter().skip_while(|(known, _)| *known != unit).nth(1)
        else {
            return self.unit(unit, whole.max(1));
        };
        let rest = rest / smaller_length;
        if rest == 0 {
            return self.unit(unit, whole.max(1));
        }
        self.joined(self.unit(unit, whole), self.unit(smaller, rest))
    }

    /// What is left of `span` as a clock shows it, to the second.
    pub fn countdown(&self, span: Seconds) -> String {
        let seconds = span.0.max(0.0).ceil() as u64;
        self.clock(seconds / 60, seconds % 60)
    }

    /// How long is left of `span`, or that it is ending once less than a minute is.
    pub fn remaining(&self, span: Seconds) -> String {
        if span.0 < 60.0 {
            return self.ending().to_owned();
        }
        self.left(self.rough(span))
    }

    fn unit(&self, unit: TimeUnit, count: u64) -> String {
        match unit {
            TimeUnit::Second => self.seconds(count),
            TimeUnit::Minute => self.minutes(count),
            TimeUnit::Hour => self.hours(count),
            TimeUnit::Day => self.days(count),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeUnit {
    Day,
    Hour,
    Minute,
    Second,
}

/// The units a span is worded in, the largest first, with how many seconds each is.
const UNITS: [(TimeUnit, u64); 4] = [
    (TimeUnit::Day, 86_400),
    (TimeUnit::Hour, 3_600),
    (TimeUnit::Minute, 60),
    (TimeUnit::Second, 1),
];

/// The largest unit `seconds` fills a whole one of, the second when it fills none.
fn largest(seconds: u64) -> (TimeUnit, u64) {
    UNITS
        .into_iter()
        .find(|&(_, length)| seconds >= length)
        .unwrap_or((TimeUnit::Second, 1))
}
