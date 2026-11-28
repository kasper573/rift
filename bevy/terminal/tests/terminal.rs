use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_terminal::{
    EMBED, LineKind, LineSpeaker, TerminalChannel, TerminalCommanded, TerminalEmbed, TerminalInput,
    TerminalLine, TerminalSaid, best_matches, hear, register, reply, say,
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
    send_on(world, conn, TerminalChannel::default(), text);
}

fn send_on(world: &mut World, conn: Entity, channel: TerminalChannel, text: &str) {
    send_holding(world, conn, channel, text, &[]);
}

fn send_holding(
    world: &mut World,
    conn: Entity,
    channel: TerminalChannel,
    text: &str,
    embeds: &[TerminalEmbed],
) {
    hear(
        world,
        conn,
        &TerminalInput {
            text: text.to_owned(),
            channel,
            embeds: embeds.to_vec(),
        },
    );
}

fn thing(key: &str) -> TerminalEmbed {
    TerminalEmbed {
        kind: "item".to_owned(),
        key: key.to_owned(),
    }
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
fn a_typed_line_is_said_trimmed_on_its_channel_or_run_as_a_command_and_answered_where_it_was_typed()
{
    {
        let mut app = server();
        let world = app.world_mut();
        let conn = connect(world);

        send(world, conn, "  hello there  ");
        send(world, conn, "   ");

        assert_eq!(
            said(world),
            vec![TerminalSaid {
                conn,
                channel: TerminalChannel::default(),
                text: "hello there".to_owned(),
                embeds: Vec::new(),
            }]
        );
        assert!(replies(world).is_empty());
    }
    {
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
    {
        let mut app = server();
        let world = app.world_mut();
        let (conn, other) = (connect(world), connect(world));
        let party = TerminalChannel(1);

        send_on(world, conn, party, "on my way");
        send_on(world, conn, party, "/wave");
        reply(world, conn, "You wave.");
        reply(world, other, "Nothing typed yet.");

        assert_eq!(
            said(world),
            vec![TerminalSaid {
                conn,
                channel: party,
                text: "on my way".to_owned(),
                embeds: Vec::new(),
            }]
        );
        let line = |text: &str, channel| TerminalLine {
            text: text.to_owned(),
            kind: LineKind::Notice,
            channel,
            embeds: Vec::new(),
        };
        assert_eq!(
            replies(world),
            vec![
                (conn, line("You wave.", party)),
                (
                    other,
                    line("Nothing typed yet.", TerminalChannel::default())
                ),
            ],
            "an answer lands where its question was typed, and on the first channel before any was"
        );
    }
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
fn a_line_holds_a_thing_at_every_mark_of_it_or_is_not_taken_and_each_stays_with_its_line() {
    let mut app = server();
    let world = app.world_mut();
    let conn = connect(world);
    let channel = TerminalChannel::default();
    let (sword, shield, helm) = (thing("sword"), thing("shield"), thing("helm"));

    send_holding(
        world,
        conn,
        channel,
        &format!("selling {EMBED} cheap"),
        std::slice::from_ref(&sword),
    );
    send_holding(world, conn, channel, &format!("selling {EMBED} cheap"), &[]);
    send_holding(
        world,
        conn,
        channel,
        "selling nothing",
        std::slice::from_ref(&sword),
    );

    assert_eq!(
        said(world),
        vec![TerminalSaid {
            conn,
            channel,
            text: format!("selling {EMBED} cheap"),
            embeds: vec![sword.clone()],
        }],
        "a line whose marks and things do not match one to one is not taken"
    );

    let speaker = LineSpeaker {
        name: "trader".to_owned(),
        typist: None,
    };
    say(
        world,
        &[conn],
        channel,
        speaker.clone(),
        format!("a {EMBED} today\nand a {EMBED} with a {EMBED}"),
        &[sword.clone(), shield.clone(), helm.clone()],
    );

    let line = |text: String, embeds| TerminalLine {
        text,
        kind: LineKind::Said(speaker.clone()),
        channel,
        embeds,
    };
    assert_eq!(
        replies(world),
        vec![
            (conn, line(format!("a {EMBED} today"), vec![sword])),
            (
                conn,
                line(format!("and a {EMBED} with a {EMBED}"), vec![shield, helm])
            ),
        ],
        "what is said over several lines reaches a terminal with each thing in the line that marks it"
    );
}
