use bevy::prelude::{IntoScheduleConfigs, Plugin, Time, Update, in_state};
use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replication::{Interest, Remote, Replicated, ReplicationInterest, ToClient};

use crate::core::chunk::{CHUNK_SIDE, Chunk, ChunkIndex};
use crate::core::interpolate::RenderValueSystems;
use crate::core::math::Pos;
use crate::core::render::Look;
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::world::opacity::Opacity;
use crate::core::tick::Planner;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::movement::{Position, RenderPosition, position};
use crate::systems::player::session::MyCharacter;
use crate::systems::player::{ClientId, Connected, PlayedBy, Players};
use crate::systems::screen::ClientScreen;
use crate::systems::script::condition::CompiledCondition;

/// How far the game's players see.
pub fn sight(world: &World) -> Meters {
    world.resource::<GameSettings>().sight
}

pub fn register(app: &mut App) {
    app.insert_resource(ReplicationInterest(Box::new(ClientSight::default())));
}

/// Where toward the edge of its character's sight a client starts to fade what stands there, and
/// where it has faded it whole, as shares of the sight: a little short of the edge, past which
/// the server tells it of the thing no more.
const FADES_FROM: f32 = 0.8;
const FADED_BY: f32 = 0.95;

/// Fades what a client's character sees at the edge of its sight, so a thing coming into sight
/// or leaving it thins in or out, with all it sheds, rather than appearing or vanishing at once.
pub struct SightEdgePlugin;

impl Plugin for SightEdgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (fade_arrivals, fade_at_the_edge)
                .chain()
                .after(RenderValueSystems)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

/// How much of a thing `away` from a character that sees `sight` far shows.
fn shown_at(away: Meters, sight: Meters) -> f32 {
    let (from, by) = (sight.0 * FADES_FROM, sight.0 * FADED_BY);
    let faded = ((away.0 - from) / (by - from).max(f32::EPSILON)).clamp(0.0, 1.0);
    1.0 - faded * faded * (3.0 - 2.0 * faded)
}

/// Since when a thing has been coming into the world a client is shown, or going out of it:
/// what does so within its character's sight fades as the game's look says, rather than
/// appearing or vanishing at once.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum PresenceFade {
    In(Seconds),
    Out(Seconds),
}

impl PresenceFade {
    /// How much of the thing shows `now`, when its fade takes `fades`.
    fn shown(self, now: Seconds, fades: Seconds) -> f32 {
        let along = |since: Seconds| ((now - since).0 / fades.0.max(f32::EPSILON)).clamp(0.0, 1.0);
        match self {
            PresenceFade::In(since) => along(since),
            PresenceFade::Out(since) => 1.0 - along(since),
        }
    }
}

/// What the server has just begun to show a client.
type Arrived = (Added<Position>, With<Remote>, Without<PresenceFade>);

/// Where something stands and is drawn, and how much of it shows.
type Placed = (
    Entity,
    &'static Position,
    Option<&'static RenderPosition>,
    Option<&'static Opacity>,
    Option<&'static PresenceFade>,
);

/// Starts fading in whatever the server begins to show a client whose character already stands
/// in a world on view: what joins a world it is looking at, not the world it arrives in.
fn fade_arrivals(
    viewpoint: Res<MyCharacter>,
    transition: Res<ActiveTransition>,
    look: Res<Look>,
    time: Res<Time>,
    arrived: Query<Entity, Arrived>,
    mut commands: Commands,
) {
    let looking = viewpoint.0.is_some()
        && !viewpoint.is_changed()
        && transition.phase() == ScreenTransitionPhase::Idle;
    if !looking || look.presence_fades.is_zero() {
        return;
    }
    let now = Seconds(time.elapsed_secs());
    for entity in &arrived {
        commands
            .entity(entity)
            .try_insert((PresenceFade::In(now), Opacity(0.0)));
    }
}

fn fade_at_the_edge(
    viewpoint: Res<MyCharacter>,
    settings: Res<GameSettings>,
    look: Res<Look>,
    time: Res<Time>,
    placed: Query<Placed>,
    mut commands: Commands,
) {
    let Some(mine) = viewpoint.0 else {
        return;
    };
    let stands =
        |at: &Position, shown: Option<&RenderPosition>| shown.map_or(at.pos, |shown| shown.0);
    let Ok(eye) = placed.get(mine).map(|(_, at, shown, ..)| stands(at, shown)) else {
        return;
    };
    let now = Seconds(time.elapsed_secs());
    for (entity, at, shown, opacity, fade) in &placed {
        let present = fade.map_or(1.0, |fade| fade.shown(now, look.presence_fades));
        match fade {
            Some(PresenceFade::Out(_)) if present <= 0.0 => {
                commands.entity(entity).try_despawn();
                continue;
            }
            Some(PresenceFade::In(_)) if present >= 1.0 => {
                commands.entity(entity).try_remove::<PresenceFade>();
            }
            _ => {}
        }
        let wanted = if entity == mine {
            1.0
        } else {
            shown_at(eye.distance(stands(at, shown)), settings.sight) * present
        };
        if opacity.map_or(1.0, |opacity| opacity.0) != wanted {
            commands.entity(entity).try_insert(Opacity(wanted));
        }
    }
}

