//! `Bind` is the composable behavior primitive: an `Element -> Element` decorator applied with a
//! repeatable `use=`. Repeated binders apply in order, and because events fan out, two binders that
//! touch the same event both run.

mod harness;

use bevy_view::{Bind, node, view};
use harness::{Ui, log};

fn click_logging(message: &'static str) -> Bind {
    Bind::new(move |element| element.on_click(move |w| log(w, message)))
}

fn over_logging(message: &'static str) -> Bind {
    Bind::new(move |element| element.on_over(move |w| log(w, message)))
}

#[test]
fn a_bind_decorates_the_element() {
    let mut ui = Ui::new();
    ui.render(node().bind(click_logging("bound")));
    let entity = ui.children()[0];
    ui.activate_click(entity);
    assert_eq!(ui.log(), vec!["bound".to_owned()]);
}

#[test]
fn repeated_use_applies_each_binder_in_order() {
    let mut ui = Ui::new();
    ui.render(view! { <node use={click_logging("a")} use={click_logging("b")}/> });
    let entity = ui.children()[0];
    ui.activate_click(entity);
    assert_eq!(ui.log(), vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn binders_fan_out_on_a_shared_event() {
    let mut ui = Ui::new();
    ui.render(view! { <node use={over_logging("x")} use={over_logging("y")}/> });
    let entity = ui.children()[0];
    ui.activate_over(entity);
    assert_eq!(ui.log(), vec!["x".to_owned(), "y".to_owned()]);
}
