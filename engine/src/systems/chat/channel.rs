//! The channels chat is said on: which there are, who is in each and who hears a line.

use bevy_ecs::prelude::*;
use bevy_terminal::TerminalChannel;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::{ContentFile, Fixture, ImageRef, NamedContent, Nearby, RowCheck};
use crate::core::math::Rgb;
use crate::systems::player::PlayedBy;
use crate::systems::society::SocietyView;

/// The channels the engine knows, in the order a player's are listed. Each is a file every game
/// provides.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChatChannel {
    /// Everyone is in it, and a line reaches everyone in the speaker's world.
    #[default]
    Area,
    /// The members of a party, wherever each of them plays.
    Party,
}

impl Fixture for ChatChannel {
    type Row = ChatChannelDef;
    const FOLDER: Option<&'static str> = Some("interface");
    const WHOLE_TABLE: bool = true;
}

impl ChatChannel {
    pub fn terminal(self) -> TerminalChannel {
        let place = ChatChannel::VARIANTS
            .iter()
            .position(|channel| *channel == self)
            .and_then(|place| u8::try_from(place).ok());
        TerminalChannel(place.expect("a channel is one of the few the engine lists"))
    }

    pub fn of(channel: TerminalChannel) -> Option<ChatChannel> {
        ChatChannel::VARIANTS.get(usize::from(channel.0)).copied()
    }
}

/// How a channel is told from the others: by its name on its tab, and by a sign and a colour on
/// every line said on it, so never by its colour alone.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatChannelDef {
    pub name: String,
    pub sign: ImageRef,
    pub color: Rgb,
}

impl ContentFile for ChatChannelDef {
    const TABLE: &'static str = "chat_channel";

    fn fresh(near: &Nearby) -> Result<ChatChannelDef, String> {
        Ok(ChatChannelDef {
            name: near.name(),
            sign: near.image()?,
            color: Rgb::WHITE,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, MOST_NAME);
    }
}

impl NamedContent for ChatChannelDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

/// A tab's name fits its tab however narrow chat is drawn.
const MOST_NAME: usize = 16;

/// What a system brings for its channel.
#[derive(Clone, Copy)]
pub struct ChatChannelSource {
    pub channel: ChatChannel,
    /// Whether the player of a character is in the channel. Asked as the character comes to a
    /// world and whenever what its player sees of society changes.
    pub open_to: fn(&World, Entity) -> bool,
    /// Carries what the player of a character said on the channel to whoever hears it.
    pub send: fn(&mut World, Entity, String),
}

#[derive(Resource, Default)]
pub struct Sources(Vec<ChatChannelSource>);

pub fn source(world: &mut World, source: ChatChannelSource) {
    world.resource_mut::<Sources>().0.push(source);
}

/// What brings `channel`, in a world whose game has it.
pub(super) fn source_of(world: &World, channel: ChatChannel) -> Option<ChatChannelSource> {
    let sources = &world.resource::<Sources>().0;
    sources
        .iter()
        .find(|source| source.channel == channel)
        .copied()
}

/// The channels the player of a character is in, in the order the engine lists them: the tabs
/// their chat has.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ChatChannels(pub Vec<ChatChannel>);

/// Tells every player the channels they are in, as that changes: as they come to the world,
/// and as what they see of society does.
pub fn list(world: &mut World, due: &mut Unlisted) {
    let players: Vec<Entity> = due.iter(world).collect();
    for player in players {
        let listed = world.get::<ChatChannels>(player);
        if !listed.is_some_and(|listed| listed.0.iter().copied().eq(open_to(world, player))) {
            let open = ChatChannels(open_to(world, player).collect());
            world.entity_mut(player).insert(open);
        }
    }
}

type Unlisted = QueryState<
    Entity,
    (
        With<PlayedBy>,
        Or<(Without<ChatChannels>, Changed<SocietyView>)>,
    ),
>;

fn open_to(world: &World, player: Entity) -> impl Iterator<Item = ChatChannel> {
    let sources = &world.resource::<Sources>().0;
    ChatChannel::VARIANTS
        .iter()
        .copied()
        .filter(move |channel| {
            sources
                .iter()
                .any(|source| source.channel == *channel && (source.open_to)(world, player))
        })
}
