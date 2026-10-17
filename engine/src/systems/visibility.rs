use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::{
    AppVisibilityExt, Replicated, ScopeLifetime, SendTargets, ToClients, VisibilityFilter,
};
use bevy_replicon::server::visibility::client_visibility::ClientVisibility;
use bevy_replicon::server::visibility::filters_mask::FilterBit;
use bevy_replicon::server::visibility::registry::FilterRegistry;
use bevy_replicon::shared::replication::registry::ReplicationRegistry;
use fixedbitset::FixedBitSet;

use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::attention::Attention;
use crate::systems::dialogue::Conversation;
use crate::systems::item::Inventory;
use crate::systems::movement::{HeardMove, MoveTarget, Position, position};
use crate::systems::player::{ArrivalHold, ClientId, CommandLock, Owner, Players};
use crate::systems::quest::QuestLog;
use crate::systems::script::condition::CompiledCondition;
use crate::systems::shop::ShopView;
use crate::systems::skill::ResetPrompt;

pub const VIEW_DISTANCE: Tiles = Tiles(24.0);
// Compared against squared distance to avoid a per-pair `sqrt`. Exact: `VIEW_DISTANCE` is a perfect
// square in f32, so `d <= VIEW_DISTANCE` iff `d² <= VIEW_DISTANCE²` for all inputs.
const VIEW_DISTANCE_SQ: f32 = VIEW_DISTANCE.0 * VIEW_DISTANCE.0;

pub fn register(app: &mut App) {
    app.add_visibility_filter::<OwnedBy>();
    app.init_resource::<ScopeBits>();
    app.init_resource::<Sights>();
    app.init_resource::<Slots>();
    app.add_observer(grant_hidden_slots);
    app.add_observer(release_slot);
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
            .get::<Owner>(viewer)
            .is_some_and(|owner| owner.client == *client),
        Some(Presence::When(shown)) => shown.holds(world, viewer),
    }
}

pub fn coexist(world: &World, a: Entity, b: Entity) -> bool {
    present(world, a, b) && present(world, b, a)
}

type Clients = QueryState<(Entity, &'static ClientId)>;
type Views = QueryState<(&'static mut ClientVisibility, &'static mut HiddenSlots)>;
type Subjects = QueryState<
    (
        Entity,
        &'static Position,
        Option<&'static AreaTag>,
        Option<&'static SubjectSlot>,
    ),
    With<Replicated>,
>;
type Presences = QueryState<Entity, (With<Presence>, With<Replicated>)>;

/// Replicon is told only about range transitions: a client remembers which subjects it hides in a
/// bitset over the subjects' slots, so the steady state costs no hashing per subject and client.
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
    let found: Vec<(Subject, Option<SubjectSlot>)> = subjects_query
        .iter(world)
        .map(|(entity, position, tag, slot)| {
            let subject = Subject {
                entity,
                pos: position.pos,
                area: tag.map(|tag| tag.area),
            };
            (subject, slot.copied())
        })
        .collect();
    let subjects: Vec<(Subject, usize)> = found
        .into_iter()
        .map(|(subject, slot)| {
            let slot = slot.unwrap_or_else(|| {
                let slot = SubjectSlot(world.resource_mut::<Slots>().allocate());
                world.entity_mut(subject.entity).insert(slot);
                slot
            });
            (subject, slot.0)
        })
        .collect();
    let slots = world.resource::<Slots>().len();
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
        for (subject, slot) in &subjects {
            let hide = !sight.as_ref().is_some_and(|sight| sight.sees(subject));
            if hidden.0.contains(*slot) != hide {
                hidden.0.set(*slot, hide);
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
    let clients: Vec<(Entity, ClientId)> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
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
    let Some(owner) = world.get::<Owner>(player).map(|owner| owner.client) else {
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
    type Scope = (
        Inventory,
        Conversation,
        CommandLock,
        ArrivalHold,
        Attention,
        ShopView,
        QuestLog,
        MoveTarget,
        HeardMove,
        ResetPrompt,
    );

    fn is_visible(&self, _: Entity, client: Option<&ClientId>) -> bool {
        client == Some(&self.0)
    }
}

fn grant_hidden_slots(add: On<Add<ClientId>>, mut commands: Commands) {
    commands.entity(add.entity).insert(HiddenSlots::default());
}

#[derive(Component, Clone, Copy)]
#[component(immutable)]
pub struct SubjectSlot(usize);

#[derive(Component, Default)]
pub struct HiddenSlots(FixedBitSet);

#[derive(Resource, Default)]
struct Slots {
    free: Vec<usize>,
    len: usize,
}

impl Slots {
    fn allocate(&mut self) -> usize {
        self.free.pop().unwrap_or_else(|| {
            self.len += 1;
            self.len - 1
        })
    }

    fn len(&self) -> usize {
        self.len
    }
}

fn release_slot(
    remove: On<Remove<SubjectSlot>>,
    subjects: Query<&SubjectSlot>,
    mut slots: ResMut<Slots>,
    mut clients: Query<&mut HiddenSlots>,
) {
    let Ok(&SubjectSlot(slot)) = subjects.get(remove.entity) else {
        return;
    };
    slots.free.push(slot);
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
struct Sights(QueryState<(Entity, &'static ClientId)>);

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
    pos: Pos<Tiles>,
    area: Option<AreaId>,
}

struct Sight {
    focus: Entity,
    pos: Option<Pos<Tiles>>,
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
