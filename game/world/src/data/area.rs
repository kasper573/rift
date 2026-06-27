use crate::systems::area::AreaDef;

seq_macro::seq!(N in 1..=256 {
    crate::table! {
        Island: AreaDef { map: "island", bench: false },
        Forest: AreaDef { map: "forest", bench: false },
        #(
            BenchArea~N: AreaDef { map: "island", bench: true },
        )*
    }
});

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
