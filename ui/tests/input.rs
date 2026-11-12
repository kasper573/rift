use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use ui::input::{NumberField, SliderField};

struct Typing {
    ctx: Context,
    field: Rect,
}

impl Typing {
    fn new() -> Self {
        let ctx = Context::default();
        ui::apply(&ctx);
        Self {
            ctx,
            field: Rect::NOTHING,
        }
    }

    /// One frame of the field holding `values`, after `events`: whether it took new numbers.
    fn frame<const K: usize>(
        &mut self,
        events: Vec<Event>,
        values: [&mut f32; K],
        range: Option<std::ops::RangeInclusive<f32>>,
    ) -> bool {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
            events,
            ..RawInput::default()
        };
        let mut taken = false;
        let mut field = Rect::NOTHING;
        let mut once = Some((values, range));
        let mut output = self.ctx.run_ui(input, |ui| {
            let Some((values, range)) = once.take() else {
                return;
            };
            let number = NumberField::of(values);
            let number = match range {
                Some(range) => number.range(range),
                None => number,
            };
            taken = number.show(ui);
            field = ui.min_rect();
        });
        output.textures_delta.clear();
        self.field = field;
        taken
    }

    /// The field clicked into, all of it selected, `text` typed over it and `then` pressed.
    fn types<const K: usize>(
        &mut self,
        values: [&mut f32; K],
        range: Option<std::ops::RangeInclusive<f32>>,
        text: &str,
        then: Key,
    ) -> bool {
        let mut held: Vec<f32> = values.iter().map(|value| **value).collect();
        let mut frame = |typing: &mut Self, events: Vec<Event>| {
            let mut refs = held.iter_mut();
            let values: [&mut f32; K] = std::array::from_fn(|_| refs.next().expect("a value"));
            typing.frame(events, values, range.clone())
        };
        frame(self, Vec::new());
        let at = self.field.center();
        let click = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(self, vec![Event::PointerMoved(at), click(true)]);
        frame(self, vec![click(false)]);
        frame(self, vec![key(Key::A, Modifiers::COMMAND)]);
        frame(self, vec![Event::Text(text.to_owned())]);
        let taken = frame(self, vec![key(then, Modifiers::NONE)]);
        for (value, typed) in values.into_iter().zip(held) {
            *value = typed;
        }
        taken
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn a_number_field_takes_what_is_typed_only_once_it_reads_and_is_confirmed() {
    struct Case {
        start: &'static [f32],
        text: &'static str,
        range: Option<std::ops::RangeInclusive<f32>>,
        then: Key,
        expected: &'static [f32],
        taken: bool,
    }
    let case = |start, text, range, then, expected, taken| Case {
        start,
        text,
        range,
        then,
        expected,
        taken,
    };
    let cases = [
        case(&[1.0], "7.5", None, Key::Enter, &[7.5], true),
        case(&[1.0], "99", Some(0.0..=10.0), Key::Enter, &[10.0], true),
        case(&[1.0], "-3", Some(0.0..=10.0), Key::Enter, &[0.0], true),
        case(&[1.0], "seven", None, Key::Enter, &[1.0], false),
        case(&[1.0], "7", None, Key::Escape, &[1.0], false),
        case(&[1.0, 2.0], "3, 4", None, Key::Enter, &[3.0, 4.0], true),
        case(&[1.0, 2.0], "5", None, Key::Enter, &[1.0, 2.0], false),
    ];
    for Case {
        start,
        text,
        range,
        then,
        expected,
        taken,
    } in cases
    {
        let mut values = start.to_vec();
        let took = match values.as_mut_slice() {
            [x] => Typing::new().types([x], range, text, then),
            [x, y] => Typing::new().types([x, y], range, text, then),
            _ => unreachable!("cases hold one number or two"),
        };
        assert_eq!(
            (values.as_slice(), took),
            (expected, taken),
            "typing {text:?} then {then:?} over {start:?}"
        );
    }
}

#[test]
fn a_slider_keeps_a_value_outside_its_range_until_slid_then_takes_a_plain_one() {
    let ctx = Context::default();
    ui::apply(&ctx);
    let frame = |events: Vec<Event>, value: &mut f32| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
            events,
            ..RawInput::default()
        };
        let mut moved = false;
        let mut slider = Rect::NOTHING;
        let mut output = ctx.run_ui(input, |ui| {
            moved = SliderField::new(value, 0.0..=1.0).show(ui);
            let room = ui.min_rect();
            slider = Rect::from_min_size(
                room.min,
                Vec2::new(ui.spacing().slider_width, room.height()),
            );
        });
        output.textures_delta.clear();
        (moved, slider)
    };
    let mut value = 3.0;
    let (moved, slider) = frame(Vec::new(), &mut value);
    assert!(!moved);
    assert_eq!(value, 3.0);
    let at = slider.center();
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    assert!(frame(vec![Event::PointerMoved(at), press(true)], &mut value).0);
    frame(vec![press(false)], &mut value);
    assert_eq!(value, 0.5);
}

#[test]
fn a_wheel_takes_a_click_on_a_leaf_only_once_it_stands_wholly_open() {
    use ui::wheel::{WheelPick, wheel};

    let ctx = Context::default();
    ui::apply(&ctx);
    let center = Pos2::new(200.0, 200.0);
    let frame = |events: Vec<Event>, opened: f32| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))),
            events,
            ..RawInput::default()
        };
        let mut picked = None;
        let mut output = ctx.run_ui(input, |ui| {
            let picks = ["wave", "nod", "cheer", "sit"]
                .map(|name| WheelPick { name, image: None })
                .into();
            picked = wheel(ui, center, picks, opened);
        });
        output.textures_delta.clear();
        picked
    };
    let button = |at: Pos2, pressed: bool| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let click = |at: Pos2, opened: f32| {
        frame(vec![Event::PointerMoved(at)], opened);
        frame(vec![button(at, true)], opened);
        frame(vec![button(at, false)], opened)
    };
    let second_leaf = center + Vec2::new(119.0, 0.0);

    assert_eq!(
        click(second_leaf, 0.5),
        None,
        "a fan still opening takes no click"
    );
    assert_eq!(
        click(second_leaf, 1.0),
        Some(1),
        "the leaf under the pointer, the first at the top and the rest clockwise"
    );
    assert_eq!(click(center, 1.0), None, "the rivet is no leaf");
    assert_eq!(click(second_leaf, 0.0), None, "a folded fan is not there");
}
