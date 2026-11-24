pub mod channel;
pub mod embed;
pub mod log;

use crate::core::script::EventDef;
use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::Vocabulary;
use crate::systems::area::OBJECT_RUNNER;
use crate::systems::area::spawnable::SPAWNABLE_RUNNER;
use crate::systems::npc::NPC_RUNNER;
use std::collections::VecDeque;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replication::{AppMessageExt, AppReplicationExt, ToClient};
use bevy_terminal::{
    LineKind, LineSpeaker, TerminalCompletionRequest, TerminalCompletions, TerminalEmbed,
    TerminalInput, TerminalLine, TerminalSaid,
};
pub use channel::{ChatChannel, ChatChannelDef, ChatChannelSource, ChatChannels, Sources, source};
pub use embed::{ChatEmbed, ChatEmbedKind, ChatLocation, ChatSaid, EMBED_ASK};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, SchemaMark};
use crate::core::math::Rgb;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault, ValueType};
use crate::core::script::{EffectCall, EventKind, Expr, Instances, Owner, RunnerKind};
use crate::core::text::LineText;
use crate::core::time;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::actor::{Appearance, Name as ActorName};
use crate::systems::area::AreaTag;
use crate::systems::dialogue::span::ScriptSpan;
use crate::systems::input::map::InputMap;
use crate::systems::messages::{self, messages};
use crate::systems::movement::position;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::Npc;
use crate::systems::player::{self, CharacterAccount, ClientId, PlayedBy, Players};
use crate::systems::society::{Settling, SocietyView};
use crate::systems::stat;
use crate::systems::visibility;

messages! {
    /// What the game says of chat.
    pub struct ChatMessages {
        /// What a player is told saying more lines than the game lets them in a while.
        too_fast,
        /// What the items a chat line may hold are called together.
        items,
        /// What the quests a chat line may hold are called together.
        quests,
        /// What the skills a chat line may hold are called together.
        skills,
        /// What the locations a chat line may hold are called together.
        locations,
        /// What a chat line says an item it may hold is.
        item_hint,
        /// What a chat line says a quest it may hold is.
        quest_hint,
        /// What a chat line says a skill it may hold is.
        skill_hint,
        /// What a chat line says a location it may hold is.
        location_hint,
        /// The name a line is said under by a player who has none, `{number}` the number of their connection.
        player_number(number: Number),
        /// The name a line is said under by a speaker who has none.
        player,
        /// What a location set in a chat line is called: the area `{area}` it is in and where, `{x}` and `{y}`.
        location_spot(area: Text, x: Number, y: Number),
    }
}

pub fn register(app: &mut App) {
    bevy_terminal::register(app, to_client);
    app.add_client_message::<TerminalInput>()
        .add_client_message::<TerminalCompletionRequest>()
        .add_server_message::<TerminalLine>()
        .add_server_message::<ChatSpeaker>()
        .add_server_message::<TerminalCompletions>()
        .replicate_private::<ChatChannels>();
}

/// How much a player may say: no more than `lines` lines within any stretch of `within`, on
/// every channel alike. And the colours players' names are written in, each player's always the
/// same one of them, so a reader tells one speaker's lines from the next one's.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSettings {
    pub lines: u32,
    pub within: Seconds,
    pub name_colors: Vec<Rgb>,
    /// How many lines the chat log keeps of each channel. The oldest go first.
    pub kept_lines: u32,
}

impl ChatSettings {
    /// The colour the name of `account` is written in: its id as a number, counted round the
    /// colours.
    pub fn name_color(&self, account: &AccountId) -> Option<Rgb> {
        let number = account.0.bytes().fold(0usize, |number, byte| {
            number.wrapping_mul(31).wrapping_add(usize::from(byte))
        });
        let place = number.checked_rem(self.name_colors.len())?;
        self.name_colors.get(place).copied()
    }
}

