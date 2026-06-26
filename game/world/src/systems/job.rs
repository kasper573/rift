//! Jobs: the [`JobDef`] catalog and the [`Job`] a player carries. Each job is a ladder of levels,
//! every level an exp threshold plus the effects reaching it grants. A player always has a job; for
//! now jobs are just id/name/levels, but skills will later reference them.

use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::table::{self, Content, Id};
use crate::systems::effect::{self, EffectCommand};
use crate::systems::player::Xp;

const FILE: &str = "job_table.json";

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Job>();
    effect::source(app, level_effects);
}

/// Effect source: every effect from the job levels the actor's [`Xp`] has reached.
fn level_effects(world: &World, entity: Entity) -> Vec<EffectCommand> {
    let Some(job) = world.get::<Job>(entity) else {
        return Vec::new();
    };
    let level = level(world, entity) as usize;
    job.def
        .get()
        .levels
        .iter()
        .take(level)
        .flat_map(|tier| tier.effects.iter().cloned())
        .collect()
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Job {
    pub def: Id<JobDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobDef {
    pub id: String,
    pub name: String,
    pub levels: Vec<JobLevel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobLevel {
    pub exp: u32,
    #[serde(default, deserialize_with = "crate::systems::effect::commands")]
    pub effects: Vec<EffectCommand>,
}

impl Content for JobDef {
    fn table() -> &'static [JobDef] {
        defs()
    }
    fn id(&self) -> &str {
        &self.id
    }
}

pub fn defs() -> &'static [JobDef] {
    static DEFS: OnceLock<Vec<JobDef>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let defs: Vec<JobDef> = table::load(FILE);
        table::unique_ids(defs.iter().map(|def| def.id.as_str()), FILE);
        assert!(!defs.is_empty(), "{FILE}: at least one job is required");
        defs
    })
}

/// The job every fresh player starts in: the first row of the table.
pub fn default_job() -> Id<JobDef> {
    defs();
    Id::new(0)
}

/// An actor's level: how many of its job's level tiers its [`Xp`] has reached. Players carry a job;
/// anything else (npcs) is level 0.
pub fn level(world: &World, entity: Entity) -> u32 {
    let Some(job) = world.get::<Job>(entity) else {
        return 0;
    };
    let xp = world.get::<Xp>(entity).map_or(0, |xp| xp.amount);
    job.def
        .get()
        .levels
        .iter()
        .filter(|tier| tier.exp <= xp)
        .count() as u32
}
