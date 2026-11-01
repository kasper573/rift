//! How numbers and lists read in a line of text.

use std::fmt::Display;

pub fn plural<N: PartialEq + From<u8>>(count: N) -> &'static str {
    if count == N::from(1) { "" } else { "s" }
}

/// `count` of `noun`, the noun in the plural unless there is one.
pub fn counted<N: Display + PartialEq + From<u8>>(count: N, noun: &str) -> String {
    let shown = count.to_string();
    format!("{shown} {noun}{}", plural(count))
}

pub fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

pub fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `number` with its sign, a plus for a gain and a typographic minus for a loss, to hundredths.
pub fn signed(number: f64) -> String {
    let magnitude = (number.abs() * 100.0).round() / 100.0;
    let sign = if number < 0.0 { '−' } else { '+' };
    format!("{sign}{magnitude}")
}

/// `text` cut to `max` characters, an ellipsis ending what was cut.
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The time of day `unix_millis` after the epoch falls on in the user's own time zone.
pub fn clock(unix_millis: u64, with_seconds: bool) -> String {
    let format = if with_seconds { "%H:%M:%S" } else { "%H:%M" };
    i64::try_from(unix_millis)
        .ok()
        .and_then(|millis| jiff::Timestamp::from_millisecond(millis).ok())
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime(format)
                .to_string()
        })
        .unwrap_or_default()
}

/// How long ago something was, in the largest unit that reads naturally.
pub fn ago(elapsed_seconds: f32) -> String {
    let seconds = elapsed_seconds.max(0.0) as u64;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if seconds < 60 {
        "just now".to_owned()
    } else if minutes < 60 {
        format!("{minutes} min ago")
    } else if hours < 24 {
        format!("{hours} h ago")
    } else if days == 1 {
        "yesterday".to_owned()
    } else if days < 7 {
        format!("{days} days ago")
    } else if days < 14 {
        "last week".to_owned()
    } else if days < 31 {
        format!("{} weeks ago", days / 7)
    } else {
        format!("{} months ago", days / 30)
    }
}
