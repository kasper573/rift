mod harness;

use std::time::Duration;

use bevy_picking::prelude::Pickable;
use bevy_view::view;
use harness::{State, Ui};
use ui::{Tooltip, TooltipContent, TooltipOutlet, TooltipProvider, TooltipTrigger};

const DELAY: Duration = Duration::from_millis(400);
const SKIP: Duration = Duration::from_millis(300);

fn tooltip(open: &State<bool>) -> impl Fn() -> bevy_view::View {
    let open = open.clone();
    move || {
        let set = open.clone();
        view! {
            <TooltipProvider delay={DELAY} skip_delay={SKIP}>
                <Tooltip open={open.get()} on_open_change={move |_w, value| set.set(value)}>
                    <TooltipTrigger><text>"icon"</text></TooltipTrigger>
                    <TooltipContent><text>"tip"</text></TooltipContent>
                </Tooltip>
            </TooltipProvider>
            <TooltipOutlet/>
        }
    }
}

fn trigger_of(ui: &Ui) -> bevy_ecs::prelude::Entity {
    let provider = ui.children()[0];
    let tooltip = ui.children_of(provider)[0];
    ui.children_of(tooltip)[0]
}

#[test]
fn a_tooltip_opens_only_after_the_delay() {
    let mut ui = Ui::new().with_clock();
    let open = State::new(false);
    let tree = tooltip(&open);
    ui.render(tree());
    let trigger = trigger_of(&ui);

    ui.activate_over(trigger);
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["icon".to_owned()],
        "still closed before the delay elapses"
    );

    ui.advance(DELAY);
    ui.render(tree());
    assert!(
        ui.texts().contains(&"tip".to_owned()),
        "opens once the delay has elapsed"
    );
}

#[test]
fn a_tooltip_closes_when_the_pointer_leaves() {
    let mut ui = Ui::new().with_clock();
    let open = State::new(false);
    let tree = tooltip(&open);
    ui.render(tree());
    let trigger = trigger_of(&ui);

    ui.activate_over(trigger);
    ui.advance(DELAY);
    ui.render(tree());
    assert!(ui.texts().contains(&"tip".to_owned()));

    ui.activate_out(trigger);
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["icon".to_owned()],
        "leaving the trigger unmounts the tooltip"
    );
}

#[test]
fn skip_delay_opens_a_tooltip_immediately_within_the_window() {
    let mut ui = Ui::new().with_clock();
    let open = State::new(false);
    let tree = tooltip(&open);
    ui.render(tree());
    let trigger = trigger_of(&ui);

    ui.activate_over(trigger);
    ui.advance(DELAY);
    ui.render(tree());
    ui.activate_out(trigger);
    ui.render(tree());

    ui.advance(Duration::from_millis(100));
    ui.activate_over(trigger);
    ui.render(tree());
    assert!(
        ui.texts().contains(&"tip".to_owned()),
        "re-hovering within the skip window opens with no delay"
    );
}

#[test]
fn tooltip_content_is_not_interactive() {
    let mut ui = Ui::new().with_clock();
    let open = State::new(false);
    let tree = tooltip(&open);
    ui.render(tree());
    let trigger = trigger_of(&ui);

    ui.activate_over(trigger);
    ui.advance(DELAY);
    ui.render(tree());

    let outlet = ui.children()[1];
    let content = ui.children_of(outlet)[0];
    let pickable = ui
        .get::<Pickable>(content)
        .expect("tooltip content carries Pickable");
    assert!(!pickable.is_hoverable, "tooltip content ignores picking");
}
