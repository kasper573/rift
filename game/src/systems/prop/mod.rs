pub mod render;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::Replicated;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::tiling::{TilePos, Tiles};
use crate::data;
use crate::systems::actor::{Hitbox, Name};
use crate::systems::area::{self, AreaTag, MapMarker, MarkerName};
use crate::systems::interact::{self, Interaction, Interactive};
use crate::systems::movement::Position;
use crate::systems::rule::Requirement;
use crate::systems::visibility::Presence;

pub use crate::data::prop::Id as PropId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Prop>();
    interact::interaction_source(app, interaction);
}

pub struct PropDef {
    pub display_name: &'static str,
    pub look: Option<AssetRef>,
    pub interaction: Option<Interaction>,
}

pub struct Fixture {
    pub prop: PropId,
    pub at: MarkerName,
    pub shown: &'static [&'static dyn Requirement],
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Prop {
    pub def: PropId,
}

pub fn spawn_all(world: &mut World) {
    let area_id = world.resource::<crate::systems::WorldArea>().0;
    let def = area_id.get();
    let area = world
        .resource::<AssetService>()
        .resolve(def.map, area::build_area);
    for fixture in def.props {
        let marker = area
            .marker(fixture.at)
            .expect("validated at startup: fixtures stand on markers");
        let footprint = footprint(marker);
        let prop = fixture.prop.get();
        let entity = world
            .spawn((
                Replicated,
                Position {
                    pos: Pos::new(footprint.center().x, footprint.max().y - 0.5),
                },
                Hitbox {
                    size: footprint.size,
                },
                AreaTag { area: area_id },
                Prop { def: fixture.prop },
                Name {
                    name: prop.display_name.to_owned(),
                },
            ))
            .id();
        if prop.interaction.is_some() {
            world.entity_mut(entity).insert(Interactive);
        }
        if !fixture.shown.is_empty() {
            world
                .entity_mut(entity)
                .insert(Presence::When(fixture.shown));
        }
    }
}

pub fn conversation_starts() -> Vec<data::dialogue::Id> {
    data::prop::TABLE
        .iter()
        .flat_map(|def| {
            def.interaction
                .iter()
                .flat_map(interact::conversation_starts)
        })
        .collect()
}

fn interaction(world: &World, entity: Entity) -> Option<&'static Interaction> {
    world.get::<Prop>(entity)?.def.get().interaction.as_ref()
}

fn footprint(marker: MapMarker) -> Rect<Tiles> {
    match marker {
        MapMarker::Rect(rect) => rect,
        MapMarker::Point(at) => Rect::new(at.snap() + Offset::new(-0.5, -0.5), Size::new(1.0, 1.0)),
    }
}
