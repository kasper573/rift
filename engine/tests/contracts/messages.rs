use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::content::{Content, ContentScope};
use engine::core::store::RecordRow;
use engine::core::time::UnixMillis;
use engine::systems::account::AccountId;
use engine::systems::friend::{self, AskFriendship};
use engine::systems::inbox::{
    DeleteMessage, InboxView, MessageAnswered, MessageText, OpenMessage, SendMessage, TakeAttached,
};
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::payout::{self, PayoutReason};
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, JoinOrigin};
use engine::systems::player::{self};
use engine::systems::save::SavedCharacter;
use engine::systems::save::load;
use engine::systems::social_request::RequestAnswer;
use serde_json::json;
use testkit::{Game, Map};

use crate::saves::{Saves, checkpoint};
use crate::server::Server;

#[test]
fn a_message_carries_what_is_attached_from_one_bag_to_another() {
    let mut server = Server::of(game(), vec![friend::request_source()]);
    let content = server.village.world.resource::<Content>().clone();
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "forest");
    befriend(&mut server, 1, 2, "Hale");
    let (lure, rod, plank) = (
        item(&content, "lure"),
        item(&content, "rod"),
        item(&content, "plank"),
    );
    carry(&mut server, 1, &[lure(6), rod(1)]);
    carry(&mut server, 2, &[plank(50)]);
    let mut answered = MessageCursor::default();
    let mut texts = MessageCursor::default();

    write(
        &mut server,
        1,
        "Hale",
        " Lures, as promised ",
        &[lure(4), rod(1)],
    );
    assert_eq!(
        heard::<MessageAnswered>(&mut server, 1, &mut answered),
        [MessageAnswered { sent: true }]
    );
    assert_eq!(
        bag(&server, 1),
        [lure(2)],
        "what is attached leaves the sender's bag"
    );
    assert_eq!(
        server.told(1).last().map(String::as_str),
        Some("Message sent to Hale")
    );
    assert_eq!(
        server.told(2).last().map(String::as_str),
        Some("Wren wrote to you: Lures, as promised.  opens it"),
        "the receiver is told at once, in whatever world they are"
    );
    let inbox = whole_inbox(&mut server, 2);
    let letter = inbox.rows[0].clone();
    assert_eq!(
        (inbox.whole, inbox.held, inbox.unread(), inbox.rows.len()),
        (true, 1, 1, 1)
    );
    assert_eq!(
        (
            letter.title.as_str(),
            letter.from.as_ref().map(|from| from.name.as_str()),
            letter.read,
            letter.attached.clone()
        ),
        (
            "Lures, as promised",
            Some("Wren"),
            false,
            vec![lure(4), rod(1)]
        )
    );

    server.send(2, OpenMessage(letter.id));
    server.step();
    assert_eq!(
        heard::<MessageText>(&mut server, 2, &mut texts),
        [MessageText {
            id: letter.id,
            text: "Six feathers and the old rod.".to_owned()
        }],
        "its text is sent as it is opened"
    );
    assert_eq!(
        inbox_of(&mut server, 2).unread(),
        0,
        "and opening it reads it"
    );
    assert_eq!(bag(&server, 2), [plank(50)], "which takes nothing");

    server.send(2, DeleteMessage(letter.id));
    server.step();
    assert_eq!(
        inbox_of(&mut server, 2).rows.len(),
        1,
        "a message is not deleted while anything is attached"
    );
    let take = |stack| TakeAttached {
        message: letter.id,
        stack,
    };
    server.send(2, take(None));
    server.step();
    assert_eq!(
        (
            bag(&server, 2),
            inbox_of(&mut server, 2).rows[0].attached.clone()
        ),
        (vec![plank(50), lure(4)], vec![rod(1)]),
        "each stack is taken on its own, and what finds no room stays attached"
    );
    assert_eq!(
        server.forest.errors(2),
        ["1 of 2 taken. The rest found no room and waits here"]
    );
    server.send(2, take(Some(0)));
    server.step();
    assert_eq!(
        server.forest.errors(2).last().map(String::as_str),
        Some("Needs 1 free slot")
    );
    carry(&mut server, 2, &[lure(4)]);
    server.send(2, take(Some(0)));
    server.step();
    assert_eq!(bag(&server, 2), [lure(4), rod(1)]);
    assert_eq!(inbox_of(&mut server, 2).rows[0].attached, []);

    server.send(2, DeleteMessage(letter.id));
    server.step();
    let inbox = inbox_of(&mut server, 2);
    assert_eq!(
        (inbox.rows.len(), inbox.held),
        (0, 0),
        "then it is deleted for good"
    );
}

