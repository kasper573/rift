//! Scenes a bench stages on the server for a test character, through the requests its own client
//! would send, so the client only has to be watched while they play out.

use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::math::{Pos, PosSchema};
use crate::core::time::{DayClock, TimeCourse, WallClock, WeatherClock};
use crate::core::units::{MeterPos, Meters};
use crate::systems::area;
use crate::systems::combat::{self, AttackRequest, Relation};
use crate::systems::interact::{self, InteractRequest, Verb};
use crate::systems::movement::{MoveRequest, MoveSeq, Position};
use crate::systems::notification::{self, NotificationId};
use crate::systems::npc::Npc;
use crate::systems::player::{conn_player, tell_clocks};
use crate::systems::script::Place;
use crate::systems::text::LineText;
use crate::systems::{GameSettings, WorldArea, from_client, stat};

/// Where a game server that stages scenes takes them, beside its test characters.
pub const STAGING_ROUTE: &str = "/test/stage";

/// What a server that hands out test characters takes at [`STAGING_ROUTE`], for the character
/// the asking account plays.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Staging {
    /// Walks the character to `to` in its area.
    Walk {
        #[schemars(with = "PosSchema")]
        to: Pos<Meters>,
    },
    /// Attacks the nearest foe.
    Fight,
    /// Talks to the nearest NPC with something to say.
    Talk,
    /// Puts the character at `place`, in its own area or another.
    Place { place: Place },
    /// Holds the weather of the character's area at the spell of its climate named `spell`, for
    /// everyone in the area, until staged again.
    Weather { spell: String },
    /// Sets the day to `hour` o'clock for everyone on the server, turning on from there or held,
    /// until staged again.
    Day { hour: f32, turning: bool },
    /// Tells the character's player the notification `notification`, as a script tells it, with
    /// nothing in the places its text leaves for a script's values.
    Notify { notification: NotificationId },
}

/// Stages `staging` for the character the connection `conn` plays, as its client would ask for it.
pub fn stage(world: &mut World, conn: Entity, staging: Staging) -> Result<(), String> {
    let player = conn_player(world, conn).ok_or("the connection plays no character")?;
    match staging {
        Staging::Walk { to } => from_client(
            world,
            conn,
            MoveRequest {
                pos: to,
                seq: MoveSeq::default(),
            },
        ),
        Staging::Fight => {
            let foe = nearest(world, player, |world, npc| {
                combat::relation(world, player, npc) == Relation::Enemy
                    && !stat::is_dead(world, npc)
            })
            .ok_or("no foe in the character's area")?;
            from_client(world, conn, AttackRequest { target: foe });
        }
        Staging::Talk => {
            let npc = nearest(world, player, |world, npc| {
                interact::interaction_of(world, npc).is_some_and(|it| it.verb == Verb::Talk)
            })
            .ok_or("nobody to talk to in the character's area")?;
            from_client(world, conn, InteractRequest { target: npc });
        }
        Staging::Place { place } => area::warp::warp(world, player, place)?,
        Staging::Weather { spell } => {
            let area = world.resource::<WorldArea>().0;
            let climate = &area::load(world.resource::<Content>(), area).climate;
            let held = climate
                .spell_named(&spell)
                .ok_or_else(|| format!("the area's climate has no spell named {spell}"))?;
            world.insert_resource(WeatherClock(Some(TimeCourse::Held(held))));
            tell_clocks(world);
        }
        Staging::Day { hour, turning } => {
            let cycle = world
                .resource::<Content>()
                .built::<GameSettings>()
                .daylight
                .ok_or("the game has no day cycle")?;
            if !(0.0..=24.0).contains(&hour) {
                return Err(format!("{hour} is no hour of the day"));
            }
            let course = if turning {
                cycle.turning_from(hour, world.resource::<WallClock>().now)
            } else {
                TimeCourse::Held(hour)
            };
            world.insert_resource(DayClock(Some(course)));
            tell_clocks(world);
        }
        Staging::Notify { notification } => {
            let def = notification.get(world.resource::<Content>()).clone();
            let told = def.notification(LineText::spoken(&def.text, &[]));
            notification::notify(world, player, None, told);
        }
    }
    Ok(())
}

fn nearest(
    world: &mut World,
    player: Entity,
    which: impl Fn(&World, Entity) -> bool,
) -> Option<Entity> {
    let at = world.get::<Position>(player)?.pos;
    let npcs: Vec<(Entity, Pos<Meters>)> = world
        .query_filtered::<(Entity, &Position), With<Npc>>()
        .iter(world)
        .map(|(npc, position)| (npc, position.pos))
        .collect();
    npcs.into_iter()
        .filter(|&(npc, _)| which(world, npc))
        .min_by(|a, b| a.1.distance(at).0.total_cmp(&b.1.distance(at).0))
        .map(|(npc, _)| npc)
}
