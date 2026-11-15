use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use bevy_terminal::{LineKind, TerminalInput};
use engine::core::units::{Meters, Seconds};
use engine::systems::area::AreaDef;
use engine::systems::area_map::{self, MapMarkKind};
use engine::systems::chat::{ChatChannel, ChatChannels, ChatSpeaker};
use engine::systems::item::{self, DropTaker, Reservation};
use engine::systems::movement::position;
use engine::systems::npc::{self, NpcDef};
use engine::systems::party::share::shares;
use engine::systems::party::{ExperienceSplit, PartyRequest, SharedExperience};
use engine::systems::player::{ClientId, Xp};
use engine::systems::rewards::{self, KillCredited};
use engine::systems::social_request::RequestAnswer;
use engine::systems::{GameSettings, combat, request_sources, stat};
use serde_json::json;
use testkit::{Game, Map};

use crate::saves::{Saves, checkpoint};
use crate::server::{ChatShown, Server, SpeakersShown};

#[test]
fn a_party_is_made_by_an_invitation_and_run_by_its_leader() {
    let mut server = Server::of(game(), request_sources());
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "forest");
    server.sign_in(3, "Marrow", "village");
    server.sign_in(4, "Osric", "village");

    invite(&mut server, 1, "Hale");
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (
            vec![("You invited Hale to your party".to_owned(), true)],
            vec![("Wren invites you to their party".to_owned(), false)]
        )
    );
    assert_eq!(
        party(&mut server, 1),
        Vec::<String>::new(),
        "an invitation alone makes no party"
    );
    server.answer(2, RequestAnswer::Accept);
    server.step();
    let both = ["Wren ★ village", "Hale forest"];
    assert_eq!(
        (party(&mut server, 1), party(&mut server, 2)),
        (
            both.map(String::from).to_vec(),
            both.map(String::from).to_vec()
        ),
        "accepting makes one of the two, led by whoever invited, wherever each is"
    );
    assert_eq!(server.told(1), ["Hale joined the party"]);

    invite(&mut server, 3, "Hale");
    assert_eq!(
        server.village.errors(3),
        ["Hale is in a party"],
        "a player is in one at most"
    );
    invite(&mut server, 2, "Marrow");
    invite(&mut server, 1, "Marrow");
    assert_eq!(
        server.cards(3),
        [("Hale invites you to their party".to_owned(), false)],
        "any member invites, and a party invites a player once"
    );
    server.answer(3, RequestAnswer::Deny);
    server.step();
    assert_eq!(server.told(2), ["Marrow denied your invitation"]);
    invite(&mut server, 1, "Marrow");
    server.later(61.0);
    assert_eq!(
        (server.cards(3).len(), server.told(1).last().cloned()),
        (0, Some("Marrow did not answer your invitation".to_owned())),
        "an invitation is short-lived, and its inviter is told how it went"
    );
    invite(&mut server, 1, "Marrow");
    server.answer(3, RequestAnswer::Accept);
    server.step();
    assert_eq!(party(&mut server, 3).len(), 3);
    server.village.forget_notified();
    invite(&mut server, 1, "Osric");
    assert_eq!(server.village.errors(1), ["The party is full"]);

    server.send(2, PartyRequest::Lead(server.number("Marrow")));
    server.step();
    assert_eq!(
        party(&mut server, 1)[0],
        "Wren ★ village",
        "only the leader hands the lead over"
    );
    server.send(1, PartyRequest::Lead(server.number("Hale")));
    server.step();
    assert_eq!(
        party(&mut server, 3),
        ["Wren village", "Hale ★ forest", "Marrow village"]
    );
    assert_eq!(
        server.told(3).last().map(String::as_str),
        Some("Hale leads the party now")
    );

    server.send(1, PartyRequest::Remove(server.number("Marrow")));
    server.step();
    assert_eq!(party(&mut server, 3).len(), 3, "only the leader removes");
    server.send(2, PartyRequest::Remove(server.number("Marrow")));
    server.step();
    assert_eq!(
        (party(&mut server, 3), party(&mut server, 1)),
        (
            Vec::<String>::new(),
            vec!["Wren village".to_owned(), "Hale ★ forest".to_owned()]
        )
    );
    assert_eq!(
        (
            server.told(3).last().cloned(),
            server.told(1).last().cloned()
        ),
        (
            Some("You were removed from the party".to_owned()),
            Some("Marrow was removed from the party".to_owned())
        )
    );

    server.send(2, PartyRequest::Leave);
    server.step();
    assert_eq!(
        (party(&mut server, 2), party(&mut server, 1)),
        (Vec::<String>::new(), vec!["Wren ★ village".to_owned()]),
        "a party its leader leaves is led by the member who has been in it longest"
    );
    invite(&mut server, 1, "Hale");
    server.answer(2, RequestAnswer::Accept);
    server.step();
    server.send(2, PartyRequest::Disband);
    server.step();
    assert_eq!(party(&mut server, 1).len(), 2, "only the leader ends it");
    server.send(1, PartyRequest::Disband);
    server.step();
    assert_eq!(
        (party(&mut server, 1), party(&mut server, 2)),
        (Vec::<String>::new(), Vec::<String>::new())
    );
    assert_eq!(
        server.told(2).last().map(String::as_str),
        Some("Wren disbanded the party")
    );
}

