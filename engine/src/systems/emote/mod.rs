pub mod wheel;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentId, ImageRef, Loading, NamedContent, Nearby, RowCheck,
};
use crate::systems::actor::skeleton::{Declared, GestureName};
use crate::systems::actor::{
    Action, Actor, ActorModel, Appearance, Motion, Playing, Pose, held, playing,
};
use crate::systems::player::{PlayedBy, player_requests};
use crate::systems::{movement, reach};

pub type EmoteId = ContentId<EmoteDef>;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<EmoteRequest>();
}

/// Something a player has their character do on a whim, picked off the wheel of emotes: a
/// gesture made once, or a pose held until the character moves or does anything else. A
/// character is offered the emotes its body has a clip for.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct EmoteDef {
    pub name: String,
    pub icon: ImageRef,
    #[serde(flatten)]
    pub does: EmoteMotion,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmoteMotion {
    /// A gesture by the name a skeleton's sets give it, made once.
    Gesture(GestureName),
    /// A pose taken and held; picked again while held, it is left.
    Pose(Pose),
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct EmoteRequest {
    pub emote: EmoteId,
}

pub fn requests(world: &mut World) {
    player_requests::<EmoteRequest>(world, true, |world, player, request| {
        perform(world, player, request.emote)
    });
}

/// Has an actor do an emote where it stands, stopping its walk and whatever it was reaching
/// for: a gesture from standing, a pose taken, or left when it is the one held.
pub fn perform(world: &mut World, actor: Entity, emote: EmoteId) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let def = emote.get(&content);
    if !offered(world, actor, def) {
        return Err(format!("You can't {} like this", def.name.to_lowercase()));
    }
    reach::drop_intent(world, actor);
    movement::halt(world, actor);
    match &def.does {
        EmoteMotion::Gesture(gesture) => {
            if let Some(mut standing) = world.get_mut::<Actor>(actor) {
                standing.pose = Pose::Idle;
            }
            playing::play(world, actor, Motion::Gesture(gesture.clone()), None);
        }
        EmoteMotion::Pose(pose) => {
            let held = world.get::<Actor>(actor).map(|actor| actor.pose);
            let to = if held == Some(*pose) {
                Pose::Idle
            } else {
                *pose
            };
            playing::pose_to(world, actor, to);
        }
    }
    Ok(())
}

/// Whether an actor's body can do the emote: it has a clip for the gesture, or for holding the
/// pose.
pub fn offered(world: &World, actor: Entity, emote: &EmoteDef) -> bool {
    match &emote.does {
        EmoteMotion::Gesture(gesture) => {
            playing::plays(world, actor, &Motion::Gesture(gesture.clone()))
        }
        EmoteMotion::Pose(pose) => {
            let content = world.resource::<Content>();
            world.get::<Appearance>(actor).is_some_and(|appearance| {
                ActorModel::get(content, appearance.model)
                    .holds(held::stance_of(world, actor), *pose)
            })
        }
    }
}

/// Stands every player's character back up out of a pose once it does anything but hold it:
/// walks, reaches for something, talks, fights, or plays anything but the pose's own clips.
pub fn rise(world: &mut World, posed: &mut QueryState<(Entity, &Actor), With<PlayedBy>>) {
    let risen: Vec<Entity> = posed
        .iter(world)
        .filter(|(_, actor)| actor.pose != Pose::Idle)
        .filter(|(entity, actor)| {
            let busy = world.get::<Playing>(*entity).is_some_and(|playing| {
                !matches!(playing.motion, Motion::Begin(_) | Motion::End(_))
            });
            busy || actor.action != Action::Idle
                || movement::moving(world, *entity)
                || reach::current(world, *entity).is_some()
        })
        .map(|(entity, _)| entity)
        .collect();
    for entity in risen {
        if let Some(mut actor) = world.get_mut::<Actor>(entity) {
            actor.pose = Pose::Idle;
        }
    }
}

/// Every emote makes a gesture some skeleton's sets declare, or takes a pose other than idling.
pub fn check_all(loading: &mut Loading) {
    let content = loading.content();
    let declared = content.built::<Declared>();
    for (id, emote) in loading.rows::<EmoteDef>(&content) {
        let mut check = RowCheck::default();
        match &emote.does {
            EmoteMotion::Gesture(gesture) if !declared.gestures.contains(gesture) => check.error(
                "/gesture",
                "undeclared gesture",
                gesture.as_str(),
                "a gesture some skeleton's set declares",
            ),
            EmoteMotion::Pose(Pose::Idle) => check.error(
                "/pose",
                "no pose",
                "idle",
                "a pose to take: idling is what leaving one comes back to",
            ),
            _ => {}
        }
        loading.row_errors(id, check);
    }
}

impl ContentFile for EmoteDef {
    const TABLE: &'static str = "emote";

    fn fresh(near: &Nearby) -> Result<EmoteDef, String> {
        Ok(EmoteDef {
            name: near.name(),
            icon: near.image()?,
            does: EmoteMotion::Pose(Pose::Sit),
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 24);
    }
}

impl NamedContent for EmoteDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}