/// Whom something is there for; everyone, without one.
#[derive(Component, Clone)]
pub enum Presence {
    For(ClientId),
    When(CompiledCondition),
    /// For the characters `viewers` has it there for, each as `shown` holds for it when it is
    /// there on a condition too.
    Among {
        viewers: Viewers,
        shown: Option<CompiledCondition>,
    },
}

/// The characters something is there for, by who they are.
#[derive(Clone, Debug, PartialEq)]
pub enum Viewers {
    /// These alone: an npc that is away stays for whoever is in the middle of something with it.
    Only(Vec<Entity>),
    /// All but these: what an npc leaves behind is not seen by those it stays for.
    Except(Vec<Entity>),
}

impl Viewers {
    fn has(&self, viewer: Entity) -> bool {
        match self {
            Viewers::Only(these) => these.contains(&viewer),
            Viewers::Except(these) => !these.contains(&viewer),
        }
    }
}

pub fn present(world: &World, subject: Entity, viewer: Entity) -> bool {
    match world.get::<Presence>(subject) {
        None => true,
        Some(Presence::For(client)) => world
            .get::<PlayedBy>(viewer)
            .is_some_and(|owner| owner.client == *client),
        Some(Presence::When(shown)) => shown.holds(world, viewer),
        Some(Presence::Among { viewers, shown }) => {
            viewers.has(viewer)
                && shown
                    .as_ref()
                    .is_none_or(|shown| shown.holds(world, viewer))
        }
    }
}

pub fn coexist(world: &World, a: Entity, b: Entity) -> bool {
    present(world, a, b) && present(world, b, a)
}

/// Whether `subject` is among what `viewer` is shown: present to each other, in the same area
/// within [`sight`]. What a player may aim at, reach for or pick up, since their client
/// shows nothing else; a request for anything farther didn't come from what they see.
pub fn in_sight(world: &World, viewer: Entity, subject: Entity) -> bool {
    coexist(world, viewer, subject)
        && area::same_area(world, viewer, subject)
        && matches!(
            (position(world, viewer), position(world, subject)),
            (Some(at), Some(there)) if at.distance(there) <= sight(world)
        )
}

/// Has `subject` there for `viewers` alone of those it is there for, or for all of them again
/// with none. What is there for one client alone stays theirs.
pub fn show_among(world: &mut World, subject: Entity, viewers: Option<Viewers>) {
    let (now, shown) = match world.get::<Presence>(subject) {
        Some(Presence::For(_)) => return,
        Some(Presence::When(shown)) => (None, Some(shown)),
        Some(Presence::Among { viewers, shown }) => (Some(viewers), shown.as_ref()),
        None => (None, None),
    };
    if now == viewers.as_ref() {
        return;
    }
    let shown = shown.cloned();
    let mut subject = world.entity_mut(subject);
    match (viewers, shown) {
        (Some(viewers), shown) => {
            subject.insert(Presence::Among { viewers, shown });
        }
        (None, Some(shown)) => {
            subject.insert(Presence::When(shown));
        }
        (None, None) => {
            subject.remove::<Presence>();
        }
    }
}

/// What a client sees: everything replicated that stands nowhere, its own character, and what
/// stands in its character's area within sight of it; of all those, only what is present
/// to its character. Only the client playing a character is told its private components.
#[derive(Default)]
pub struct ClientSight {
    sight: Meters,
    unplaced: Vec<Entity>,
    /// Every replicated entity that stands somewhere, by its index, as a view checks it: read
    /// once per send rather than once per client that looks its way.
    standing: Vec<Option<Standing>>,
}

/// What a view checks of an entity standing in a chunk.
#[derive(Clone, Copy)]
struct Standing {
    entity: Entity,
    at: Pos<Meters>,
    area: Option<AreaId>,
    shown: Shown,
}

#[derive(Clone, Copy)]
enum Shown {
    ToAll,
    To(ClientId),
    ToNone,
    /// On a condition of the viewer, asked of the world.
    When,
}

impl Interest for ClientSight {
    fn prepare(&mut self, world: &mut World) {
        self.sight = sight(world);
        self.unplaced = world
            .query_filtered::<Entity, (With<Replicated>, Without<Position>)>()
            .iter(world)
            .collect();
        self.standing.clear();
        self.standing.resize(world.entities().len() as usize, None);
        let mut placed = world.query_filtered::<
            (Entity, &Position, Option<&AreaTag>, Option<&Presence>),
            With<Replicated>,
        >();
        for (entity, position, tag, presence) in placed.iter(world) {
            self.standing[entity.index_u32() as usize] = Some(Standing {
                entity,
                at: position.pos,
                area: tag.map(|tag| tag.area),
                shown: match presence {
                    None => Shown::ToAll,
                    Some(Presence::For(client)) => Shown::To(*client),
                    Some(Presence::When(shown)) => match shown.shared(world) {
                        Some(true) => Shown::ToAll,
                        Some(false) => Shown::ToNone,
                        None => Shown::When,
                    },
                    Some(Presence::Among { .. }) => Shown::When,
                },
            });
        }
    }

