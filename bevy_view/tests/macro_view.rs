//! The `view!` macro is sugar over the runtime builder: every contract proven against the builder
//! must hold when the same UI is written as markup. These tests author UI the way a game would and
//! assert on the rendered tree.

mod harness;

use bevy_ecs::prelude::*;
use bevy_view::view;
use harness::{Ui, log};

#[derive(Resource)]
struct Flag(bool);

#[test]
fn a_self_closing_node_mounts_one_child() {
    let mut ui = Ui::new();
    ui.render(view! { <node/> });
    assert_eq!(ui.child_count(), 1);
}

#[test]
fn a_static_text_element_renders_its_literal() {
    let mut ui = Ui::new();
    ui.render(view! { <text>"hello"</text> });
    assert_eq!(ui.texts(), vec!["hello".to_owned()]);
}

#[test]
fn nested_markup_renders_in_order() {
    let mut ui = Ui::new();
    ui.render(view! {
        <node>
            <text>"a"</text>
            <node><text>"b"</text></node>
        </node>
    });
    assert_eq!(ui.texts(), vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn multiple_roots_become_a_fragment() {
    let mut ui = Ui::new();
    ui.render(view! {
        <text>"first"</text>
        <text>"second"</text>
    });
    assert_eq!(ui.child_count(), 2);
    assert_eq!(ui.texts(), vec!["first".to_owned(), "second".to_owned()]);
}

#[test]
fn an_on_click_attribute_wires_the_handler() {
    let mut ui = Ui::new();
    ui.render(view! { <node on:click={|w| log(w, "hit")}/> });
    let button = ui.children()[0];
    ui.activate_click(button);
    assert_eq!(ui.log(), vec!["hit".to_owned()]);
}

#[test]
fn mount_and_cleanup_attributes_fire() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(true));
    let view = || {
        view! {
            <Show when={|w: &World| w.resource::<Flag>().0}>
                <node on:mount={|w| log(w, "mount")} on:cleanup={|w| log(w, "cleanup")}/>
            </Show>
        }
    };
    ui.render(view());
    ui.world().insert_resource(Flag(false));
    ui.render(view());
    assert_eq!(ui.log(), vec!["mount".to_owned(), "cleanup".to_owned()]);
}

#[test]
fn a_show_element_reveals_its_body_when_true() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(false));
    let view = || {
        view! {
            <Show when={|w: &World| w.resource::<Flag>().0}>
                <text>"visible"</text>
            </Show>
        }
    };
    ui.render(view());
    assert_eq!(ui.child_count(), 0);
    ui.world().insert_resource(Flag(true));
    ui.render(view());
    assert_eq!(ui.texts(), vec!["visible".to_owned()]);
}

#[test]
fn a_hide_element_is_the_inverse_of_show() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(true));
    let view = || {
        view! {
            <Hide when={|w: &World| w.resource::<Flag>().0}>
                <text>"shown when false"</text>
            </Hide>
        }
    };
    ui.render(view());
    assert_eq!(ui.child_count(), 0, "hidden while the flag is true");
    ui.world().insert_resource(Flag(false));
    ui.render(view());
    assert_eq!(ui.texts(), vec!["shown when false".to_owned()]);
}

#[test]
fn a_for_element_renders_one_child_per_item() {
    let mut ui = Ui::new();
    ui.render(view! {
        <For each={|_| vec![1u64, 2, 3]} key={|id| *id} let={id}>
            <text>{ id.to_string() }</text>
        </For>
    });
    assert_eq!(
        ui.texts(),
        vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]
    );
}

#[test]
fn dynamic_text_from_a_closure_reads_the_world() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(true));
    let view = || {
        view! {
            <text>{ |w: &World| if w.resource::<Flag>().0 { "on".to_owned() } else { "off".to_owned() } }</text>
        }
    };
    ui.render(view());
    assert_eq!(ui.texts(), vec!["on".to_owned()]);
    ui.world().insert_resource(Flag(false));
    ui.render(view());
    assert_eq!(ui.texts(), vec!["off".to_owned()]);
}

#[test]
fn an_embedded_expression_composes_a_component_function() {
    fn badge(label: &str) -> bevy_view::View {
        view! { <node><text>{ label.to_owned() }</text></node> }
    }
    let mut ui = Ui::new();
    ui.render(view! {
        <node>
            { badge("one") }
            { badge("two") }
        </node>
    });
    assert_eq!(ui.texts(), vec!["one".to_owned(), "two".to_owned()]);
}
