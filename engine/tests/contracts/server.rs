use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use engine::core::content::{Content, ContentScope};
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::Seconds;
use engine::systems::account::{AccountId, AccountIdentity};
use engine::systems::friend::{self, AskFriendship, FindPlayers, PlayersFound, RemoveFriend};
use engine::systems::history::HistoryNews;
use engine::systems::player::{self, ClientId, SpawnPolicy, SpawnRules};
use engine::systems::social_request::{
    self, AnswerSocialRequest, BlockPlayer, ChooseAcceptsRequests, RequestAnswer, SocialRequestAsk,
    SocialRequestKind, SocialRequestSource,
};
use engine::systems::society::{
    self, AccountNumber, ServerSociety, Society, SocietyAsk, SocietyView,
};
use engine::systems::worlds::AboveWorlds;
use testkit::{Game, Sim};

use crate::saves::{Saves, confer};

/// A server of two worlds, the village and the forest, and what it keeps above them.
pub struct Server {
    pub above: AboveWorlds,
    pub village: Sim,
    pub forest: Sim,
}

impl Server {
    /// A server of `game`, whose players ask each other for what `sources` bring.
    pub fn of(game: Game, sources: Vec<SocialRequestSource>) -> Server {
        let content = game.load();
        let above = AboveWorlds {
            society: ServerSociety::of(Society::new(sources)),
            ..AboveWorlds::unsaved()
        };
        Server::with(&content, above)
    }

    /// A server of the game `content` holds that keeps what `saves` holds, and saves to it.
    pub fn saving(content: &Content, saves: &Saves, sources: Vec<SocialRequestSource>) -> Server {
        let society = saves
            .runtime
            .block_on(Society::read(
                &saves.store,
                &ContentScope::of(content),
                sources,
            ))
            .expect("the store is read");
        let above = AboveWorlds {
            society: ServerSociety::of(society),
            ..saves.above(content)
        };
        Server::with(content, above)
    }

    fn with(content: &Content, above: AboveWorlds) -> Server {
        let mut server = Server {
            above,
            village: serving(content, "village"),
            forest: serving(content, "forest"),
        };
        server.step();
        server
    }

    /// The worlds confer and then each plays a tick, as a server's do.
    pub fn step(&mut self) {
        confer(&mut [&mut self.village, &mut self.forest], &mut self.above);
        self.village.tick();
        self.forest.tick();
    }

    pub fn later(&mut self, seconds: f32) {
        for sim in [&mut self.village, &mut self.forest] {
            let mut clock = sim.world.resource_mut::<WallClock>();
            clock.now = clock.now.after(Seconds(seconds));
        }
        self.step();
        self.step();
    }

    pub fn now(&self) -> UnixMillis {
        self.village.world.resource::<WallClock>().now
    }

    /// The account called `name` signs in and joins the world of `area` as `client`.
    pub fn sign_in(&mut self, client: u32, name: &str, area: &str) -> Entity {
        let identity = AccountIdentity {
            id: AccountId(name.to_owned()),
            name: name.to_owned(),
            roles: Vec::new(),
        };
        let now = self.now();
        self.above
            .society
            .signed_in(&identity, now, &mut self.above.checkpoints.unwritten);
        let sim = match area {
            "village" => &mut self.village,
            _ => &mut self.forest,
        };
        sim.sign_in(client, name, None)
    }

    pub fn village_or_forest(&mut self, client: u32) -> &mut Sim {
        let here = self
            .village
            .world
            .resource::<player::Players>()
            .0
            .contains_key(&ClientId(client));
        if here {
            &mut self.village
        } else {
            &mut self.forest
        }
    }

    pub fn number(&self, name: &str) -> AccountNumber {
        self.above
            .society
            .read()
            .accounts()
            .number(&AccountId(name.to_owned()))
            .expect("an account that signed in")
    }

    /// The player of `client` asks the player called `of` for something of `kind`, as the
    /// system that owns the kind asks for them.
    pub fn ask(&mut self, client: u32, kind: SocialRequestKind, of: &str) {
        let ask = SocialRequestAsk::Ask {
            kind,
            of: self.number(of),
            about: (kind == SocialRequestKind::Seat).then(|| "Rowboat".to_owned()),
        };
        let world = &mut self.village_or_forest(client).world;
        society::ask(world, ClientId(client), SocietyAsk::Request(ask));
    }

    pub fn send<M: Message>(&mut self, client: u32, message: M) {
        let sim = self.village_or_forest(client);
        let conn = sim.conn(client);
        sim.send(conn, message);
        sim.tick();
    }

    /// The player of `client` answers the card in front of them.
    pub fn answer(&mut self, client: u32, answer: RequestAnswer) {
        let request = self.view(client).requests[0].id;
        self.send(client, AnswerSocialRequest { request, answer });
    }

    pub fn block(&mut self, client: u32, name: &str, blocked: bool) {
        let account = self.number(name);
        self.send(client, BlockPlayer { account, blocked });
    }

    pub fn view(&mut self, client: u32) -> SocietyView {
        let sim = self.village_or_forest(client);
        let character = sim.player(client);
        sim.world
            .get::<SocietyView>(character)
            .cloned()
            .expect("what the player sees of society")
    }

    /// What each of the player's cards says, and whether it is one they asked.
    pub fn cards(&mut self, client: u32) -> Vec<(String, bool)> {
        let requests = self.view(client).requests;
        requests
            .into_iter()
            .map(|card| (card.says, card.mine))
            .collect()
    }

    /// What the feed told the player, all of it so far.
    pub fn told(&mut self, client: u32) -> Vec<String> {
        let sim = self.village_or_forest(client);
        let errors = sim.errors(client);
        sim.told(client)
            .into_iter()
            .map(|told| told.text.plain_words())
            .filter(|said| !errors.contains(said))
            .collect()
    }
}

fn serving(content: &Content, area: &str) -> Sim {
    let mut sim = Sim::new(content.clone());
    sim.serve(area)
        .message::<ToClient<HistoryNews>>()
        .message::<ToClient<PlayersFound>>()
        .accept::<AnswerSocialRequest>()
        .accept::<BlockPlayer>()
        .accept::<ChooseAcceptsRequests>()
        .accept::<AskFriendship>()
        .accept::<RemoveFriend>()
        .accept::<FindPlayers>()
        .add((
            society::view::greet,
            social_request::requests,
            friend::requests,
            player::linger,
        ));
    sim.world.insert_resource(WallClock {
        now: UnixMillis(1_000_000_000),
        reset: UtcHour::MIDNIGHT,
    });
    sim.world.insert_resource(SpawnRules {
        policy: SpawnPolicy::Map,
        immortal: false,
    });
    sim
}
