use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::{
    TerminalCommanded, TerminalInput, TerminalLine, TerminalSaid, best_matches, hear, register,
};

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
fn a_line_starting_with_the_prefix_is_a_command_to_run_rather_than_said() {
    let mut app = server();
    let world = app.world_mut();
    let conn = connect(world);

    send(world, conn, " /wave at everyone ");

    assert!(said(world).is_empty());
    let commanded: Vec<TerminalCommanded> = world
        .resource_mut::<Messages<TerminalCommanded>>()
        .drain()
        .collect();
    assert_eq!(
        commanded,
        vec![TerminalCommanded {
            conn,
            line: "wave at everyone".to_owned()
        }]
    );
}

#[test]
fn what_is_typed_matches_the_labels_it_runs_through_best_first_and_no_more_than_asked() {
    let labels = [
        "Health Potion",
        "Plank",
        "Copper Pickaxe",
        "Greater Health Potion",
    ];
    let best = |typed: &str, most: usize| -> Vec<&str> {
        best_matches(typed, labels, most)
            .into_iter()
            .map(|place| labels[place])
            .collect()
    };

    assert_eq!(best("", 2), ["Health Potion", "Plank"]);
    assert_eq!(best("hp", 5), ["Health Potion", "Greater Health Potion"]);
    assert_eq!(best("PICK", 5), ["Copper Pickaxe"]);
    assert_eq!(best("p", 1).len(), 1);
    assert!(best("xyz", 5).is_empty());
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