#[test]
fn a_message_that_cannot_be_sent_moves_nothing() {
    let mut server = Server::of(game(), vec![friend::request_source()]);
    let content = server.village.world.resource::<Content>().clone();
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "village");
    befriend(&mut server, 1, 2, "Hale");
    let (lure, map) = (item(&content, "lure"), item(&content, "map"));
    carry(&mut server, 1, &[lure(6), map(1)]);
    let mut answered = MessageCursor::default();
    let mut refused =
        |server: &mut Server, to: &str, title: &str, attached: &[ItemStack], why: &str| {
            server.village.forget_notified();
            write(server, 1, to, title, attached);
            assert_eq!(server.village.errors(1), [why]);
            assert_eq!(
                heard::<MessageAnswered>(server, 1, &mut answered),
                [MessageAnswered { sent: false }],
                "{why}"
            );
            assert_eq!(bag(server, 1), [lure(6), map(1)], "{why}");
            assert_eq!(
                inbox_of(server, 2).rows.len() + inbox_of(server, 3).rows.len(),
                0,
                "{why}"
            );
        };
    refused(&mut server, "Hale", "  ", &[], "Give it a title");
    refused(
        &mut server,
        "Marrow",
        "Hello",
        &[lure(1)],
        "You can only write to friends and party members",
    );
    refused(
        &mut server,
        "Hale",
        "A map",
        &[map(1)],
        "Tattered Map stays with you",
    );
    refused(
        &mut server,
        "Hale",
        "Lures",
        &[lure(7)],
        "Needs 1 more Feather Lure",
    );

    write(&mut server, 1, "Hale", "One", &[]);
    write(&mut server, 1, "Hale", "Two", &[]);
    server.village.forget_notified();
    write(&mut server, 1, "Hale", "Three", &[]);
    assert_eq!(
        server.village.errors(1),
        ["Hale's inbox is full"],
        "an inbox holds only so many messages from players"
    );
    let first = inbox_of(&mut server, 2).rows[1].id;
    server.send(2, DeleteMessage(first));
    server.step();
    server.village.forget_notified();
    write(&mut server, 1, "Hale", "Three", &[]);
    assert_eq!(
        server.village.errors(1),
        ["Wait a moment before writing again"],
        "and a player sends only so many within a minute"
    );
    server.later(61.0);
    write(&mut server, 1, "Hale", "Three", &[]);
    let titles: Vec<String> = inbox_of(&mut server, 2)
        .rows
        .into_iter()
        .map(|row| row.title)
        .collect();
    assert_eq!(titles, ["Three", "Two"], "the newest first");
}

#[test]
fn messages_wait_for_whoever_is_gone_and_are_kept_from_one_server_to_the_next() {
    let saves = Saves::in_a_file();
    let content = game().load();
    let lure = item(&content, "lure");
    let mut server = Server::saving(&content, &saves, vec![friend::request_source()]);
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    befriend(&mut server, 1, 2, "Hale");
    carry(&mut server, 1, &[lure(6)]);
    server.village.disconnect(2);
    server.later(31.0);
    write(&mut server, 1, "Hale", "For later", &[lure(2)]);
    write(&mut server, 1, "Hale", "And this", &[]);
    assert_eq!(
        bag(&server, 1),
        [lure(4)],
        "a message reaches a player who is gone"
    );
    let report = checkpoint(
        &mut [&mut server.village, &mut server.forest],
        &mut server.above,
    );
    assert!(report.outcome.is_ok());

    let mut next = Server::saving(&content, &saves, vec![friend::request_source()]);
    next.sign_in(2, "Hale", "forest");
    let inbox = whole_inbox(&mut next, 2);
    let titles: Vec<&str> = inbox.rows.iter().map(|row| row.title.as_str()).collect();
    assert_eq!(
        (titles, inbox.held, inbox.unread()),
        (vec!["And this", "For later"], 2, 2)
    );
    assert_eq!(inbox.rows[1].attached, [lure(2)], "with what was attached");
    next.send(2, OpenMessage(inbox.rows[1].id));
    next.send(2, DeleteMessage(inbox.rows[0].id));
    next.send(
        2,
        TakeAttached {
            message: inbox.rows[1].id,
            stack: None,
        },
    );
    next.step();
    next.sign_in(1, "Wren", "village");
    write(&mut next, 1, "Hale", "A third", &[]);
    let report = checkpoint(&mut [&mut next.village, &mut next.forest], &mut next.above);
    assert!(report.outcome.is_ok());

    let mut last = Server::saving(&content, &saves, vec![friend::request_source()]);
    last.sign_in(2, "Hale", "village");
    let inbox = whole_inbox(&mut last, 2);
    let rows: Vec<(&str, bool, usize)> = inbox
        .rows
        .iter()
        .map(|row| (row.title.as_str(), row.read, row.attached.len()))
        .collect();
    assert_eq!(
        (rows, inbox.held),
        (vec![("A third", false, 0), ("For later", true, 0)], 2),
        "read, taken, deleted and delivered since: all of it is kept, each under an id of its own"
    );
}

