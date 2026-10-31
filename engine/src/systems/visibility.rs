use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::{
    AppVisibilityExt, FilterScope, Replicated, ScopeLifetime, SendTargets, SingleComponent,
    ToClients, VisibilityFilter,
};
use bevy_replicon::server::visibility::client_visibility::ClientVisibility;
use bevy_replicon::server::visibility::filters_mask::FilterBit;
use bevy_replicon::server::visibility::registry::FilterRegistry;
use bevy_replicon::shared::replication::registry::ReplicationRegistry;
use bevy_replicon::shared::replication::registry::receive_fns::MutWrite;
use bevy_replicon::shared::replication::visibility::VisibilityScope;
use fixedbitset::FixedBitSet;

use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::mirror::AppMirrorExt;
use crate::systems::movement::{Position, position};
use crate::systems::player::{ClientId, PlayedBy, Players};
use crate::systems::script::condition::CompiledCondition;

pub const VIEW_DISTANCE: Meters = Meters(24.0);
// Compared against squared distance to avoid a per-pair `sqrt`. Exact: `VIEW_DISTANCE` is a perfect
// square in f32, so `d <= VIEW_DISTANCE` iff `d² <= VIEW_DISTANCE²` for all inputs.
const VIEW_DISTANCE_SQ: f32 = VIEW_DISTANCE.0 * VIEW_DISTANCE.0;

pub fn register(app: &mut App) {
    app.mirror::<Presence>();
    app.add_visibility_filter::<OwnedBy>();
    app.init_resource::<ScopeBits>();
    app.add_observer(grant_hidden_slots);
    app.add_observer(forget_subject);
    app.add_observer(reveal_to_all);
}

#[derive(Component, Clone)]
pub enum Presence {
    For(ClientId),
    When(CompiledCondition),
}

pub fn present(world: &World, subject: Entity, viewer: Entity) -> bool {
    match world.get::<Presence>(subject) {
        None => true,
        Some(Presence::For(client)) => world
            .get::<PlayedBy>(viewer)
            .is_some_and(|owner| owner.client == *client),
        Some(Presence::When(shown)) => shown.holds(world, viewer),
    }
}

pub fn coexist(world: &World, a: Entity, b: Entity) -> bool {
    present(world, a, b) && present(world, b, a)
}

type Clients = QueryState<(Entity, &'static ClientId)>;
type Views = QueryState<(&'static mut ClientVisibility, &'static mut HiddenSlots)>;
type Subjects = QueryState<(Entity, &'static Position, Option<&'static AreaTag>), With<Replicated>>;
type Presences = QueryState<Entity, (With<Presence>, With<Replicated>)>;

/// Replicon is told only about range transitions: a client remembers which subjects it hides in a
/// bitset over the subjects' entity indices, so the steady state costs no hashing per subject and
/// client.
pub fn update(
    world: &mut World,
    clients_query: &mut Clients,
    views: &mut Views,
    subjects_query: &mut Subjects,
    presences: &mut Presences,
) {
    let clients: Vec<(Entity, ClientId)> = clients_query
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
    if clients.is_empty() {
        return;
    }
    let bits = *world.resource::<ScopeBits>();
    let conditional: Vec<Entity> = presences.iter(world).collect();
    let subjects: Vec<Subject> = subjects_query
        .iter(world)
        .map(|(entity, position, tag)| Subject {
            entity,
            pos: position.pos,
            area: tag.map(|tag| tag.area),
        })
        .collect();
    let slots = subjects
        .iter()
        .map(|subject| slot_of(subject.entity) + 1)
        .max()
        .unwrap_or(0);
    for (client, id) in clients {
        let sight = sight(world, id);
        let present: Vec<(Entity, bool)> = conditional
            .iter()
            .map(|&subject| {
                let shown = sight
                    .as_ref()
                    .is_some_and(|sight| present(world, subject, sight.focus));
                (subject, shown)
            })
            .collect();
        let Ok((mut visibility, mut hidden)) = views.get_mut(world, client) else {
            continue;
        };
        let visibility = visibility.as_mut();
        let hidden = hidden.as_mut();
        hidden.0.grow(slots);
        for subject in &subjects {
            let slot = slot_of(subject.entity);
            let hide = !sight.as_ref().is_some_and(|sight| sight.sees(subject));
            if hidden.0.contains(slot) != hide {
                hidden.0.set(slot, hide);
                visibility.set(subject.entity, bits.range, !hide);
            }
        }
        for (subject, shown) in present {
            visibility.set(subject, bits.presence, shown);
        }
    }
}

pub fn seen_by(world: &mut World, entity: Entity) -> Vec<Entity> {
    let Some(pos) = position(world, entity) else {
        return Vec::new();
    };
    let subject = Subject {
        entity,
        pos,
        area: world.get::<AreaTag>(entity).map(|tag| tag.area),
    };
    let clients: Vec<(Entity, ClientId)> =
        world.resource_scope(|world, mut sights: Mut<Sights>| {
            sights
                .0
                .iter(world)
                .map(|(entity, &id)| (entity, id))
                .collect()
        });
    clients
        .into_iter()
        .filter(|&(_, id)| {
            sight(world, id)
                .is_some_and(|sight| sight.sees(&subject) && present(world, entity, sight.focus))
        })
        .map(|(client, _)| client)
        .collect()
}

pub fn private_viewers(world: &mut World, player: Entity) -> Vec<Entity> {
    let Some(owner) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
        return Vec::new();
    };
    world.resource_scope(|world, mut sights: Mut<Sights>| {
        sights
            .0
            .iter(world)
            .filter(|(_, client)| **client == owner)
            .map(|(conn, _)| conn)
            .collect()
    })
}

