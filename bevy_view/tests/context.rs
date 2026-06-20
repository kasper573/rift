mod harness;

use bevy_view::{context, node, provide};
use harness::{Ui, log};

#[derive(Clone)]
struct Theme(&'static str);

#[test]
fn context_resolves_the_nearest_provider() {
    let mut ui = Ui::new();
    ui.render(
        node()
            .bind(provide(Theme("dark")))
            .child(node().on_mount_with(|world, entity| {
                let theme = context::<Theme>(world, entity).unwrap().0;
                log(world, theme);
            })),
    );
    assert_eq!(ui.log(), vec!["dark".to_owned()]);
}

#[test]
fn an_inner_provider_shadows_an_outer_one() {
    let mut ui = Ui::new();
    ui.render(
        node()
            .bind(provide(Theme("outer")))
            .child(
                node()
                    .bind(provide(Theme("inner")))
                    .child(node().on_mount_with(|world, entity| {
                        log(world, context::<Theme>(world, entity).unwrap().0);
                    })),
            ),
    );
    assert_eq!(ui.log(), vec!["inner".to_owned()]);
}

#[test]
fn an_unprovided_type_resolves_to_none() {
    let mut ui = Ui::new();
    ui.render(node().on_mount_with(|world, entity| {
        let present = context::<Theme>(world, entity).is_some();
        log(world, if present { "some" } else { "none" });
    }));
    assert_eq!(ui.log(), vec!["none".to_owned()]);
}