#[test]
fn what_a_bag_has_no_room_for_as_its_character_comes_back_is_sent_by_the_game() {
    let roomy = || game().edit_settings(|settings| settings["slots"]["bag"] = json!(3));
    let before = roomy().load();
    let (lure, plank, rod) = (
        item(&before, "lure"),
        item(&before, "plank"),
        item(&before, "rod"),
    );
    let wren = SavedCharacter {
        account: AccountId("Wren".to_owned()),
        saved_at: UnixMillis(1_000_000_000),
        character: CharacterSnapshot {
            bag: vec![lure(6), plank(50), rod(1)].into(),
            ..CharacterSnapshot::of_player(&before, "Wren", player::def(&before))
        },
        told: Vec::new(),
    };
    let saved = RecordRow::of(&wren, &ContentScope::of(&before)).body;
    let smaller = || game().edit_settings(|settings| settings["slots"]["bag"] = json!(2));
    let content = smaller().load();
    let loaded = load::load(&content, &saved).expect("she loads");
    let join_as = JoinAs {
        snapshot: loaded.saved.character,
        start: loaded.start,
        origin: JoinOrigin::Saved {
            at: loaded.saved.saved_at,
            told: loaded.saved.told,
        },
    };

    let mut server = Server::of(smaller(), vec![friend::request_source()]);
    let (lure, plank, rod) = (
        item(&content, "lure"),
        item(&content, "plank"),
        item(&content, "rod"),
    );
    let character = server.sign_in_as(1, "Wren", "village", Some(join_as));
    server.step();
    server.step();
    assert_eq!(bag(&server, 1), [lure(6), plank(50)]);
    let inbox = inbox_of(&mut server, 1);
    let rows: Vec<_> = inbox
        .rows
        .iter()
        .map(|row| (row.title.as_str(), row.from.clone(), row.attached.clone()))
        .collect();
    assert_eq!(
        (rows, inbox.held),
        (
            vec![("What your inventory had no room for", None, vec![rod(1)])],
            0
        ),
        "a message from the game, which counts against no limit"
    );
    let again = SavedCharacter::of_live(&server.village.world, character).expect("her character");
    assert_eq!(
        again.character.bag.set_aside(),
        [],
        "and what was sent is in her character no longer"
    );
}

#[test]
fn a_payout_fills_the_bag_and_sends_what_has_no_room_by_message() {
    let mut server = Server::of(game(), vec![friend::request_source()]);
    let content = server.village.world.resource::<Content>().clone();
    let (lure, plank, rod) = (
        item(&content, "lure"),
        item(&content, "plank"),
        item(&content, "rod"),
    );
    let wren = server.sign_in(1, "Wren", "village");
    carry(&mut server, 1, &[plank(50)]);
    let pay = |server: &mut Server, stacks: &[ItemStack]| {
        let paid = payout::pay(
            &mut server.village.world,
            wren,
            stacks,
            PayoutReason::Given(None),
        );
        server.step();
        paid
    };
    let messages = |server: &mut Server| -> Vec<(String, bool, Vec<ItemStack>)> {
        inbox_of(server, 1)
            .rows
            .into_iter()
            .map(|row| (row.title, row.from.is_none(), row.attached))
            .collect()
    };

    let paid = pay(&mut server, &[lure(6)]);
    assert_eq!(
        (paid.kept, paid.sent, bag(&server, 1), messages(&mut server)),
        (
            vec![lure(6)],
            Vec::new(),
            vec![plank(50), lure(6)],
            Vec::new()
        ),
        "what fits goes into the bag, and nothing is sent"
    );

    let paid = pay(&mut server, &[rod(1), lure(4)]);
    assert_eq!(
        (paid.kept.clone(), paid.sent.clone(), bag(&server, 1)),
        (vec![lure(4)], vec![rod(1)], vec![plank(50), lure(10)]),
        "a stack with no room never holds back the ones after it"
    );
    assert_eq!(
        paid.overflow(&content).map(|line| line.plain_words()),
        Some("No room in your bag for 1 rod: sent to your messages.".to_owned()),
        "the payout's notification tells what went to messages"
    );
    assert_eq!(
        messages(&mut server),
        [("Items from village".to_owned(), true, vec![rod(1)])],
        "the rest comes as a message from the game, named for what paid it"
    );

    carry(&mut server, 1, &[]);
    server.village.disconnect(1);
    let paid = payout::pay(
        &mut server.village.world,
        wren,
        &[lure(2)],
        PayoutReason::Given(None),
    );
    server.step();
    assert_eq!(
        (paid.kept, paid.sent),
        (Vec::new(), vec![lure(2)]),
        "a character whose player is away is paid everything by message"
    );
    server.sign_in(1, "Wren", "village");
    server.step();
    assert_eq!(
        (bag(&server, 1), messages(&mut server).len()),
        (Vec::new(), 2),
        "and the message waits for them, though they left before it was written"
    );
}