/// Who typed a line, for the client of someone who hears it: the colour their name is written
/// in and what their character looks like, by which chat tells them from other speakers.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChatSpeaker {
    pub typist: ClientId,
    pub name_color: Rgb,
    pub appearance: Appearance,
}

impl ChatSpeaker {
    /// The player of `character` as a speaker, when an account plays it.
    pub fn of(world: &World, character: Entity) -> Option<ChatSpeaker> {
        let account = world.get::<CharacterAccount>(character)?;
        let chat = &world.resource::<GameSettings>().chat;
        Some(ChatSpeaker {
            typist: world.get::<PlayedBy>(character)?.client,
            name_color: chat.name_color(&account.0)?,
            appearance: world.get::<Appearance>(character)?.clone(),
        })
    }
}

/// The channel everyone is in: a line said on it reaches the speaker's world, where townsfolk
/// and scripted things overhear it.
pub fn area_channel() -> ChatChannelSource {
    ChatChannelSource {
        channel: ChatChannel::Area,
        open_to: |_, _| true,
        send: say_to_area,
    }
}

/// Sends the terminal's lines to the clients they are for.
pub fn route_lines(world: &mut World) {
    bevy_terminal::route(world, to_client);
}

fn to_client(world: &mut World, conn: Entity, line: TerminalLine) {
    world.write_message(ToClient {
        client: conn,
        message: line,
    });
}

pub fn hear(world: &mut World) {
    for typed in super::drain_requests::<TerminalInput>(world) {
        bevy_terminal::hear(world, typed.client, &typed.message);
    }
}

/// Says what players typed, each line on the channel it was typed on, to whoever hears that
/// channel. A line typed on a channel its player is not in is dropped, as is one that holds
/// what the game does not have, and one typed faster than the game lets a player speak is
/// refused.
pub fn speak(world: &mut World) {
    let saids: Vec<TerminalSaid> = world
        .resource_mut::<Messages<TerminalSaid>>()
        .drain()
        .collect();
    for said in saids {
        let Some(speaker) = player::conn_player(world, said.conn) else {
            continue;
        };
        let source = ChatChannel::of(said.channel)
            .and_then(|channel| channel::source_of(world, channel))
            .filter(|source| (source.open_to)(world, speaker));
        let Some(source) = source else {
            continue;
        };
        let content = world.resource::<Content>();
        let held: Option<Vec<TerminalEmbed>> = said
            .embeds
            .iter()
            .map(|sent| {
                let embed = ChatEmbed::read(content, sent)?.held(content)?;
                Some(embed.sent(content))
            })
            .collect();
        let Some(embeds) = held else {
            continue;
        };
        if !paced(world, said.conn) {
            let refusal = messages::of(world.resource::<Content>())
                .chat
                .too_fast()
                .to_owned();
            bevy_terminal::reply_failure(world, said.conn, refusal);
            continue;
        }
        let said = ChatSaid {
            text: said.text,
            embeds,
        };
        (source.send)(world, speaker, said);
    }
}

/// What the player of `speaker` said on `channel`, as a line for the player of `hearer`:
/// nothing, for a hearer who keeps the speaker away.
pub fn line_across(
    settling: &Settling,
    channel: ChatChannel,
    speaker: &AccountId,
    hearer: &AccountId,
    said: &ChatSaid,
) -> Option<TerminalLine> {
    if settling.society.blocks().blocks(hearer, speaker) {
        return None;
    }
    let typist = settling.roster.account(speaker).map(|player| player.client);
    Some(TerminalLine {
        text: said.text.clone(),
        kind: LineKind::Said(LineSpeaker {
            name: settling.name(speaker),
            typist: typist.map(|client| u64::from(client.0)),
        }),
        channel: channel.terminal(),
        embeds: said.embeds.clone(),
    })
}

