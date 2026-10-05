use crate::core::time::Seconds;
use crate::systems::memory::{MemoryDef, MemoryKind, Resets};

crate::table! {
    TobbVisits: MemoryDef {
        label: "Visits to Tobb",
        kind: MemoryKind::Counter { step_every: Seconds(600.0) },
        resets: Resets::Never,
    },
    TobbGaveLure: MemoryDef {
        label: "Tobb's lucky lure",
        kind: MemoryKind::Flag,
        resets: Resets::Never,
    },
    TobbNewsToday: MemoryDef {
        label: "Heard Tobb's news today",
        kind: MemoryKind::Flag,
        resets: Resets::Daily,
    },
    TideChestLooted: MemoryDef {
        label: "Emptied the tide chest today",
        kind: MemoryKind::Flag,
        resets: Resets::Daily,
    },
    InnFavour: MemoryDef {
        label: "Favour at the inn",
        kind: MemoryKind::Counter { step_every: Seconds(300.0) },
        resets: Resets::Never,
    },
    PellGrudge: MemoryDef {
        label: "Pell's grudge",
        kind: MemoryKind::Timer(Seconds(1_800.0)),
        resets: Resets::Never,
    },
    PellFighting: MemoryDef {
        label: "Fighting Pell",
        kind: MemoryKind::Flag,
        resets: Resets::OnLeavingArea,
    },
    PellDead: MemoryDef {
        label: "Pell defeated",
        kind: MemoryKind::Timer(Seconds(1_800.0)),
        resets: Resets::Never,
    },
    BribedIlsa: MemoryDef {
        label: "Bribed Ilsa",
        kind: MemoryKind::Flag,
        resets: Resets::Never,
    },
    SidedWithOrcs: MemoryDef {
        label: "Sided with the orcs",
        kind: MemoryKind::Flag,
        resets: Resets::Never,
    },
    FoundTatteredMap: MemoryDef {
        label: "Found the tattered map",
        kind: MemoryKind::Flag,
        resets: Resets::Never,
    },
    SpurnedUgra: MemoryDef {
        label: "Turned Ugra away",
        kind: MemoryKind::Flag,
        resets: Resets::Never,
    },
    HeardHarbourBell: MemoryDef {
        label: "Heard the harbour bell",
        kind: MemoryKind::Timer(Seconds(600.0)),
        resets: Resets::Never,
    },
    ChiefTauntedYou: MemoryDef {
        label: "Taunted by an Orc Chief",
        kind: MemoryKind::Timer(Seconds(120.0)),
        resets: Resets::Never,
    },
}
