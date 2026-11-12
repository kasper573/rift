use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::stage;
use crate::core::content::RowCheck;
use crate::core::geometry::Capsule;
use crate::core::render::{
    CameraRig, CameraRigSystems, GroundHeights, Shot, ground_height, window_size, world_point,
};
use crate::core::units::{Degrees, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::{Actor, Hitbox};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DialogueCamera {
    /// From behind whoever is spoken to, over their shoulder, onto whoever speaks.
    #[default]
    Shoulder,
    /// Everyone in the conversation at once, from in front of the player.
    Participants,
    /// Where the player keeps the camera.
    Gameplay,
}

/// How the camera frames a conversation.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DialogueShots {
    pub shoulder: ShoulderShot,
    pub participants: ParticipantsShot,
}

/// Where the camera stands by whoever it looks past, in their heights, and where it aims.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShoulderShot {
    /// How far behind them.
    pub back: f32,
    /// How far to their side.
    pub aside: f32,
    /// How far over the ground they stand on.
    pub up: f32,
    /// How high on whoever it looks at it aims, in their heights.
    pub aim: f32,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParticipantsShot {
    /// How far round from straight in front of the player the camera stands.
    pub angle: Degrees,
    /// How far down it looks.
    pub pitch: Degrees,
    /// How much of everyone, from their feet up, it leaves under the frame.
    pub cropped: f32,
    /// How much of the frame it leaves free around them, on every side.
    pub margin: f32,
}

/// The least of the window's height a conversation may leave free for a shot to frame anyone in
/// it: with less the eye would stand too far off to show them.
const LEAST_FREE: f32 = 0.2;
/// A margin of half the frame on every side would leave none of it.
const MOST_MARGIN: f32 = 0.45;

/// The rules of the shots a schema can't state: an eye that stands off whoever it looks past,
/// and a frame with room in it.
pub fn check(check: &mut RowCheck, shots: &DialogueShots) {
    let DialogueShots {
        shoulder,
        participants,
    } = shots;
    check.at_least(
        "/dialogue_shots/shoulder/back",
        shoulder.back,
        0.1,
        "heights",
    );
    check.at_least("/dialogue_shots/shoulder/up", shoulder.up, 0.0, "heights");
    check.range(
        "/dialogue_shots/participants/angle",
        participants.angle.0,
        0.0,
        180.0,
        "degrees",
    );
    check.range(
        "/dialogue_shots/participants/pitch",
        participants.pitch.0,
        0.0,
        89.0,
        "degrees",
    );
    check.range(
        "/dialogue_shots/participants/cropped",
        participants.cropped,
        0.0,
        0.9,
        "of a body's height",
    );
    check.range(
        "/dialogue_shots/participants/margin",
        participants.margin,
        0.0,
        MOST_MARGIN,
        "of the frame",
    );
}

pub struct DialogueCameraPlugin;

impl Plugin for DialogueCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnExit(ClientScreen::Area), leave_shot)
            .add_systems(
                Update,
                frame_conversation
                    .in_set(CameraRigSystems)
                    .run_if(in_state(ClientScreen::Area))
                    .run_if(resource_exists::<CameraRig>),
            );
    }
}

fn leave_shot(rig: Option<ResMut<CameraRig>>) {
    if let Some(mut rig) = rig {
        rig.unstage();
    }
}

fn frame_conversation(world: &mut World) {
    let shot = shot(world);
    let mut staged = *world.resource::<CameraRig>();
    staged.stage(shot);
    // Everything that follows the camera reads the rig as changed whenever it is written.
    if staged != *world.resource::<CameraRig>() {
        *world.resource_mut::<CameraRig>() = staged;
    }
}

fn shot(world: &mut World) -> Option<Shot> {
    let cast = stage::cast(world)?;
    let me = session::me(world)?;
    let player = Body::of(world, me)?;
    let tall = player.capsule?.height;
    let shots = world.resource::<GameSettings>().dialogue_shots;
    let rig = *world.resource::<CameraRig>();
    let kept = rig.kept();
    let with = cast.with.and_then(|with| Body::of(world, with));
    let speaker = cast
        .speaker
        .and_then(|speaker| Some((speaker, Body::of(world, speaker)?)));
    let over_shoulder = || {
        let (spoken_to, speaks) = match speaker {
            Some((body, _)) if body == me => (with?, player),
            Some((_, speaker)) => (player, speaker),
            None => (player, with?),
        };
        shots.shoulder.shot(spoken_to, speaks, tall, kept)
    };
    let everyone = |world: &mut World| {
        let bodies: Vec<Body> = [me]
            .into_iter()
            .chain(cast.with)
            .chain(cast.speakers.iter().copied())
            .filter_map(|body| Body::of(world, body))
            .collect();
        let front = match with {
            Some(with) => (with.foot - player.foot).xz().try_normalize(),
            None => None,
        }
        .or_else(|| {
            let heading = world.get::<Actor>(me)?.facing.heading();
            Some(Vec2::new(heading.x, heading.y))
        })?;
        let window = window_size(world)?;
        let free = stage::rects(world)
            .iter()
            .map(|rect| rect.top())
            .fold(window.y, f32::min);
        let frame = Frame {
            window,
            free,
            fov: rig.framing().fov,
        };
        shots.participants.shot(&bodies, player, front, frame, kept)
    };
    let shot = match cast.camera {
        DialogueCamera::Gameplay => return None,
        DialogueCamera::Shoulder => over_shoulder().or_else(|| everyone(world)),
        DialogueCamera::Participants => everyone(world),
    }?;
    Some(match world.get_resource::<GroundHeights>() {
        Some(ground) => shot.over(&ground.heights),
        None => shot,
    })
}

