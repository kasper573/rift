use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::{TerminalInput, TerminalLine, TerminalSaid, hear, register};

fn server() -> App {
    let mut app = App::new();
    register(&mut app, |world, conn, line| {
        world.resource_mut::<Delivered>().0.push((conn, line));
    });
    app.init_resource::<Delivered>();
    app
}

#[derive(Resource, Default)]
struct Delivered(Vec<(Entity, TerminalLine)>);

fn connect(world: &mut World) -> Entity {
    world.spawn_empty().id()
}

fn send(world: &mut World, conn: Entity, text: &str) {
    hear(
        world,
        conn,
        &TerminalInput {
            text: text.to_owned(),
        },
    );
}

fn said(world: &mut World) -> Vec<TerminalSaid> {
    world
        .resource_mut::<Messages<TerminalSaid>>()
        .drain()
        .collect()
}

fn replies(world: &mut World) -> Vec<(Entity, TerminalLine)> {
    std::mem::take(&mut world.resource_mut::<Delivered>().0)
}

#[test]
fn chat_line_is_said_trimmed_and_not_replied_to_and_a_blank_one_is_not_said() {
    let mut app = server();
    let world = app.world_mut();
    let conn = connect(world);

    send(world, conn, "  hello there  ");
    send(world, conn, "   ");

    assert_eq!(
        said(world),
        vec![TerminalSaid {
            conn,
            text: "hello there".to_owned()
        }]
    );
    assert!(replies(world).is_empty());
}
