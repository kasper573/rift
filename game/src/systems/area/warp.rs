use bevy_app::App;
use bevy_ecs::prelude::*;

use super::{Id, MarkerName, Portal};
use crate::core::assets::AssetService;
use crate::systems::rule::{self, Requirement};

#[derive(Clone, Copy)]
pub struct WarpLock {
    pub area: Id,
    pub warp: MarkerName,
    pub requires: &'static [&'static dyn Requirement],
}

#[derive(Resource, Default)]
pub struct WarpLocks(Vec<WarpLock>);

pub fn lock_warps(app: &mut App, locks: impl IntoIterator<Item = WarpLock>) {
    app.world_mut().resource_mut::<WarpLocks>().0.extend(locks);
}

pub fn passable(world: &World, traveller: Entity, area: Id, portal: &Portal) -> bool {
    world
        .resource::<WarpLocks>()
        .0
        .iter()
        .filter(|lock| lock.area == area && lock.warp.0 == portal.name)
        .all(|lock| rule::met(world, traveller, lock.requires))
}

impl WarpLock {
    pub fn check(&self, assets: &AssetService) {
        let map = super::load(assets, self.area);
        if !map.portals.iter().any(|portal| portal.name == self.warp.0) {
            panic!(
                "area {:?}: a lock guards warp '{}', which the map lacks",
                self.area, self.warp.0
            );
        }
    }
}