fn item(content: &Content, name: &str) -> impl Fn(u32) -> ItemStack + Copy {
    let item = content
        .by_path::<ItemDef>(&testkit::path("item", name))
        .expect("an item of the test game");
    move |count| ItemStack::new(item, count)
}

/// The players of `asker` and `asked` become friends.
fn befriend(server: &mut Server, asker: u32, asked: u32, name: &str) {
    server.send(asker, AskFriendship(server.number(name)));
    server.step();
    server.answer(asked, RequestAnswer::Accept);
    server.step();
}

/// The character of `client` carries `stacks` and nothing else.
fn carry(server: &mut Server, client: u32, stacks: &[ItemStack]) {
    let sim = server.village_or_forest(client);
    let character = sim.player(client);
    let mut bag = sim.world.get_mut::<Inventory>(character).expect("a bag");
    bag.slots = stacks.to_vec();
}

fn bag(server: &Server, client: u32) -> Vec<ItemStack> {
    [&server.village, &server.forest]
        .into_iter()
        .find_map(|sim| {
            let character = *sim
                .world
                .resource::<player::Players>()
                .0
                .get(&player::ClientId(client))?;
            Some(sim.world.get::<Inventory>(character)?.slots.clone())
        })
        .expect("a character with a bag")
}

/// The player of `client` writes to the player called `to`.
fn write(server: &mut Server, client: u32, to: &str, title: &str, attached: &[ItemStack]) {
    let message = SendMessage {
        to: server.number(to),
        title: title.to_owned(),
        text: "Six feathers and the old rod.".to_owned(),
        attached: attached.to_vec(),
    };
    server.send(client, message);
    server.step();
}

/// What the player of `client` sees of their inbox.
fn inbox_of(server: &mut Server, client: u32) -> InboxView {
    let sim = server.village_or_forest(client);
    let character = sim.player(client);
    sim.world
        .get::<InboxView>(character)
        .cloned()
        .unwrap_or_default()
}

/// The inbox of the player of `client`, once the server has all of it in hand.
fn whole_inbox(server: &mut Server, client: u32) -> InboxView {
    for _ in 0..5000 {
        server.step();
        let inbox = inbox_of(server, client);
        if inbox.whole {
            return inbox;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the inbox of client {client} was never read");
}

/// What the client of `client` was sent of `M` since `cursor` last read.
fn heard<M: Message + Clone>(
    server: &mut Server,
    client: u32,
    cursor: &mut MessageCursor<ToClient<M>>,
) -> Vec<M> {
    let sim = server.village_or_forest(client);
    let conn = sim.conn(client);
    sim.read(cursor)
        .into_iter()
        .filter(|sent| sent.client == conn)
        .map(|sent| sent.message)
        .collect()
}

/// A game of two areas with friends and messages, whose inboxes hold two messages from players
/// and whose players send two a minute, with things to carry: one of them bound.
fn game() -> Game {
    let stacked = |max: u32| json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": max }] });
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .item("lure", stacked(50))
        .item("plank", stacked(50))
        .item("rod", json!({ "kind": { "type": "resource" } }))
        .item(
            "map",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "bound" }] }),
        )
        .edit("item", "lure", |lure| lure["name"] = json!("Feather Lure"))
        .edit("item", "map", |map| map["name"] = json!("Tattered Map"))
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["slots"]["bag"] = json!(2);
            settings["friends"] =
                json!({ "most": 100, "requests": 20, "request_lasts": 604_800.0 });
            settings["messages"] = json!({
                "inbox": 2, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
        })
}
