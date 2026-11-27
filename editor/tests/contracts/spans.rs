use editor::spans;
use engine::core::content::serial::json_of;
use engine::core::text::{TextFx, TextInk};
use engine::systems::dialogue::span::ScriptSpan;
use serde_json::{Value, json};

fn line_of(json: Value) -> Vec<ScriptSpan> {
    serde_json::from_value(json).expect("a line of spans")
}

fn json(spans: Option<Vec<ScriptSpan>>) -> Option<Value> {
    spans.map(|spans| json_of(&spans).expect("spans write"))
}

fn token_of(span: Value) -> String {
    spans::plain(&line_of(json!([span])))
}

#[test]
fn retyping_a_line_keeps_its_styled_spans_and_its_values_whole_or_takes_them_whole() {
    {
        let line = line_of(json!(["The ", { "text": "red", "ink": "danger" }, " key"]));
        assert_eq!(
            json(spans::retype(&line, "The red key", "The reed key")),
            Some(json!(["The ", { "text": "reed", "ink": "danger" }, " key"]))
        );
        assert_eq!(
            json(spans::retype(&line, "The red key", "The reds key")),
            Some(json!(["The ", { "text": "reds", "ink": "danger" }, " key"]))
        );
    }
    {
        let line = line_of(json!(["You have ", { "value": 3 }]));
        let shown = spans::plain(&line);
        assert_eq!(
            json(spans::retype(&line, &shown, &format!("{shown} coins"))),
            Some(json!(["You have ", { "value": 3 }, " coins"]))
        );

        let line = line_of(json!([{ "value": 1 }, { "value": 2 }]));
        let shown = spans::plain(&line);
        let between = token_of(json!({ "value": 1 })).chars().count();
        let mut typed: Vec<char> = shown.chars().collect();
        typed.insert(between, '+');
        let typed: String = typed.into_iter().collect();
        assert_eq!(
            json(spans::retype(&line, &shown, &typed)),
            Some(json!([{ "value": 1 }, "+", { "value": 2 }]))
        );
    }
    {
        let line = line_of(json!(["You have ", { "value": 3 }, " coins"]));
        let shown = spans::plain(&line);
        let token = token_of(json!({ "value": 3 }));
        let without = shown.replace(&token, "");
        assert_eq!(
            json(spans::retype(&line, &shown, &without)),
            Some(json!(["You have  coins"]))
        );
        let cut: String = token.chars().skip(1).collect();
        let partly = shown.replace(&token, &cut);
        assert_eq!(json(spans::retype(&line, &shown, &partly)), None);
    }
    {
        let line = line_of(json!(["a ", { "text": "red", "ink": "danger" }, " key"]));
        let typed = json(spans::retype(&line, "a red key", "a  key"));
        assert_eq!(typed, Some(json!(["a  key"])));
    }
}

#[test]
fn restyling_a_selection_splits_the_span_around_it_leaving_values_and_a_value_replaces_it() {
    {
        let line = line_of(json!(["a red key"]));
        let styled = spans::restyle(&line, (2, 5), |style| {
            style.push(TextFx::Ink(TextInk::Danger))
        });
        assert_eq!(
            json(Some(styled)),
            Some(json!(["a ", { "text": "red", "ink": "danger" }, " key"]))
        );
    }
    {
        let line = line_of(json!(["a ", { "value": 3 }, " b"]));
        let shown = spans::plain(&line);
        let styled = spans::restyle(&line, (0, shown.chars().count()), |style| {
            style.push(TextFx::Slow)
        });
        assert_eq!(
            json(Some(styled)),
            Some(json!([
                { "text": "a ", "slow": true },
                { "value": 3 },
                { "text": " b", "slow": true }
            ]))
        );
    }
    {
        let line = line_of(json!(["a red key"]));
        assert_eq!(
            json(spans::with_value(&line, (2, 5))),
            Some(json!(["a ", { "value": 0 }, " key"]))
        );
    }
}