/// What the game itself says on `channel`.
pub fn notice(channel: ChatChannel, text: &str) -> TerminalLine {
    TerminalLine {
        text: text.to_owned(),
        kind: LineKind::Notice,
        channel: channel.terminal(),
        embeds: Vec::new(),
    }
}

/// A chat line and a speech bubble from the owner.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[schemars(transform = SchemaMark::System.with("chat"))]
pub struct Say {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<ScriptSpan>,
    /// Without one, as far as the speaker is seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(transform = SchemaMark::Value.with("number"))]
    pub within: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for Say {
    fn owners(&self) -> &'static [RunnerKind] {
        &[OBJECT_RUNNER, SPAWNABLE_RUNNER, NPC_RUNNER]
    }
    fn needs_subject(&self) -> bool {
        false
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let values = ScriptSpan::values(&self.text)
            .into_iter()
            .map(|value| cx.expr(value, ValueType::Text))
            .collect::<Result<Vec<_>, _>>()?;
        let within = cx.opt_expr(self.within.as_ref(), ValueType::Number)?;
        let spans = self.text.clone();
        Ok(Box::new(move |world, _, cx| {
            let Some(speaker) = cx.owner else {
                return EffectStep::Done;
            };
            let spoken: Vec<_> = values.iter().map(|value| value(world, cx)).collect();
            let within = within.as_ref().map_or(visibility::sight(world), |within| {
                Meters(within(world, cx).as_num() as f32)
            });
            say(world, speaker, ScriptSpan::said(&spans, &spoken), within);
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(Says)]
    }
}

pub fn say(world: &mut World, speaker: Entity, text: LineText, within: Meters) {
    let name = world
        .get::<ActorName>(speaker)
        .map(|name| name.name.clone())
        .unwrap_or_default();
    let words = text.words(
        world.resource::<Content>(),
        &world.resource::<InputMap>().key_names(),
    );
    let voice = LineSpeaker {
        name: name.clone(),
        typist: None,
    };
    let everyone = hearers(world, None);
    let area = ChatChannel::Area.terminal();
    bevy_terminal::say(world, &everyone, area, voice, words, &[]);
    let kind = match world.get::<Npc>(speaker) {
        Some(npc) => NotificationKind::Speech { speaker: npc.def },
        None => NotificationKind::Narration {
            label: name.into(),
            sfx: None,
        },
    };
    let (Some(at), Some(area)) = (
        position(world, speaker),
        world.get::<AreaTag>(speaker).map(|tag| tag.area),
    ) else {
        return;
    };
    let listeners: Vec<Entity> = world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .filter(|player| {
            world
                .get::<AreaTag>(*player)
                .is_some_and(|tag| tag.area == area)
        })
        .filter(|player| position(world, *player).is_some_and(|there| there.distance(at) <= within))
        .filter(|player| visibility::present(world, speaker, *player))
        .collect();
    for player in listeners {
        notification::notify(
            world,
            player,
            Some(speaker),
            Notification::new(kind.clone(), text.clone()),
        );
    }
}

/// Raises `heard` on every scripted object and npc in earshot of a player who said what it listens
/// for: a listed word or phrase among the player's words, whatever their case.
fn raise_heard(world: &mut World, speaker: Entity, said: &str) {
    let (Some(at), Some(area)) = (
        position(world, speaker),
        world.get::<AreaTag>(speaker).map(|tag| tag.area),
    ) else {
        return;
    };
    let listeners: Vec<(Owner, Meters)> = world
        .resource::<Instances>()
        .owners_answering(EventKind::of::<Heard>())
        .iter()
        .filter_map(|&owner| {
            let Owner::Entity(_, listener) = owner else {
                return None;
            };
            let in_earshot = listener != speaker
                && !stat::is_dead(world, listener)
                && world.get::<AreaTag>(listener).map(|tag| tag.area) == Some(area)
                && visibility::coexist(world, listener, speaker);
            let distance = position(world, listener)?.distance(at);
            in_earshot.then_some((owner, distance))
        })
        .collect();
    let spoken = Arc::new(spoken_words(said));
    for (owner, distance) in listeners {
        let spoken = spoken.clone();
        world.resource_mut::<Instances>().raise(
            owner,
            EventKind::of::<Heard>(),
            Some(speaker),
            Vec::new(),
            move |event| {
                event.downcast_ref::<Heard>().is_some_and(|heard| {
                    distance <= heard.within
                        && heard.words.iter().any(|phrase| says(&spoken, phrase))
                })
            },
        );
    }
}

fn says(spoken: &[String], phrase: &str) -> bool {
    let phrase = spoken_words(phrase);
    !phrase.is_empty() && spoken.windows(phrase.len()).any(|run| run == phrase)
}

fn spoken_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn say_to_area(world: &mut World, speaker: Entity, said: ChatSaid) {
    raise_heard(world, speaker, &said.text);
    let client = world.get::<PlayedBy>(speaker).map(|by| by.client);
    let words = &messages::of(world.resource::<Content>()).chat;
    let name = world
        .get::<ActorName>(speaker)
        .map(|name| name.name.clone())
        .or_else(|| client.map(|client| words.player_number(client.0)))
        .unwrap_or_else(|| words.player().to_owned());
    let voice = LineSpeaker {
        name,
        typist: client.map(|client| u64::from(client.0)),
    };
    let seen = ChatSpeaker::of(world, speaker);
    let hearers = hearers(world, Some(speaker));
    if let Some(seen) = seen {
        for &conn in &hearers {
            world.write_message(ToClient {
                client: conn,
                message: seen.clone(),
            });
        }
    }
    let area = ChatChannel::Area.terminal();
    bevy_terminal::say(world, &hearers, area, voice, said.text, &said.embeds);
}

/// The connections of this world that hear `speaker`: every one whose player does not keep the
/// speaker's away.
fn hearers(world: &mut World, speaker: Option<Entity>) -> Vec<Entity> {
    let speaker = speaker
        .and_then(|speaker| world.get::<SocietyView>(speaker))
        .map(|view| view.account);
    let conns: Vec<(Entity, ClientId)> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .map(|(conn, client)| (conn, *client))
        .collect();
    let players = world.resource::<Players>();
    conns
        .into_iter()
        .filter(|(_, client)| {
            let view = players
                .0
                .get(client)
                .and_then(|player| world.get::<SocietyView>(*player));
            !view.is_some_and(|view| {
                view.blocked
                    .iter()
                    .any(|blocked| Some(blocked.account) == speaker)
            })
        })
        .map(|(conn, _)| conn)
        .collect()
}

/// When a connection said its latest lines, the oldest first.
#[derive(Component, Default)]
struct LinesSaid(VecDeque<Seconds>);

/// Whether the connection `conn` may say a line now, which then counts as said.
fn paced(world: &mut World, conn: Entity) -> bool {
    let rules = &world.resource::<GameSettings>().chat;
    let (lines, within) = (rules.lines, rules.within);
    let now = time::uptime(world);
    let Ok(mut conn) = world.get_entity_mut(conn) else {
        return false;
    };
    let mut said = conn.take::<LinesSaid>().unwrap_or_default();
    said.0.retain(|at| now.0 - at.0 < within.0);
    let room = said.0.len() < lines as usize;
    if room {
        said.0.push_back(now);
    }
    conn.insert(said);
    room
}

/// A script that may have its owner say something in chat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Says;

impl ReachDef for Says {}

/// A player said one of the `words` or phrases within `within` of the script's owner.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Heard {
    pub words: Vec<String>,
    pub within: Meters,
}

impl EventDef for Heard {
    fn owners(&self) -> &'static [RunnerKind] {
        &[OBJECT_RUNNER, SPAWNABLE_RUNNER, NPC_RUNNER]
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.event::<Heard>("heard").effect::<Say>("say");
}
