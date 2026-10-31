use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{StatusEffectId, Statuses, boost};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::time::{Seconds, UnixMillis, WallClock};
use crate::core::vfx::VfxSite;
use crate::systems::actor::Actor;
use crate::systems::equipment::Equipment;
use crate::systems::item::{Inventory, ItemId, ItemKind};
use crate::systems::mirror::Ghost;
use crate::systems::notification;
use crate::systems::script::effect::Audience;
use crate::systems::script::event::RiftEvent;
use crate::systems::script::{EventKind, Owner, OwnerRow, RiftInstances, RiftScripts};
use crate::systems::skill::{self, Learned, Rank, SkillId, SkillKind};
use crate::systems::vfx::{self, VfxShaderFixture};

/// How often a status's `during` sound repeats when no `during` vfx sets the pace.
const DURING_SOUND_PERIOD: Seconds = Seconds(2.0);

/// Where an actor's aspects come from: a status it carries, an item it wears or carries as a
/// resource, a passive skill it knows. Each is a scripted row, active while that holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AspectSource {
    Status(StatusEffectId),
    Item(ItemId),
    Skill(SkillId),
}

impl AspectSource {
    pub fn owner(self) -> Owner {
        let row = self.row();
        Owner::Row(row.kind, row.row)
    }

    pub fn row(self) -> OwnerRow {
        match self {
            AspectSource::Status(id) => OwnerRow::of_row(id),
            AspectSource::Item(id) => OwnerRow::of_row(id),
            AspectSource::Skill(id) => OwnerRow::of_row(id),
        }
    }
}

/// The aspects active on an actor, with the clocks of their interval nodes and of a status's
/// `during` vfx.
#[derive(Component, Default)]
pub struct ActiveAspects(pub Vec<ActiveAspect>);

pub struct ActiveAspect {
    pub source: AspectSource,
    /// When the status behind it was applied: a status applied again is a fresh aspect.
    since: Option<UnixMillis>,
    cadences: Vec<Cadence>,
    during: Option<Entity>,
    during_next: Option<UnixMillis>,
}

struct Cadence {
    seconds: Seconds,
    next: UnixMillis,
}

type Stirred = Or<(
    Added<Actor>,
    Changed<Equipment>,
    Changed<Inventory>,
    Changed<Learned>,
    Changed<Statuses>,
)>;

