use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::{
    CommandCtx, LineTone, TerminalCommand, TerminalInput, TerminalLine, TerminalSaid, hear,
    register,
};

const SHOUT: TerminalCommand = TerminalCommand {
    name: "shout",
    description: "Say it loudly",
    run: shout,
};

fn shout(_world: &mut World, ctx: &CommandCtx) -> Result<String, String> {
    if ctx.args.is_empty() {
        return Err("nothing to shout".to_owned());
    }
    Ok(ctx.args.to_uppercase())
}

fn server(commands: &[TerminalCommand]) -> App {
    let mut app = App::new();
    register(&mut app, commands, |world, conn, line| {
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

fn line(text: &str, tone: LineTone) -> TerminalLine {
    TerminalLine {
        text: text.to_owned(),
        tone,
    }
}

#[test]
fn chat_line_is_said_trimmed_and_not_replied_to_and_a_blank_one_is_not_said() {
    let mut app = server(&[]);
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

#[test]
fn commands_lists_every_command_to_the_sender() {
    let mut app = server(&[SHOUT]);
    let world = app.world_mut();
    let conn = connect(world);

    send(world, conn, "/commands");

    assert!(said(world).is_empty());
    assert_eq!(
        replies(world),
        vec![
            (
                conn,
                line(
                    "/commands — List the commands available to you.",
                    LineTone::Said
                )
            ),
            (conn, line("/shout — Say it loudly", LineTone::Said)),
        ]
    );
}

#[test]
fn registered_command_runs_with_its_arguments() {
    let mut app = server(&[SHOUT]);
    let world = app.world_mut();
    let conn = connect(world);

    send(world, conn, "/shout hello there");

    assert!(said(world).is_empty());
    assert_eq!(
        replies(world),
        vec![(conn, line("HELLO THERE", LineTone::Said))]
    );
}

#[test]
fn a_failing_or_unknown_command_replies_as_failure() {
    let mut app = server(&[SHOUT]);
    let world = app.world_mut();
    let conn = connect(world);

    send(world, conn, "/shout");
    send(world, conn, "/nope");

    assert!(said(world).is_empty());
    assert_eq!(
        replies(world),
        vec![
            (conn, line("nothing to shout", LineTone::Failed)),
            (
                conn,
                line("Unknown command /nope — try /commands", LineTone::Failed)
            ),
        ]
    );
}
