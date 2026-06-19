//! Disclosure components are controlled: the app owns `open`, the component mounts its content as
//! control flow while open, and a trigger requests the next value through `on_open_change`. The root is
//! a context-providing node, so its trigger sits one level in.

mod harness;

use bevy_view::view;
use harness::{State, Ui};
use ui::{
    AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogOutlet, AlertDialogTrigger,
    Collapsible, CollapsibleContent, CollapsibleTrigger, Dialog, DialogContent, DialogOutlet,
    DialogOverlay, DialogTrigger,
};

#[test]
fn collapsible_trigger_toggles_open() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = {
        let open = open.clone();
        move || {
            let set = open.clone();
            view! {
                <Collapsible open={open.get()} on_open_change={move |_w, value| set.set(value)}>
                    <CollapsibleTrigger><text>"head"</text></CollapsibleTrigger>
                    <CollapsibleContent><text>"body"</text></CollapsibleContent>
                </Collapsible>
            }
        }
    };

    // The body stays mounted (it collapses by height, not by unmounting); the trigger drives `open`.
    ui.render(tree());
    assert!(
        ui.texts().contains(&"body".to_owned()),
        "content is mounted"
    );

    let root = ui.children()[0];
    let trigger = ui.children_of(root)[0];
    ui.activate_click(trigger);
    assert!(open.get(), "the trigger requests open=true");
    ui.render(tree());

    ui.activate_click(trigger);
    assert!(!open.get(), "the trigger requests open=false");
}

#[test]
fn dialog_opens_from_its_trigger_and_closes_on_the_backdrop() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = {
        let open = open.clone();
        move || {
            let set = open.clone();
            view! {
                <Dialog open={open.get()} on_open_change={move |_w, value| set.set(value)}>
                    <DialogTrigger><text>"open"</text></DialogTrigger>
                    <DialogOverlay/>
                    <DialogContent><text>"panel"</text></DialogContent>
                </Dialog>
                <DialogOutlet/>
            }
        }
    };

    ui.render(tree());
    assert!(!ui.texts().contains(&"panel".to_owned()), "starts closed");

    let root = ui.children()[0];
    let trigger = ui.children_of(root)[0];
    ui.activate_click(trigger);
    ui.render(tree());
    assert!(
        ui.texts().contains(&"panel".to_owned()),
        "opens into the outlet"
    );

    let outlet = ui.children()[1];
    let overlay = ui.children_of(outlet)[0];
    ui.activate_click(overlay);
    // The close eases out: a render lets the root notice the edge and hold the content, then settling
    // past the exit and rendering again drops it.
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert!(
        !ui.texts().contains(&"panel".to_owned()),
        "backdrop press closes it"
    );
}

#[test]
fn alert_dialog_closes_only_through_its_actions() {
    let mut ui = Ui::new();
    let open = State::new(false);
    let tree = {
        let open = open.clone();
        move || {
            let set = open.clone();
            view! {
                <AlertDialog open={open.get()} on_open_change={move |_w, value| set.set(value)}>
                    <AlertDialogTrigger><text>"delete"</text></AlertDialogTrigger>
                    <AlertDialogContent>
                        <text>"sure?"</text>
                        <AlertDialogCancel><text>"no"</text></AlertDialogCancel>
                    </AlertDialogContent>
                </AlertDialog>
                <AlertDialogOutlet/>
            }
        }
    };

    ui.render(tree());
    let root = ui.children()[0];
    let trigger = ui.children_of(root)[0];
    ui.activate_click(trigger);
    ui.render(tree());
    assert!(ui.texts().contains(&"sure?".to_owned()), "opens");

    let outlet = ui.children()[1];
    let content = ui.children_of(outlet)[0];
    let cancel = ui.children_of(content)[1];
    ui.activate_click(cancel);
    ui.render(tree());
    ui.settle();
    ui.render(tree());
    assert!(!ui.texts().contains(&"sure?".to_owned()), "cancel closes");
}