pub fn send_private<M: Message + Clone>(world: &mut World, player: Entity, message: M) {
    for conn in private_viewers(world, player) {
        send_to(world, conn, message.clone());
    }
}

pub fn send_to<M: Message>(world: &mut World, conn: Entity, message: M) {
    world.write_message(ToClients {
        targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(conn)),
        message,
    });
}

#[derive(Component)]
#[component(immutable)]
pub struct OwnedBy(pub ClientId);

impl VisibilityFilter for OwnedBy {
    type ClientComponent = ClientId;
    type Scope = OwnersOnly;

    fn is_visible(&self, _: Entity, client: Option<&ClientId>) -> bool {
        client == Some(&self.0)
    }
}

/// What a character carries that only its own client is told of: whatever a system declares
/// private, before this world's visibility is registered.
pub struct OwnersOnly;

pub fn keep_private<C: Component<Mutability: MutWrite<C>>>(app: &mut App) {
    let mut private = app.world_mut().get_resource_or_init::<Private>();
    assert!(
        !private.scoped,
        "{} is declared private after visibility was registered",
        std::any::type_name::<C>()
    );
    private.scopes.push(SingleComponent::<C>::visibility_scope);
}

#[derive(Resource, Default)]
struct Private {
    scopes: Vec<fn(&mut World, &mut ReplicationRegistry) -> VisibilityScope>,
    scoped: bool,
}

impl FilterScope for OwnersOnly {
    fn visibility_scope(world: &mut World, registry: &mut ReplicationRegistry) -> VisibilityScope {
        let scopes = {
            let mut private = world.get_resource_or_init::<Private>();
            private.scoped = true;
            std::mem::take(&mut private.scopes)
        };
        let mut masks = scopes
            .into_iter()
            .filter_map(|scope| match scope(world, registry) {
                VisibilityScope::Components(mask) => Some(mask),
                _ => None,
            });
        let mut hidden = masks
            .next()
            .expect("a private component declared before visibility");
        for mask in masks {
            hidden |= &mask;
        }
        VisibilityScope::Components(hidden)
    }
}

fn grant_hidden_slots(add: On<Add<ClientId>>, mut commands: Commands) {
    commands.entity(add.entity).insert(HiddenSlots::default());
}

/// The subjects a client hides, by entity index; an index a despawned subject leaves behind is
/// cleared, so whatever is spawned into it starts shown as replicon has it.
#[derive(Component, Default)]
pub struct HiddenSlots(FixedBitSet);

fn slot_of(subject: Entity) -> usize {
    subject.index_u32() as usize
}

fn forget_subject(remove: On<Remove<Replicated>>, mut clients: Query<&mut HiddenSlots>) {
    let slot = slot_of(remove.entity);
    for mut hidden in &mut clients {
        if slot < hidden.0.len() {
            hidden.0.set(slot, false);
        }
    }
}

fn reveal_to_all(
    remove: On<Remove<Presence>>,
    bits: Res<ScopeBits>,
    mut clients: Query<&mut ClientVisibility>,
) {
    for mut visibility in &mut clients {
        visibility.set(remove.entity, bits.presence, true);
    }
}

#[derive(Resource)]
pub(crate) struct Sights(QueryState<(Entity, &'static ClientId)>);

impl FromWorld for Sights {
    fn from_world(world: &mut World) -> Self {
        Sights(world.query())
    }
}

#[derive(Resource, Clone, Copy)]
struct ScopeBits {
    range: FilterBit,
    presence: FilterBit,
}

impl FromWorld for ScopeBits {
    fn from_world(world: &mut World) -> Self {
        ScopeBits {
            range: entity_scope_bit(world),
            presence: entity_scope_bit(world),
        }
    }
}

fn entity_scope_bit(world: &mut World) -> FilterBit {
    world.resource_scope(|world, mut filters: Mut<FilterRegistry>| {
        world.resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
            filters.register_scope::<Entity>(world, &mut registry, ScopeLifetime::WhileVisible)
        })
    })
}

struct Subject {
    entity: Entity,
    pos: Pos<Meters>,
    area: Option<AreaId>,
}

struct Sight {
    focus: Entity,
    pos: Option<Pos<Meters>>,
    area: Option<AreaId>,
}

fn sight(world: &World, client: ClientId) -> Option<Sight> {
    let focus = world.resource::<Players>().0.get(&client).copied()?;
    Some(Sight {
        focus,
        pos: position(world, focus),
        area: world.get::<AreaTag>(focus).map(|tag| tag.area),
    })
}

impl Sight {
    fn sees(&self, subject: &Subject) -> bool {
        if subject.entity == self.focus {
            return true;
        }
        let (Some(pos), Some(area)) = (self.pos, self.area) else {
            return false;
        };
        subject.area == Some(area) && (pos - subject.pos).square_length() <= VIEW_DISTANCE_SQ
    }
}
