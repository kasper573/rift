use engine::core::content::{ErrorPlace, LoadReport};
use engine::core::text::{LineText, SpanText};
use engine::core::units::Seconds;
use engine::systems::input::map::InputAction;
use engine::systems::messages;
use serde_json::json;

use crate::support::game;

const CATALOG: &str = "fixtures/game.messages.json";

#[test]
fn the_catalog_holds_each_message_to_icu_and_to_the_arguments_it_takes() {
    struct Case {
        what: &'static str,
        pointer: &'static str,
        set: serde_json::Value,
        rule: &'static str,
    }
    let cases = [
        Case {
            what: "an argument the message does not take",
            pointer: "/time/seconds",
            set: json!("{cuont, number} s"),
            rule: "unknown argument",
        },
        Case {
            what: "an argument to a message that takes none",
            pointer: "/time/ending",
            set: json!("Ending in {count}"),
            rule: "unknown argument",
        },
        Case {
            what: "a number chosen by as words",
            pointer: "/time/seconds",
            set: json!("{count, select, one {a second} other {seconds}}"),
            rule: "argument misused",
        },
        Case {
            what: "a number said as it is, which no locale formats",
            pointer: "/time/seconds",
            set: json!("{count} s"),
            rule: "argument misused",
        },
        Case {
            what: "a message that is not ICU MessageFormat",
            pointer: "/time/seconds",
            set: json!("{count, plural, one {# second}"),
            rule: "wrong type",
        },
        Case {
            what: "a locale that is none",
            pointer: "/locale",
            set: json!("no such place"),
            rule: "wrong type",
        },
    ];
    for case in cases {
        let report = game()
            .edit_messages(|catalog| {
                *catalog
                    .pointer_mut(case.pointer)
                    .expect("the catalog has the message") = case.set.clone()
            })
            .try_load()
            .err()
            .unwrap_or_else(|| panic!("{} loads", case.what));
        assert_eq!(
            refused(&report),
            [(case.pointer.to_owned(), case.rule)],
            "{}",
            case.what
        );
    }
}

#[test]
fn a_message_counts_formats_numbers_and_lists_the_way_its_locale_does() {
    {
        let said = |locale: &str, count: u64| {
            let content = game()
                .edit_messages(|catalog| {
                    catalog["locale"] = json!(locale);
                    catalog["time"]["seconds"] =
                        json!("{count, plural, one {# second} other {# seconds}}");
                })
                .load();
            messages::of(&content).time.seconds(count)
        };
        assert_eq!(
            [
                said("en-US", 1),
                said("en-US", 0),
                said("en-US", 1500),
                said("fr", 0),
                said("de", 1500),
            ],
            [
                "1 second",
                "0 seconds",
                "1,500 seconds",
                "0 second",
                "1.500 seconds"
            ],
            "plurals and digits follow the catalog's locale: French counts zero as one, German groups with dots"
        );
        let content = game()
            .edit_messages(|catalog| {
                catalog["time"]["seconds"] = json!("{count, number} s");
            })
            .load();
        assert_eq!(
            messages::of(&content).time.rough(Seconds(30.0)),
            "30 s",
            "a span is said in its largest unit"
        );
    }
    {
        let content = game()
            .edit_messages(|catalog| {
                catalog["interface"]["list_pair"] = json!("{first} y {second}");
                catalog["interface"]["list_start"] = json!("{first}, {rest}");
                catalog["interface"]["list_middle"] = json!("{first}, {rest}");
                catalog["interface"]["list_end"] = json!("{first} y {last}");
            })
            .load();
        let interface = &messages::of(&content).interface;
        let words = |items: &[&str]| {
            interface.listed_words(items.iter().map(|item| item.to_string()).collect())
        };
        assert_eq!(
            [
                words(&[]),
                words(&["pan"]),
                words(&["pan", "vino"]),
                words(&["pan", "vino", "queso", "miel"]),
            ],
            ["", "pan", "pan y vino", "pan, vino, queso y miel"]
        );
        let key = interface.listed(vec![
            LineText::plain("pan"),
            SpanText::from(InputAction::Interact).into(),
        ]);
        assert!(
            key.0.contains(&SpanText::from(InputAction::Interact)),
            "a key listed stays a key: {key:?}"
        );
    }
}

fn refused(report: &LoadReport) -> Vec<(String, &'static str)> {
    report
        .errors
        .iter()
        .filter(|error| error.file == CATALOG)
        .map(|error| {
            let pointer = match &error.at {
                ErrorPlace::Json { pointer, .. } => pointer.clone(),
                other => format!("{other:?}"),
            };
            (pointer, error.problem.rule)
        })
        .collect()
}