#[test]
fn a_member_whose_player_is_gone_leaves_after_a_while_and_a_party_outlives_its_server() {
    let saves = Saves::in_a_file();
    let content = game().load();
    let mut server = Server::saving(&content, &saves, request_sources());
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "village");
    for name in ["Hale", "Marrow"] {
        invite(&mut server, 1, name);
    }
    server.answer(2, RequestAnswer::Accept);
    server.answer(3, RequestAnswer::Accept);
    server.step();

    server.village.disconnect(1);
    let left_at = server.now();
    server.step();
    server.step();
    assert_eq!(
        party(&mut server, 2),
        ["Wren away", "Hale ★ village", "Marrow village"],
        "a leader whose player is gone hands the lead to the longest-standing member there"
    );
    let wren = server.view(2).party.expect("a party").members[0].clone();
    assert_eq!(wren.leaves_at, Some(left_at.after(Seconds(300.0))));
    server.sign_in(1, "Wren", "village");
    server.step();
    assert_eq!(
        party(&mut server, 1),
        ["Wren village", "Hale ★ village", "Marrow village"],
        "who comes back in time is a member still, and leads no more"
    );
    let report = checkpoint(
        &mut [&mut server.village, &mut server.forest],
        &mut server.above,
    );
    assert!(report.outcome.is_ok());

    let mut next = Server::saving(&content, &saves, request_sources());
    next.sign_in(3, "Marrow", "forest");
    next.step();
    next.step();
    assert_eq!(
        party(&mut next, 3),
        ["Wren away", "Hale away", "Marrow ★ forest"],
        "a party outlives its server, every member away until they are back"
    );
    next.later(301.0);
    assert_eq!(
        party(&mut next, 3),
        ["Marrow ★ forest"],
        "and whoever stays away too long is out of it"
    );
}

