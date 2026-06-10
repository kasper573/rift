use std::collections::HashMap;

use world::Entity;
use world::actors;
use world::actors::SfxId;
use world::area::Area;
use world::math::{Pos, Tiles};
use world::protocol::{Actor, Position, action_name};

use crate::render::{Animator, proximity_pan, proximity_volume};

/// Distinct one-shot cues landing within this window collapse to a single play.
const STACK_WINDOW: f32 = 0.1;

/// Turns replicated state into the one-shot sounds to play this frame — manifest sound cues and,
/// where a walk/run animation declares a "step" cue, the footstep of the tile underfoot — and
/// enforces the stacking rule: identical cues triggered close together (ten deaths sharing two
/// death sounds) sound each distinct cue once, not once per actor.
#[derive(Default)]
pub struct SfxTracker {
    seen: HashMap<Entity, (u8, f32)>,
    played: HashMap<&'static SfxId, f32>,
}

impl SfxTracker {
    pub fn new() -> SfxTracker {
        SfxTracker::default()
    }

    /// The sounds to play this frame. An actor first entering view is armed silently, so a corpse
    /// scrolling on screen never replays the death it finished off-screen.
    pub fn cues(
        &mut self,
        world: &mut world::World,
        area: Option<&'static Area>,
        animator: &mut Animator,
        listener: Pos<Tiles>,
        clock: f32,
    ) -> Vec<(&'static SfxId, f32, f32)> {
        self.seen
            .retain(|&entity, _| world.get_entity(entity).is_ok());
        // Per distinct id this frame keep the loudest source, so near and far actors sharing a
        // sound collapse to a single play at the nearer volume (and its pan).
        let mut frame: HashMap<&'static SfxId, (f32, f32)> = HashMap::new();
        let mut query = world.query::<(Entity, &Actor, &Position)>();
        for (entity, actor, position) in query.iter(world) {
            let now = animator.elapsed(entity, actor.action, clock);
            let Some((was, then)) = self.seen.insert(entity, (actor.action, now)) else {
                continue;
            };
            let source = position.pos;
            let volume = proximity_volume(listener, source);
            if volume <= 0.0 {
                continue;
            }
            let pan = proximity_pan(listener, source);
            let prev = if was == actor.action { then } else { -1.0 };
            let model = actors::model(actor.model);
            let action = action_name(actor.action);
            let rate = actor.attack_rate.0;
            let (cues, stepped) = model.cues(action, actor.dir, prev, now, rate);
            for id in cues {
                let slot = frame.entry(id).or_insert((0.0, 0.0));
                if volume > slot.0 {
                    *slot = (volume, pan);
                }
            }
            if let Some(area) = area
                && stepped
                && let Some(id) = area.tile_sfx_at(source.x.floor() as i32, source.y.floor() as i32)
            {
                let slot = frame.entry(id).or_insert((0.0, 0.0));
                if volume > slot.0 {
                    *slot = (volume, pan);
                }
            }
        }
        frame
            .into_iter()
            .filter(|&(id, _)| self.ready(id, clock))
            .map(|(id, (volume, pan))| (id, volume, pan))
            .collect()
    }

    /// Whether a discrete one-shot cue (e.g. a consumed item) may sound now under the stacking
    /// rule; records the play so same-window repeats collapse into it.
    pub fn ready(&mut self, id: &'static SfxId, clock: f32) -> bool {
        if self
            .played
            .get(id)
            .is_some_and(|&last| clock - last < STACK_WINDOW)
        {
            return false;
        }
        self.played.insert(id, clock);
        true
    }
}