/// Brings every actor whose statuses, gear, bag or learning changed to the aspects it should have:
/// a new one activates, one no longer wanted deactivates, and a status applied again does both.
pub fn settle(world: &mut World, stirred: &mut QueryState<Entity, (Stirred, Without<Ghost>)>) {
    let actors: Vec<Entity> = stirred.iter(world).collect();
    for actor in actors {
        let wanted = desired(world, actor);
        let active: Vec<Wanted> = world
            .get::<ActiveAspects>(actor)
            .map(|aspects| {
                aspects
                    .0
                    .iter()
                    .map(|aspect| Wanted {
                        source: aspect.source,
                        since: aspect.since,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for gone in active.iter().filter(|aspect| !wanted.contains(aspect)) {
            deactivate(world, actor, gone.source);
        }
        for new in wanted.iter().filter(|aspect| !active.contains(aspect)) {
            activate(world, actor, *new);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Wanted {
    source: AspectSource,
    since: Option<UnixMillis>,
}

/// Fires the interval nodes whose time came and replays a status's `during` vfx and sound.
pub fn tick(world: &mut World, active: &mut QueryState<(Entity, &ActiveAspects)>) {
    let now = world.resource::<WallClock>().now;
    let mut fired: Vec<(Entity, AspectSource, Seconds)> = Vec::new();
    let mut replays: Vec<(Entity, AspectSource)> = Vec::new();
    for (actor, aspects) in active.iter(world) {
        for aspect in &aspects.0 {
            for cadence in &aspect.cadences {
                if cadence.next <= now {
                    fired.push((actor, aspect.source, cadence.seconds));
                }
            }
            if aspect.during_next.is_some_and(|next| next <= now) {
                replays.push((actor, aspect.source));
            }
        }
    }
    for (actor, source, seconds) in fired {
        advance(world, actor, source, seconds, now);
        world.resource_mut::<RiftInstances>().raise(
            source.owner(),
            EventKind::Interval,
            Some(actor),
            Vec::new(),
            move |event| matches!(event, RiftEvent::Interval { seconds: every } if *every == seconds),
        );
    }
    for (actor, source) in replays {
        play_during(world, actor, source, now);
    }
}

fn desired(world: &World, actor: Entity) -> Vec<Wanted> {
    let content = world.resource::<Content>();
    let scripts = world.resource::<RiftScripts>();
    let held = |source: AspectSource| Wanted {
        source,
        since: None,
    };
    let mut wanted = Vec::new();
    if let Some(statuses) = world.get::<Statuses>(actor) {
        wanted.extend(statuses.0.iter().map(|status| Wanted {
            source: AspectSource::Status(status.effect),
            since: Some(status.since),
        }));
    }
    if let Some(equipment) = world.get::<Equipment>(actor) {
        wanted.extend(
            equipment
                .slots
                .values()
                .map(|&item| held(AspectSource::Item(item))),
        );
    }
    if let Some(bag) = world.get::<Inventory>(actor) {
        for stack in &bag.slots {
            let carried = held(AspectSource::Item(stack.item));
            if matches!(stack.item.get(content).kind, ItemKind::Resource)
                && !wanted.contains(&carried)
            {
                wanted.push(carried);
            }
        }
    }
    for (skill, rank) in skill::known(world, actor) {
        if rank > Rank::NONE && skill.get(content).kind == SkillKind::Passive {
            wanted.push(held(AspectSource::Skill(skill)));
        }
    }
    wanted.retain(|aspect| {
        matches!(aspect.source, AspectSource::Status(_))
            || !scripts.of(aspect.source.row()).is_empty()
    });
    wanted
}

fn activate(world: &mut World, actor: Entity, Wanted { source, since }: Wanted) {
    let now = world.resource::<WallClock>().now;
    let scripts = world.resource::<RiftScripts>().clone();
    let mut cadences: Vec<Cadence> = Vec::new();
    for procedure in scripts.of(source.row()) {
        for (_, event) in procedure.entry(EventKind::Interval) {
            let &RiftEvent::Interval { seconds } = event else {
                continue;
            };
            if cadences.iter().all(|cadence| cadence.seconds != seconds) {
                cadences.push(Cadence {
                    seconds,
                    next: now.after(seconds),
                });
            }
        }
    }
    let aspect = ActiveAspect {
        source,
        since,
        cadences,
        during: None,
        during_next: None,
    };
    if let Some(mut aspects) = world.get_mut::<ActiveAspects>(actor) {
        aspects.0.push(aspect);
    }
    if let AspectSource::Status(effect) = source {
        let content = world.resource::<Content>().clone();
        let def = effect.get(&content);
        if let Some(shown) = def.vfx.on_activate {
            vfx::show(world, shown, VfxSite::Actor(actor));
        }
        if let Some(heard) = def.sfx.on_activate {
            sound(world, actor, heard);
        }
        let shown = def.name.clone();
        notification::pop_as(world, actor, shown, VfxShaderFixture::StatusPop);
        play_during(world, actor, source, now);
    }
    world.resource_mut::<RiftInstances>().raise(
        source.owner(),
        EventKind::Activated,
        Some(actor),
        Vec::new(),
        |_| true,
    );
}

fn deactivate(world: &mut World, actor: Entity, source: AspectSource) {
    let taken = world
        .get_mut::<ActiveAspects>(actor)
        .and_then(|mut aspects| {
            let index = aspects
                .0
                .iter()
                .position(|aspect| aspect.source == source)?;
            Some(aspects.0.remove(index))
        });
    let Some(aspect) = taken else {
        return;
    };
    if let Some(during) = aspect.during
        && world.get_entity(during).is_ok()
    {
        world.despawn(during);
    }
    boost::drop(world, actor, source);
    if let AspectSource::Status(effect) = source {
        let content = world.resource::<Content>().clone();
        let def = effect.get(&content);
        if let Some(shown) = def.vfx.on_expire {
            vfx::show(world, shown, VfxSite::Actor(actor));
        }
        if let Some(heard) = def.sfx.on_expire {
            sound(world, actor, heard);
        }
    }
    world.resource_mut::<RiftInstances>().raise(
        source.owner(),
        EventKind::Deactivated,
        Some(actor),
        Vec::new(),
        |_| true,
    );
}

fn play_during(world: &mut World, actor: Entity, source: AspectSource, now: UnixMillis) {
    let AspectSource::Status(effect) = source else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let def = effect.get(&content);
    let previous = world
        .get::<ActiveAspects>(actor)
        .and_then(|aspects| aspects.0.iter().find(|aspect| aspect.source == source))
        .and_then(|aspect| aspect.during);
    if let Some(previous) = previous
        && world.get_entity(previous).is_ok()
    {
        world.despawn(previous);
    }
    let shown = def.vfx.during.and_then(|shown| {
        let lasts = shown.get(&content).lasts();
        let entity = vfx::show(world, shown, VfxSite::Actor(actor))?;
        Some((entity, lasts))
    });
    if let Some(heard) = def.sfx.during {
        sound(world, actor, heard);
    }
    let next = match (shown, def.sfx.during) {
        (Some((_, lasts)), _) => Some(now.after(lasts)),
        (None, Some(_)) => Some(now.after(DURING_SOUND_PERIOD)),
        (None, None) => None,
    };
    if let Some(mut aspects) = world.get_mut::<ActiveAspects>(actor)
        && let Some(aspect) = aspects.0.iter_mut().find(|aspect| aspect.source == source)
    {
        aspect.during = shown.map(|(entity, _)| entity);
        aspect.during_next = next;
    }
}

fn advance(
    world: &mut World,
    actor: Entity,
    source: AspectSource,
    seconds: Seconds,
    now: UnixMillis,
) {
    let Some(mut aspects) = world.get_mut::<ActiveAspects>(actor) else {
        return;
    };
    let Some(aspect) = aspects.0.iter_mut().find(|aspect| aspect.source == source) else {
        return;
    };
    if let Some(cadence) = aspect
        .cadences
        .iter_mut()
        .find(|cadence| cadence.seconds == seconds)
    {
        cadence.next = now.after(seconds);
    }
}

fn sound(world: &mut World, actor: Entity, sfx: SfxId) {
    notification::play_sound(world, Some(actor), Some(actor), sfx, Audience::Everyone);
}