    /// What stands in the chunks around the character, as the chunks are filed this tick.
    fn seen_by(&self, world: &World, client: Entity, seen: &mut Vec<Entity>) {
        let Some(view) = world
            .get::<ClientId>(client)
            .and_then(|&id| view(world, id))
        else {
            return;
        };
        seen.extend(
            self.unplaced
                .iter()
                .copied()
                .chain(std::iter::once(view.focus))
                .filter(|&subject| view.sees(world, subject)),
        );
        let (Some(pos), Some(area)) = (view.pos, view.area) else {
            return;
        };
        let chunks = world.resource::<ChunkIndex>();
        let reach = self.sight.0 * self.sight.0;
        for chunk in Chunk::around(pos, self.sight) {
            seen.extend(
                chunks
                    .in_chunk(chunk)
                    .iter()
                    .copied()
                    .filter(|&subject| subject != view.focus)
                    .filter(
                        |&subject| match self.standing.get(subject.index_u32() as usize) {
                            Some(Some(standing)) if standing.entity == subject => {
                                standing.area == Some(area)
                                    && (pos - standing.at).square_length() <= reach
                                    && match standing.shown {
                                        Shown::ToAll => true,
                                        Shown::To(client) => view.client == client,
                                        Shown::ToNone => false,
                                        Shown::When => present(world, subject, view.focus),
                                    }
                            }
                            _ => view.sees(world, subject),
                        },
                    ),
            );
        }
    }

    fn owns(&self, world: &World, client: Entity, entity: Entity) -> bool {
        world
            .get::<PlayedBy>(entity)
            .is_some_and(|played| world.get::<ClientId>(client) == Some(&played.client))
    }
}

/// The clients that see `entity`: those whose characters stand in the chunks around it, as the
/// chunks were filed last tick (hence a chunk of margin), and see it where it stands now.
pub fn seen_by(world: &World, entity: Entity) -> Vec<Entity> {
    let Some(at) = position(world, entity) else {
        return Vec::new();
    };
    let connected = world.resource::<Connected>();
    world
        .resource::<ChunkIndex>()
        .around(at, Meters(sight(world).0 + CHUNK_SIDE.0))
        .filter_map(|near| world.get::<PlayedBy>(near).map(|played| played.client))
        .filter(|&client| view(world, client).is_some_and(|view| view.sees(world, entity)))
        .filter_map(|client| connected.connection(client))
        .collect()
}

/// The clients that play `player`.
pub fn private_viewers(world: &World, player: Entity) -> Vec<Entity> {
    let Some(owner) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
        return Vec::new();
    };
    world
        .resource::<Connected>()
        .connection(owner)
        .into_iter()
        .collect()
}

pub fn send_private<M: Message + Clone>(world: &mut World, player: Entity, message: M) {
    for conn in private_viewers(world, player) {
        send_to(world, conn, message.clone());
    }
}

pub fn send_to<M: Message>(world: &mut World, conn: Entity, message: M) {
    world.write_message(ToClient {
        client: conn,
        message,
    });
}

/// Where a client looks from: its character, where it stands.
struct View {
    client: ClientId,
    focus: Entity,
    pos: Option<Pos<Meters>>,
    area: Option<AreaId>,
}

fn view(world: &World, client: ClientId) -> Option<View> {
    let focus = world.resource::<Players>().0.get(&client).copied()?;
    Some(View {
        client,
        focus,
        pos: position(world, focus),
        area: world.get::<AreaTag>(focus).map(|tag| tag.area),
    })
}

impl View {
    fn sees(&self, world: &World, subject: Entity) -> bool {
        if subject == self.focus {
            return true;
        }
        if world.get::<Replicated>(subject).is_none() || !present(world, subject, self.focus) {
            return false;
        }
        let Some(at) = position(world, subject) else {
            return true;
        };
        let (Some(pos), Some(area)) = (self.pos, self.area) else {
            return false;
        };
        world.get::<AreaTag>(subject).map(|tag| tag.area) == Some(area)
            && pos.distance(at) <= sight(world)
    }
}

/// Tells every client what changed in its sight, each replication tick, planned beside the rest
/// of the tick from the world as the tick found it.
#[derive(Default)]
pub struct SendSights(bevy_replication::Sender);

impl Planner for SendSights {
    fn prepare(&mut self, world: &mut World) {
        if crate::systems::replication_tick(world) {
            self.0.prepare(world);
        }
    }

    fn plan(&mut self, world: &World) {
        self.0.plan(world);
    }

    fn apply(&mut self, world: &mut World) {
        self.0.apply(world);
    }
}