#[derive(Clone, Copy)]
struct Body {
    foot: Vec3,
    capsule: Option<Capsule>,
}

impl Body {
    fn of(world: &World, body: Entity) -> Option<Body> {
        let at = world
            .get::<RenderPosition>(body)
            .map(|position| position.0)
            .or_else(|| world.get::<Position>(body).map(|position| position.pos))?;
        Some(Body {
            foot: world_point(at, ground_height(world, at)),
            capsule: world.get::<Hitbox>(body).map(|hitbox| hitbox.0),
        })
    }
}

impl ShoulderShot {
    /// Both sides of a conversation are shot from the same side of the line between them, the one
    /// the camera already stands on, so who stands left stays left from shot to shot.
    fn shot(&self, spoken_to: Body, speaks: Body, tall: Meters, kept: Shot) -> Option<Shot> {
        let along = (speaks.foot - spoken_to.foot).xz().try_normalize()?;
        let across = along.perp();
        let side = if (kept.eye - spoken_to.foot).xz().dot(across) < 0.0 {
            -across
        } else {
            across
        };
        let stature = |body: Body| body.capsule.map_or(tall, |capsule| capsule.height).0;
        let stand = (side * self.aside - along * self.back) * stature(spoken_to);
        Some(Shot {
            eye: spoken_to.foot + Vec3::new(stand.x, self.up * stature(spoken_to), stand.y),
            target: speaks.foot + Vec3::Y * self.aim * stature(speaks),
        })
    }
}

/// The window a shot is framed in: how much of it, from the top, the conversation leaves free.
#[derive(Clone, Copy)]
struct Frame {
    window: Vec2,
    free: f32,
    fov: Degrees,
}

impl ParticipantsShot {
    fn shot(
        &self,
        bodies: &[Body],
        player: Body,
        front: Vec2,
        frame: Frame,
        kept: Shot,
    ) -> Option<Shot> {
        [1.0, -1.0]
            .into_iter()
            .filter_map(|side| {
                let heading = Vec2::from_angle(side * self.angle.radians().0).rotate(front);
                self.framed(bodies, player, heading, frame)
            })
            .min_by(|a, b| {
                a.eye
                    .distance(kept.eye)
                    .total_cmp(&b.eye.distance(kept.eye))
            })
    }

    /// The nearest eye, standing `heading` of the player and looking back, from which every body
    /// shows whole in the free part of the frame, but for what is cropped off its feet.
    fn framed(&self, bodies: &[Body], player: Body, heading: Vec2, frame: Frame) -> Option<Shot> {
        let (sin, cos) = self.pitch.radians().0.sin_cos();
        let forward = Vec3::new(-heading.x * cos, -sin, -heading.y * cos);
        let right = forward.cross(Vec3::Y).try_normalize()?;
        let up = right.cross(forward);
        let points: Vec<Vec3> = bodies
            .iter()
            .filter_map(|body| Some((body.foot, body.capsule?)))
            .flat_map(|(foot, capsule)| {
                let low = foot + Vec3::Y * capsule.height.0 * self.cropped;
                let high = foot + Vec3::Y * capsule.height.0;
                let wide = right * capsule.radius.0;
                [low - wide, low + wide, high - wide, high + wide]
            })
            .map(|point| Vec3::new(point.dot(right), point.dot(up), point.dot(forward)))
            .collect();
        if points.is_empty() {
            return None;
        }
        let tan = (frame.fov.radians().0 / 2.0).tan();
        let spread = Vec2::new(tan * frame.window.x / frame.window.y.max(1.0), tan);
        let free = frame.free / frame.window.y.max(1.0);
        if free < LEAST_FREE {
            return None;
        }
        let margin = self.margin;
        // What is cropped off everyone's feet meets the frame's lower edge, so the margin is left
        // on its other sides alone.
        let low = Vec2::new(-1.0 + 2.0 * margin, 1.0 - 2.0 * free) * spread;
        let high = Vec2::new(1.0 - 2.0 * margin, 1.0 - 2.0 * free * margin) * spread;
        // Each side of the frame is a plane through the eye; the eye stands where the planes that
        // touch the outermost points on opposite sides meet, as far back as the tighter pair asks.
        let reach = |axis: usize| {
            let over = points
                .iter()
                .map(|point| point[axis] - high[axis] * point.z)
                .fold(f32::NEG_INFINITY, f32::max);
            let under = points
                .iter()
                .map(|point| low[axis] * point.z - point[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            (over, under, -(over + under) / (high[axis] - low[axis]))
        };
        let (across, upward) = (reach(0), reach(1));
        let depth = across.2.min(upward.2);
        let centred = |(over, under, _): (f32, f32, f32), axis: usize| {
            ((over + high[axis] * depth) + (low[axis] * depth - under)) / 2.0
        };
        let eye = right * centred(across, 0) + up * centred(upward, 1) + forward * depth;
        let chest = player.foot + Vec3::Y * player.capsule?.height.0 / 2.0;
        Some(Shot {
            eye,
            target: eye + forward * (chest - eye).dot(forward),
        })
    }
}
