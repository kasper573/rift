use crate::core::assets::AssetRef;
use crate::data::memory::Id as MemoryId;
use crate::data::notification::Id as NotificationId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::area::{AreaDef, MarkerName, Population, Resident, Zone};
use crate::systems::memory::{Remember, Remembers};
use crate::systems::notification::Notify;
use crate::systems::prop::Fixture;
use crate::systems::rule::Not;

crate::table! {
    Island: AreaDef {
        name: "The island",
        map: AssetRef("maps/island.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 6, roams: None },
            Population { npc: NpcId::Skeleton, count: 8, roams: None },
            Population { npc: NpcId::VampireBat, count: 5, roams: None },
            Population { npc: NpcId::Bat, count: 4, roams: None },
            Population { npc: NpcId::OrcChief, count: 2, roams: None },
        ],
        residents: &[
            Resident { npc: NpcId::Grisha, at: MarkerName("inn"), shown: &[] },
            Resident { npc: NpcId::Mara, at: MarkerName("market-stall"), shown: &[] },
            Resident { npc: NpcId::Tobb, at: MarkerName("fishing-pier"), shown: &[] },
            Resident { npc: NpcId::Bram, at: MarkerName("ferry-landing"), shown: &[] },
            Resident { npc: NpcId::Wren, at: MarkerName("bone-heap"), shown: &[] },
            Resident {
                npc: NpcId::Pell,
                at: MarkerName("dice-table"),
                shown: &[&Not(&Remembers(MemoryId::PellFighting)), &Not(&Remembers(MemoryId::PellDead))],
            },
            Resident { npc: NpcId::Ilsa, at: MarkerName("road-warden"), shown: &[] },
        ],
        props: &[
            Fixture { prop: PropId::HarbourBoard, at: MarkerName("notice-board"), shown: &[] },
            Fixture { prop: PropId::TideChest, at: MarkerName("tide-chest"), shown: &[] },
            Fixture { prop: PropId::HonestyBox, at: MarkerName("honesty-box"), shown: &[] },
        ],
        zones: &[
            Zone {
                at: MarkerName("ferry-pier"),
                with: None,
                requires: &[&Not(&Remembers(MemoryId::HeardGulls))],
                then: &[&Remember(MemoryId::HeardGulls), &Notify(NotificationId::Gulls)],
            },
        ],
        intro: None,
    },
    Forest: AreaDef {
        name: "The forest",
        map: AssetRef("maps/forest.tmx"),
        populations: &[
            Population { npc: NpcId::Orc, count: 8, roams: None },
            Population { npc: NpcId::OrcChief, count: 3, roams: None },
            Population { npc: NpcId::Skeleton, count: 5, roams: None },
            Population { npc: NpcId::Bat, count: 6, roams: None },
        ],
        residents: &[Resident { npc: NpcId::Ugra, at: MarkerName("ugra-camp"), shown: &[] }],
        props: &[Fixture { prop: PropId::StandingStone, at: MarkerName("standing-stone"), shown: &[] }],
        zones: &[],
        intro: Some(NotificationId::ForestIntro),
    },
    Frostmere: AreaDef {
        name: "Frostmere",
        map: AssetRef("maps/frostmere.tmx"),
        populations: &[
            Population { npc: NpcId::DireWolf, count: 6, roams: Some(MarkerName("wolf-woods")) },
            Population { npc: NpcId::FrostWraith, count: 4, roams: Some(MarkerName("barrow")) },
        ],
        residents: &[
            Resident { npc: NpcId::Trapper, at: MarkerName("trapper-post"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    FrostmereCabin: AreaDef {
        name: "The huntress's cabin",
        map: AssetRef("maps/frostmere-cabin.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Huntress, at: MarkerName("huntress-bench"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    FrostmereLonghouse: AreaDef {
        name: "The jarl's longhouse",
        map: AssetRef("maps/frostmere-longhouse.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Jarl, at: MarkerName("jarl-throne"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Sunscar: AreaDef {
        name: "Sunscar",
        map: AssetRef("maps/sunscar.tmx"),
        populations: &[
            Population { npc: NpcId::Scorpion, count: 5, roams: Some(MarkerName("scorpion-rocks")) },
            Population { npc: NpcId::Scorpion, count: 4, roams: Some(MarkerName("dry-wash")) },
            Population { npc: NpcId::SandMummy, count: 5, roams: Some(MarkerName("ruins")) },
        ],
        residents: &[
            Resident { npc: NpcId::NomadMerchant, at: MarkerName("merchant-stall"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    SunscarAdobe: AreaDef {
        name: "The potter's house",
        map: AssetRef("maps/sunscar-adobe.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Potter, at: MarkerName("potter-wheel"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    SunscarPavilion: AreaDef {
        name: "The silk prince's pavilion",
        map: AssetRef("maps/sunscar-pavilion.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::SilkPrince, at: MarkerName("silk-prince-seat"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Bloomvale: AreaDef {
        name: "Bloomvale",
        map: AssetRef("maps/bloomvale.tmx"),
        populations: &[
            Population { npc: NpcId::WildBoar, count: 6, roams: Some(MarkerName("boar-orchard")) },
            Population { npc: NpcId::GiantBee, count: 6, roams: Some(MarkerName("bee-meadow")) },
        ],
        residents: &[
            Resident { npc: NpcId::FarmerGirl, at: MarkerName("farmer-girl"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    BloomvaleInn: AreaDef {
        name: "The crossroads inn",
        map: AssetRef("maps/bloomvale-inn.tmx"),
        populations: &[],
        residents: &[],
        props: &[],
        zones: &[],
        intro: None,
    },
    BloomvaleCottage: AreaDef {
        name: "The baker's cottage",
        map: AssetRef("maps/bloomvale-cottage.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Baker, at: MarkerName("baker-table"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    BloomvaleMill: AreaDef {
        name: "The windmill",
        map: AssetRef("maps/bloomvale-mill.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Miller, at: MarkerName("miller-stones"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Amberwood: AreaDef {
        name: "Amberwood",
        map: AssetRef("maps/amberwood.tmx"),
        populations: &[
            Population { npc: NpcId::BrownBear, count: 4, roams: Some(MarkerName("bear-woods")) },
            Population { npc: NpcId::Goblin, count: 6, roams: Some(MarkerName("goblin-woods")) },
        ],
        residents: &[
            Resident { npc: NpcId::Lumberjack, at: MarkerName("lumber-yard"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    AmberwoodBarn: AreaDef {
        name: "The farm's barn",
        map: AssetRef("maps/amberwood-barn.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Farmhand, at: MarkerName("farmhand-stall"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    AmberwoodCabin: AreaDef {
        name: "The old woodsman's cabin",
        map: AssetRef("maps/amberwood-cabin.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::OldWoodsman, at: MarkerName("woodsman-chair"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Gloomhollow: AreaDef {
        name: "Gloomhollow",
        map: AssetRef("maps/gloomhollow.tmx"),
        populations: &[
            Population { npc: NpcId::Ghost, count: 5, roams: Some(MarkerName("haunted-streets")) },
            Population { npc: NpcId::Ghost, count: 3, roams: Some(MarkerName("churchyard")) },
            Population { npc: NpcId::Ghost, count: 3, roams: Some(MarkerName("drowned-pool")) },
            Population { npc: NpcId::Ghoul, count: 3, roams: Some(MarkerName("churchyard")) },
            Population { npc: NpcId::Ghoul, count: 4, roams: Some(MarkerName("gallows-field")) },
            Population { npc: NpcId::Ghoul, count: 5, roams: Some(MarkerName("abbey-ruins")) },
        ],
        residents: &[],
        props: &[],
        zones: &[],
        intro: None,
    },
    GloomhollowChapel: AreaDef {
        name: "The chapel",
        map: AssetRef("maps/gloomhollow-chapel.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Gravedigger, at: MarkerName("gravedigger-pew"), shown: &[] },
            Resident { npc: NpcId::Priest, at: MarkerName("priest-study"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    GloomhollowCrypt: AreaDef {
        name: "The crypt",
        map: AssetRef("maps/gloomhollow-crypt.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::CryptKeeper, at: MarkerName("crypt-keeper"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Glimmercap: AreaDef {
        name: "Glimmercap",
        map: AssetRef("maps/glimmercap.tmx"),
        populations: &[
            Population { npc: NpcId::Myconid, count: 6, roams: Some(MarkerName("deep-caps")) },
            Population { npc: NpcId::GlowMoth, count: 6, roams: Some(MarkerName("moth-glade")) },
        ],
        residents: &[
            Resident { npc: NpcId::GnomeAlchemist, at: MarkerName("alchemist-yard"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    GlimmercapMushroom: AreaDef {
        name: "The fairy's mushroom",
        map: AssetRef("maps/glimmercap-mushroom.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Fairy, at: MarkerName("fairy-table"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    GlimmercapTree: AreaDef {
        name: "The druid's tree",
        map: AssetRef("maps/glimmercap-tree.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Druid, at: MarkerName("druid-table"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Sakura: AreaDef {
        name: "Sakura",
        map: AssetRef("maps/sakura.tmx"),
        populations: &[
            Population { npc: NpcId::Kitsune, count: 6, roams: Some(MarkerName("fox-wood")) },
            Population { npc: NpcId::Ronin, count: 4, roams: Some(MarkerName("bamboo-grove")) },
            Population { npc: NpcId::Ronin, count: 2, roams: Some(MarkerName("lords-cairn")) },
        ],
        residents: &[
            Resident { npc: NpcId::Monk, at: MarkerName("monk-vigil"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    SakuraEstate: AreaDef {
        name: "The lord's estate",
        map: AssetRef("maps/sakura-estate.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::SamuraiLord, at: MarkerName("lord-seat"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    SakuraTeahouse: AreaDef {
        name: "The teahouse",
        map: AssetRef("maps/sakura-teahouse.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::TeaMaster, at: MarkerName("tea-hearth"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    SakuraPagoda: AreaDef {
        name: "The temple",
        map: AssetRef("maps/sakura-pagoda.tmx"),
        populations: &[],
        residents: &[],
        props: &[],
        zones: &[],
        intro: None,
    },
    Mirefen: AreaDef {
        name: "Mirefen",
        map: AssetRef("maps/mirefen.tmx"),
        populations: &[
            Population { npc: NpcId::BogLurker, count: 6, roams: Some(MarkerName("lurker-fen")) },
            Population { npc: NpcId::SwampSlime, count: 8, roams: Some(MarkerName("slime-bog")) },
        ],
        residents: &[
            Resident { npc: NpcId::EelFisher, at: MarkerName("eel-landing"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    MirefenStilt: AreaDef {
        name: "The fisher's stilt house",
        map: AssetRef("maps/mirefen-stilt.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::FisherBoy, at: MarkerName("net-frame"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    MirefenWitch: AreaDef {
        name: "The witch's hut",
        map: AssetRef("maps/mirefen-witch.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::SwampWitch, at: MarkerName("cauldron"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Verdant: AreaDef {
        name: "Verdant",
        map: AssetRef("maps/verdant.tmx"),
        populations: &[
            Population { npc: NpcId::Panther, count: 5, roams: Some(MarkerName("panther-jungle")) },
            Population { npc: NpcId::Lizardman, count: 6, roams: Some(MarkerName("lizard-ruins")) },
        ],
        residents: &[
            Resident { npc: NpcId::Explorer, at: MarkerName("explorer-camp"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    VerdantHut: AreaDef {
        name: "The shaman's hut",
        map: AssetRef("maps/verdant-hut.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Shaman, at: MarkerName("shaman-mat"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    VerdantTemple: AreaDef {
        name: "The vine temple",
        map: AssetRef("maps/verdant-temple.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Archaeologist, at: MarkerName("archaeologist-table"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    Emberfall: AreaDef {
        name: "Emberfall",
        map: AssetRef("maps/emberfall.tmx"),
        populations: &[
            Population { npc: NpcId::LavaGolem, count: 4, roams: Some(MarkerName("mine-yard")) },
            Population { npc: NpcId::MagmaImp, count: 7, roams: Some(MarkerName("lava-fields")) },
        ],
        residents: &[
            Resident { npc: NpcId::DwarfMiner, at: MarkerName("mine-barricade"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    EmberfallForge: AreaDef {
        name: "The forge",
        map: AssetRef("maps/emberfall-forge.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::Blacksmith, at: MarkerName("smith-anvil"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
    EmberfallShrine: AreaDef {
        name: "The fire shrine",
        map: AssetRef("maps/emberfall-shrine.tmx"),
        populations: &[],
        residents: &[
            Resident { npc: NpcId::FirePriestess, at: MarkerName("fire-altar"), shown: &[] },
        ],
        props: &[],
        zones: &[],
        intro: None,
    },
}

pub const BENCH_ID: Id = Id::Island;

/// The area new players spawn into.
pub const SPAWN_ID: Id = Id::Island;
