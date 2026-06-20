mod harness;

use bevy_view::view;
use harness::{State, Ui};
use ui::{Popover, PopoverClose, PopoverContent, PopoverOutlet, PopoverTrigger, dismiss_overlays};

fn popover(open: &State<bool>) -> impl Fn() -> bevy_view::View {
    let open = open.clone();
    move || {
        let set = open.clone();
        view! {
            <Popover open={open.get()} on_open_change={move |_w, value| set.set(value)}>
                <PopoverTrigger><text>"open"</text></PopoverTrigger>
                <PopoverContent>
                    <text>"panel"</text>
                    <PopoverClose><text>"close"</text></PopoverClose>
                </PopoverContent>
            </Popover>
            <PopoverOutlet/>
        }
    }
}

#[test]
fn a_popover_is_closed_until_its_trigger_is_clicked() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = popover(&open);

    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["open".to_owned()],
        "content is absent while closed"
    );

    let trigger = ui.children_of(ui.children()[0])[0];
    ui.activate_click(trigger);
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["open".to_owned(), "panel".to_owned(), "close".to_owned()],
        "clicking the trigger mounts the content into the outlet"
    );
}

#[test]
fn clicking_the_trigger_again_closes_the_popover() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = popover(&open);

    ui.render(tree());
    let trigger = ui.children_of(ui.children()[0])[0];
    ui.activate_click(trigger);
    ui.render(tree());
    ui.activate_click(trigger);
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["open".to_owned()],
        "a second click unmounts the content"
    );
}

#[test]
fn popover_close_dismisses_the_popover() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = popover(&open);

    ui.render(tree());
    let trigger = ui.children_of(ui.children()[0])[0];
    ui.activate_click(trigger);
    ui.render(tree());

    let outlet = ui.children()[1];
    let content = ui.children_of(outlet)[0];
    let close = ui.children_of(content)[1];
    ui.activate_click(close);
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["open".to_owned()],
        "PopoverClose closes the popover"
    );
}

#[test]
fn a_press_outside_dismisses_an_open_popover() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = popover(&open);

    ui.render(tree());
    let trigger = ui.children_of(ui.children()[0])[0];
    ui.activate_click(trigger);
    ui.render(tree());

    dismiss_overlays(ui.world(), None);
    ui.world().flush();
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert_eq!(
        ui.texts(),
        vec!["open".to_owned()],
        "a press in empty space closes it"
    );
}

#[test]
fn a_press_inside_the_popover_keeps_it_open() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = popover(&open);

    ui.render(tree());
    let trigger = ui.children_of(ui.children()[0])[0];
    ui.activate_click(trigger);
    ui.render(tree());

    dismiss_overlays(ui.world(), Some(trigger));
    ui.world().flush();
    ui.render(tree());
    assert!(
        ui.texts().contains(&"panel".to_owned()),
        "a press carrying the popover's own instance does not dismiss it"
    );
}
