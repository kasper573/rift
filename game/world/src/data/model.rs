use crate::core::assets::AssetRef;
use crate::systems::actor::{ActorModel, load};

crate::table! {
    ActorModel {
        Adventurer: load(AssetRef("models/adventurer.tsx")),
        Bat: load(AssetRef("models/bat.tsx")),
        Orc: load(AssetRef("models/orc.tsx")),
        Skeleton: load(AssetRef("models/skeleton.tsx")),
    }
}
