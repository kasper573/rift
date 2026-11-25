use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::content::{Content, ContentScope, Fixture, ImageRef};
use engine::core::store::{Record, RecordRow};
use engine::core::time::UnixMillis;
use engine::systems::account::AccountId;
use engine::systems::friend::{self, AskFriendship};
use engine::systems::interface::InterfaceIcon;
use engine::systems::item::deliver::{self, DeliveryReason};
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::mail::{
    DeleteLetter, InboxSender, InboxView, Letter, LetterAnswered, LetterSender, LetterText,
    OpenLetter, SendLetter, TakeAttached, TakenStack,
};
use engine::systems::notification::{FeedLayout, NotificationKind};
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
        heard::<LetterAnswered>(&mut server, 1, &mut answered),
        [LetterAnswered { sent: true }]
    );
    assert_eq!(
        bag(&server, 1),
        [lure(2)],
        "what is attached leaves the sender's bag"
    );
    assert_eq!(
        server.told(1).last().map(String::as_str),
        Some("Letter sent to Hale")
    );
    assert_eq!(
        server.told(2).last().map(String::as_str),
        Some("Wren: Lures, as promised"),
        "the receiver is told at once, in whatever world they are"
    );
    assert_eq!(
        arrival(&mut server, 2),
        Some((FeedLayout::Corner, mail_icon(&content))),
        "in the corner, under the mail icon"
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
            written_by(&letter.from),
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

    server.send(2, OpenLetter(letter.id));
    server.step();
    assert_eq!(
        heard::<LetterText>(&mut server, 2, &mut texts),
        [LetterText {
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

    server.send(2, DeleteLetter(letter.id));
    server.step();
    assert_eq!(
        inbox_of(&mut server, 2).rows.len(),
        1,
        "a letter is not deleted while anything is attached"
    );
    let take = |stack: Option<(u32, u32)>| TakeAttached {
        letter: letter.id,
        stack: stack.map(|(index, count)| TakenStack { index, count }),
    };
    server.send(2, take(Some((0, 1))));
    server.step();
    assert_eq!(
        (
            bag(&server, 2),
            inbox_of(&mut server, 2).rows[0].attached.clone()
        ),
        (vec![plank(50), lure(1)], vec![lure(3), rod(1)]),
        "some of a stack can be taken, and the rest stays attached"
    );
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
    server.send(2, take(Some((0, 1))));
    server.step();
    assert_eq!(
        server.forest.errors(2).last().map(String::as_str),
        Some("Needs 1 free slot")
    );
    carry(&mut server, 2, &[lure(4)]);
    server.send(2, take(Some((0, 1))));
    server.step();
    assert_eq!(bag(&server, 2), [lure(4), rod(1)]);
    assert_eq!(inbox_of(&mut server, 2).rows[0].attached, []);

    server.send(2, DeleteLetter(letter.id));
    server.step();
    let inbox = inbox_of(&mut server, 2);
    assert_eq!(
        (inbox.rows.len(), inbox.held),
        (0, 0),
        "then it is deleted for good"
    );

    write(
        &mut server,
        1,
        "Hale",
        "Nets mended by the old pier at dawn",
        &[],
    );
    assert_eq!(
        server.told(2).last().map(String::as_str),
        Some("Wren: Nets mended by the old pi…"),
        "a title longer than the notice shows is cut where the game says"
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
                heard::<LetterAnswered>(server, 1, &mut answered),
                [LetterAnswered { sent: false }],
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
        "Tattered Map is bound.",
    );
    refused(
        &mut server,
        "Hale",
        "Lures",
        &[lure(7)],
        "Needs 1 Feather Lure more",
    );

    write(&mut server, 1, "Hale", "One", &[]);
    write(&mut server, 1, "Hale", "Two", &[]);
    server.village.forget_notified();
    write(&mut server, 1, "Hale", "Three", &[]);
    assert_eq!(
        server.village.errors(1),
        ["Hale's inbox is full"],
        "an inbox holds only so many letters from players"
    );
    let first = inbox_of(&mut server, 2).rows[1].id;
    server.send(2, DeleteLetter(first));
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
        "a letter reaches a player who is gone"
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
    next.send(2, OpenLetter(inbox.rows[1].id));
    next.send(2, DeleteLetter(inbox.rows[0].id));
    next.send(
        2,
        TakeAttached {
            letter: inbox.rows[1].id,
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
    let saved_before = |from: serde_json::Value| {
        serde_json::from_value::<Letter>(json!({
            "id": 1, "to": "hale", "from": from, "title": "Kept", "text": "", "sent": 0
        }))
        .expect("a letter saved before loads")
        .from
    };
    assert_eq!(
        (
            saved_before(json!("game")),
            saved_before(json!({ "player": "wren" }))
        ),
        (
            LetterSender::System,
            LetterSender::Character(AccountId("wren".to_owned()))
        ),
        "letters saved when the system was called the game and a character a player still load"
    );
    assert_eq!(
        Letter::KIND,
        "message",
        "letters are stored under the kind stores already hold them by"
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
            bag: testkit::saved_bag(&[lure(6), plank(50), rod(1)]),
            ..CharacterSnapshot::of_def(&before, "Wren", player::def(&before))
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
        .map(|row| {
            (
                row.title.as_str(),
                written_by(&row.from),
                row.attached.clone(),
            )
        })
        .collect();
    assert_eq!(
        (rows, inbox.held),
        (
            vec![("What your inventory had no room for", None, vec![rod(1)])],
            0
        ),
        "a letter from the system, which counts against no limit"
    );
    let again = SavedCharacter::of_live(&server.village.world, character).expect("her character");
    assert_eq!(
        again.character.bag.set_aside(),
        [],
        "and what was sent is in her character no longer"
    );
}

#[test]
fn a_delivery_fills_the_bag_and_sends_what_has_no_room_by_message() {
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
        let paid = deliver::to_character(
            &mut server.village.world,
            wren,
            stacks,
            DeliveryReason::Given(None),
        );
        server.step();
        paid
    };
    let letters = |server: &mut Server| -> Vec<(String, bool, Vec<ItemStack>)> {
        inbox_of(server, 1)
            .rows
            .into_iter()
            .map(|row| (row.title, row.from == InboxSender::System, row.attached))
            .collect()
    };

    let paid = pay(&mut server, &[lure(6)]);
    assert_eq!(
        (paid.kept, paid.sent, bag(&server, 1), letters(&mut server)),
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
        deliver::overflow_line(&content, &paid.sent).plain_words(&content),
        "No room in your bag for 1 rod: sent to your mail.",
        "the delivery's notification tells what went to mail"
    );
    assert_eq!(
        letters(&mut server),
        [("Items from village".to_owned(), true, vec![rod(1)])],
        "the rest comes as a letter from the game, named for what paid it"
    );

    carry(&mut server, 1, &[]);
    server.village.disconnect(1);
    let paid = deliver::to_character(
        &mut server.village.world,
        wren,
        &[lure(2)],
        DeliveryReason::Given(None),
    );
    server.step();
    assert_eq!(
        (paid.kept, paid.sent),
        (Vec::new(), vec![lure(2)]),
        "a character whose player is away is delivered everything by letter"
    );
    server.sign_in(1, "Wren", "village");
    server.step();
    assert_eq!(
        (bag(&server, 1), letters(&mut server).len()),
        (Vec::new(), 2),
        "and the letter waits for them, though they left before it was written"
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
    let size = bag.slots.len();
    bag.slots = stacks
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::repeat(None))
        .take(size)
        .collect();
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
            Some(
                sim.world
                    .get::<Inventory>(character)?
                    .stacks()
                    .map(|(_, stack)| stack)
                    .collect(),
            )
        })
        .expect("a character with a bag")
}

/// The player of `client` writes to the player called `to`.
fn write(server: &mut Server, client: u32, to: &str, title: &str, attached: &[ItemStack]) {
    let letter = SendLetter {
        to: server.number(to),
        title: title.to_owned(),
        text: "Six feathers and the old rod.".to_owned(),
        attached: attached.to_vec(),
    };
    server.send(client, letter);
    server.step();
}

/// Where the last notice the player of `client` was told shows, and under which icon.
fn arrival(server: &mut Server, client: u32) -> Option<(FeedLayout, ImageRef)> {
    let told = server.village_or_forest(client).told(client).pop()?;
    match told.kind {
        NotificationKind::Feed(row) => Some((row.layout, row.icon?)),
        _ => None,
    }
}

fn mail_icon(content: &Content) -> ImageRef {
    InterfaceIcon::Mail.get(content).image.clone()
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

/// A game of two areas with friends and letters, whose inboxes hold two letters from players
/// and whose players send two a minute, with things to carry: one of them bound.
fn game() -> Game {
    let stacked = |max: u32| json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": max }] });
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .interface_icon("mail")
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
            settings["mail"] = json!({
                "inbox": 2, "attachments": 6, "title_length": 60, "notice_title_length": 25, "text_length": 1000, "a_minute": 2
            });
        })
}

/// The name of the character who wrote a letter, or none for the system.
fn written_by(from: &InboxSender) -> Option<&str> {
    match from {
        InboxSender::System => None,
        InboxSender::Character(from) => Some(from.name.as_str()),
    }
}