#[test]
fn a_party_shares_what_a_kill_is_worth_and_what_it_drops() {
    let mut server = Server::of(game(), request_sources());
    server
        .village
        .add(rewards::grant)
        .plan(stat::SettleVitals::default())
        .add(combat::defeats);
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "village");
    let marrow = server.sign_in(3, "Marrow", "forest");
    let osric = server.sign_in(4, "Osric", "village");
    for name in ["Hale", "Marrow"] {
        invite(&mut server, 1, name);
    }
    server.answer(2, RequestAnswer::Accept);
    server.answer(3, RequestAnswer::Accept);
    server.step();
    let xp = |server: &Server, character: Entity| {
        let sims = [&server.village, &server.forest];
        sims.iter()
            .find_map(|sim| sim.world.get::<Xp>(character))
            .map_or(0, |xp| xp.amount)
    };

    let mut marked: Vec<(String, MapMarkKind)> = area_map::marks(&mut server.village.world, wren)
        .into_iter()
        .map(|mark| (mark.name, mark.kind))
        .collect();
    marked.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        marked,
        [
            ("Hale".to_owned(), MapMarkKind::PartyMember),
            ("Osric".to_owned(), MapMarkKind::Player)
        ],
        "a member's map tells the party's players from the rest"
    );

    let mut credited = MessageCursor::<KillCredited>::default();
    let brute = brute(&mut server, wren);
    item::reserve(&mut server.village.world, brute, wren, Seconds(0.0));
    combat::inflict(&mut server.village.world, brute, Some(wren), 1_000_000.0);
    for _ in 0..3 {
        server.step();
    }
    assert_eq!(
        [wren, hale, marrow, osric].map(|character| xp(&server, character)),
        [60, 60, 0, 0],
        "the members standing by share it, each a part of its worth and the party's bonus"
    );
    let counted: Vec<Entity> = server
        .village
        .read(&mut credited)
        .into_iter()
        .map(|kill| kill.credited)
        .collect();
    assert_eq!(
        counted,
        [wren, hale],
        "and the kill counts for the quests of each"
    );
    assert_eq!(
        (
            server.told(1).last().cloned(),
            server.told(2).last().cloned()
        ),
        (
            Some("Shared with Hale".to_owned()),
            Some("Shared with Wren".to_owned())
        )
    );

    let claim = Reservation {
        by: Some(ClientId(1)),
        at: Seconds(0.0),
    };
    let takes = |server: &Server, character: Entity| {
        item::takeable(
            Some(&claim),
            &DropTaker::of(&server.village.world, character),
        )
    };
    assert_eq!(
        [wren, hale, osric].map(|character| takes(&server, character)),
        [true, true, false],
        "what drops for one member is the whole party's to take, and no one else's"
    );

    server.village.disconnect(2);
    server.step();
    let brute = self::brute(&mut server, wren);
    item::reserve(&mut server.village.world, brute, wren, Seconds(0.0));
    combat::inflict(&mut server.village.world, brute, Some(wren), 1_000_000.0);
    for _ in 0..3 {
        server.step();
    }
    assert_eq!(
        [wren, hale].map(|character| xp(&server, character)),
        [160, 60],
        "a member whose player is gone shares in nothing"
    );

    let split = |split, levels: &[u32]| {
        let rules = SharedExperience {
            split,
            range: Meters(40.0),
            bonus: 20.0,
        };
        shares(&rules, 100, levels)
    };
    assert_eq!(split(ExperienceSplit::Even, &[1, 3]), [60, 60]);
    assert_eq!(split(ExperienceSplit::ByLevel, &[1, 3]), [30, 90]);
    assert_eq!(split(ExperienceSplit::Whole, &[1, 3]), [120, 120]);
    assert_eq!(
        split(ExperienceSplit::Even, &[5]),
        [100],
        "alone, a kill is worth what it is"
    );
}

#[test]
fn a_party_talks_on_a_channel_of_its_own_wherever_its_members_play() {
    let mut server = Server::of(game(), request_sources());
    let wren = server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "forest");
    server.sign_in(4, "Osric", "village");
    server.step();
    let channels = |server: &Server, character: Entity| {
        server
            .village
            .world
            .get::<ChatChannels>(character)
            .map(|channels| channels.0.clone())
    };
    assert_eq!(
        channels(&server, wren),
        Some(vec![ChatChannel::Area]),
        "everyone is in the area's channel"
    );

    for name in ["Hale", "Marrow"] {
        invite(&mut server, 1, name);
    }
    server.answer(2, RequestAnswer::Accept);
    server.answer(3, RequestAnswer::Accept);
    server.step();
    server.step();
    assert_eq!(
        channels(&server, wren),
        Some(vec![ChatChannel::Area, ChatChannel::Party]),
        "and the members of a party in the party's too"
    );
    assert_eq!(
        (heard(&mut server, 1), heard(&mut server, 2)),
        (
            vec![
                (ChatChannel::Party, "Hale joined the party".to_owned()),
                (ChatChannel::Party, "Marrow joined the party".to_owned()),
            ],
            vec![
                (ChatChannel::Party, "You joined Wren's party".to_owned()),
                (ChatChannel::Party, "Marrow joined the party".to_owned()),
            ]
        ),
        "the party's channel opens saying why"
    );

    say(&mut server, 1, ChatChannel::Party, "boat's at the jetty");
    let line = (ChatChannel::Party, "Wren: boat's at the jetty".to_owned());
    for member in 1..=3 {
        assert_eq!(heard(&mut server, member).last(), Some(&line), "{member}");
    }
    assert!(
        heard(&mut server, 4).is_empty(),
        "who is not in the party hears nothing of it"
    );
    say(&mut server, 4, ChatChannel::Party, "let me in");
    assert_eq!(
        heard(&mut server, 1).last(),
        Some(&line),
        "nor is heard on a channel they are not in"
    );

    server.block(3, "Wren", true);
    server.block(4, "Wren", true);
    server.step();
    say(&mut server, 1, ChatChannel::Party, "anyone there?");
    say(&mut server, 1, ChatChannel::Area, "selling lures");
    assert_eq!(
        heard(&mut server, 2)[3..],
        [
            (ChatChannel::Party, "Wren: anyone there?".to_owned()),
            (ChatChannel::Area, "Wren: selling lures".to_owned()),
        ]
    );
    assert_eq!(
        (heard(&mut server, 3).last(), heard(&mut server, 4).last()),
        (Some(&line), None),
        "a line reaches nobody who keeps its speaker away, on any channel"
    );
    let colors = &server.village.world.resource::<GameSettings>().chat;
    let colors = colors.name_colors.clone();
    let told: Vec<ChatSpeaker> = [&server.village, &server.forest]
        .iter()
        .flat_map(|sim| &sim.world.resource::<SpeakersShown>().0)
        .map(|(_, speaker)| speaker.clone())
        .collect();
    assert!(
        told.len() > 3
            && colors.contains(&told[0].name_color)
            && told.iter().all(|speaker| {
                speaker.typist == ClientId(1) && speaker.name_color == told[0].name_color
            }),
        "whoever hears a player is told the one colour of the game's their name is written \
         in, on every channel and in every world: {told:?}"
    );

    server.send(2, PartyRequest::Leave);
    server.step();
    server.step();
    let hale = server.village.player(2);
    assert_eq!(channels(&server, hale), Some(vec![ChatChannel::Area]));
    assert_eq!(
        heard(&mut server, 2).last(),
        Some(&(ChatChannel::Area, "You left the party".to_owned())),
        "a channel that goes says why where its player still reads"
    );
}

