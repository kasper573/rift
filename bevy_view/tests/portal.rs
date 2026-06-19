//! Portals render their body into an app-placed outlet instead of in place: the content is absent
//! where declared and present under the outlet, keeps stable identity across renders, unmounts (with
//! cleanup) when its source goes away, and an outlet's children are never churned by a re-render.

mod harness;

use bevy_ecs::prelude::*;
use bevy_view::{PortalKind, View, boundary, node, outlet, portal, text, view};
use harness::{Ui, log};

const SINK: PortalKind = PortalKind(42);

#[derive(Resource)]
struct Flag(bool);

#[test]
fn portaled_content_renders_under_the_outlet_not_in_place() {
    let mut ui = Ui::new();
    ui.render(view! {
        <node>{ portal(SINK, text("floating")) }</node>
        { outlet(SINK) }
    });
    let source = ui.children()[0];
    assert_eq!(
        ui.children_of(source).len(),
        0,
        "the portal body is not rendered where it is declared"
    );
    let sink = ui.children()[1];
    assert_eq!(ui.texts_under(sink), vec!["floating".to_owned()]);
}

#[test]
fn the_portaled_entity_keeps_identity_across_renders() {
    let mut ui = Ui::new();
    let view = || {
        view! {
            <node>{ portal(SINK, text("x")) }</node>
            { outlet(SINK) }
        }
    };
    ui.render(view());
    let before = ui.children_of(ui.children()[1])[0];
    ui.render(view());
    let after = ui.children_of(ui.children()[1])[0];
    assert_eq!(
        before, after,
        "a stable portal keeps its entity (and retained state) across renders"
    );
}

#[test]
fn re_rendering_never_churns_the_outlet_children() {
    let mut ui = Ui::new();
    let view = || {
        view! {
            <node>{ portal(SINK, node().on_cleanup(|w| log(w, "churn"))) }</node>
            { outlet(SINK) }
        }
    };
    ui.render(view());
    ui.render(view());
    ui.render(view());
    assert_eq!(
        ui.log(),
        Vec::<String>::new(),
        "stable portal content is never torn down by a re-render"
    );
}

#[test]
fn sibling_portals_sharing_a_boundary_reach_one_outlet_without_colliding() {
    let mut ui = Ui::new();
    let view = || {
        View::fragment([
            boundary(node().children([portal(SINK, text("a")), portal(SINK, text("b"))])),
            outlet(SINK).into(),
        ])
    };
    ui.render(view());
    let sink = ui.children()[1];
    let mut texts = ui.texts_under(sink);
    texts.sort();
    assert_eq!(
        texts,
        vec!["a".to_owned(), "b".to_owned()],
        "two portals that share their boundary's instance each reach the outlet on their own path"
    );

    // A second render used to reuse one entity for both bodies (a dialog's overlay and content share a
    // boundary), listing it twice under the outlet and panicking the layout tree; it must stay stable.
    ui.render(view());
    let mut again = ui.texts_under(sink);
    again.sort();
    assert_eq!(again, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn removing_the_source_unmounts_the_portaled_content() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(true));
    let view = || {
        view! {
            <Show when={|w: &World| w.resource::<Flag>().0}>
                { portal(SINK, node().on_cleanup(|w| log(w, "gone")).child(text("floating"))) }
            </Show>
            { outlet(SINK) }
        }
    };
    ui.render(view());
    assert_eq!(
        ui.texts(),
        vec!["floating".to_owned()],
        "content mounts into the outlet while its source is present"
    );

    ui.world().insert_resource(Flag(false));
    ui.render(view());
    assert_eq!(
        ui.texts(),
        Vec::<String>::new(),
        "content unmounts from the outlet when its source goes away"
    );
    assert_eq!(
        ui.log(),
        vec!["gone".to_owned()],
        "cleanup fires exactly once"
    );
}
