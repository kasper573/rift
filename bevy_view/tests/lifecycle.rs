mod harness;

use bevy_view::{each, node, show};
use harness::{Ui, log};

#[test]
fn on_mount_fires_once_when_an_element_appears() {
    let mut ui = Ui::new();
    ui.render(node().on_mount(|w| log(w, "mount")));
    assert_eq!(ui.log(), vec!["mount".to_owned()]);
}

#[test]
fn on_mount_does_not_fire_again_while_the_element_persists() {
    let mut ui = Ui::new();
    ui.render(node().on_mount(|w| log(w, "mount")));
    ui.render(node().on_mount(|w| log(w, "mount")));
    ui.render(node().on_mount(|w| log(w, "mount")));
    assert_eq!(
        ui.log(),
        vec!["mount".to_owned()],
        "mount is for first appearance only"
    );
}

#[test]
fn on_cleanup_fires_once_when_an_element_is_removed() {
    let mut ui = Ui::new();
    ui.render(show(|_| true, node().on_cleanup(|w| log(w, "cleanup"))));
    assert_eq!(ui.log(), Vec::<String>::new());
    ui.render(show(|_| false, node().on_cleanup(|w| log(w, "cleanup"))));
    assert_eq!(ui.log(), vec!["cleanup".to_owned()]);
}

#[test]
fn cleanup_fires_when_a_for_key_disappears() {
    let mut ui = Ui::new();
    let items = |present: &'static [u64]| {
        each(
            move |_| present.to_vec(),
            |&id| id,
            |&id| node().on_cleanup(move |w| log(w, format!("cleanup {id}"))),
        )
    };
    ui.render(items(&[1, 2, 3]));
    assert_eq!(ui.log(), Vec::<String>::new());
    ui.render(items(&[1, 3]));
    assert_eq!(ui.log(), vec!["cleanup 2".to_owned()]);
}

#[test]
fn tearing_down_the_host_cleans_up_every_descendant() {
    let mut ui = Ui::new();
    ui.render(
        node()
            .on_cleanup(|w| log(w, "outer"))
            .child(node().on_cleanup(|w| log(w, "inner"))),
    );
    let host = ui.host();
    ui.world().entity_mut(host).despawn();
    ui.world().flush();
    let mut log = ui.log();
    log.sort();
    assert_eq!(log, vec!["inner".to_owned(), "outer".to_owned()]);
}

#[test]
fn re_rendering_does_not_re_run_cleanup_for_surviving_elements() {
    let mut ui = Ui::new();
    ui.render(node().on_cleanup(|w| log(w, "cleanup")));
    ui.render(node().on_cleanup(|w| log(w, "cleanup")));
    assert_eq!(
        ui.log(),
        Vec::<String>::new(),
        "a surviving element must not clean up"
    );
}

#[test]
fn a_flapping_show_mounts_and_cleans_up_each_time() {
    let mut ui = Ui::new();
    let body = || {
        node()
            .on_mount(|w| log(w, "mount"))
            .on_cleanup(|w| log(w, "cleanup"))
    };
    let toggle = |on: bool, build: fn() -> bevy_view::Element| show(move |_| on, build());
    ui.render(toggle(true, body));
    ui.render(toggle(false, body));
    ui.render(toggle(true, body));
    ui.render(toggle(false, body));
    assert_eq!(
        ui.log(),
        vec![
            "mount".to_owned(),
            "cleanup".to_owned(),
            "mount".to_owned(),
            "cleanup".to_owned(),
        ]
    );
}