/// The player of `client` says `text` on `channel`, and the server carries it.
fn say(server: &mut Server, client: u32, channel: ChatChannel, text: &str) {
    let typed = TerminalInput {
        text: text.to_owned(),
        channel: channel.terminal(),
        embeds: Vec::new(),
    };
    server.send(client, typed);
    server.step();
}

/// Every line chat showed the player of `client`, the oldest first: what was said after who
/// said it.
fn heard(server: &mut Server, client: u32) -> Vec<(ChatChannel, String)> {
    let sim = server.village_or_forest(client);
    let conn = sim.conn(client);
    let shown = &sim.world.resource::<ChatShown>().0;
    shown
        .iter()
        .filter(|(to, _)| *to == conn)
        .filter_map(|(_, line)| {
            let said = match &line.kind {
                LineKind::Said(speaker) => format!("{}: {}", speaker.name, line.text),
                LineKind::Notice | LineKind::Failed => line.text.clone(),
            };
            Some((ChatChannel::of(line.channel)?, said))
        })
        .collect()
}

/// The player of `client` invites the player called `name` to their party.
fn invite(server: &mut Server, client: u32, name: &str) {
    server.send(client, PartyRequest::Invite(server.number(name)));
    server.step();
}

/// The party the player of `client` sees: each member by name, starred when they lead, and the
/// area they play in or that they are away.
fn party(server: &mut Server, client: u32) -> Vec<String> {
    let Some(party) = server.view(client).party else {
        return Vec::new();
    };
    let forest = server.forest.id::<AreaDef>("forest");
    party
        .members
        .iter()
        .map(|member| {
            let leads = if party.leader == member.player.account {
                " ★"
            } else {
                ""
            };
            let place = match member.area {
                Some(area) if area == forest => "forest",
                Some(_) => "village",
                None => "away",
            };
            format!("{}{leads} {place}", member.player.name)
        })
        .collect()
}

fn brute(server: &mut Server, near: Entity) -> Entity {
    let sim = &mut server.village;
    let at = position(&sim.world, near).expect("a place to stand");
    let (area, brute) = (sim.id::<AreaDef>("village"), sim.id::<NpcDef>("brute"));
    npc::spawn(&mut sim.world, brute, at, area, None)
}

/// A game of two areas with parties of three, whose kills are shared evenly with a fifth more
/// for each further member, and a brute worth a hundred.
fn game() -> Game {
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .npc(
            "brute",
            25.0,
            json!([{ "type": "loot", "xp": 100, "items": [] }]),
        )
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["party"] = json!({
                "size": 3,
                "invite_lasts": 60.0,
                "absent_for": 300.0,
                "experience": { "split": "even", "range": 40.0, "bonus": 20.0 },
            });
        })
}
