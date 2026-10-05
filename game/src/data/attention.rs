use crate::core::assets::AssetRef;
use crate::systems::attention::AttentionDef;

crate::table! {
    QuestReady: AttentionDef {
        icon: AssetRef("icons/attention/quest_ready.png"),
    },
    QuestOffered: AttentionDef {
        icon: AssetRef("icons/attention/quest_offered.png"),
    },
    RepeatableReady: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_ready.png"),
    },
    RepeatableOffered: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_offered.png"),
    },
    QuestInProgress: AttentionDef {
        icon: AssetRef("icons/attention/quest_in_progress.png"),
    },
    QuestLocked: AttentionDef {
        icon: AssetRef("icons/attention/quest_locked.png"),
    },
    Merchant: AttentionDef {
        icon: AssetRef("icons/attention/merchant.png"),
    },
    Collector: AttentionDef {
        icon: AssetRef("icons/attention/collector.png"),
    },
    Innkeeper: AttentionDef {
        icon: AssetRef("icons/attention/innkeeper.png"),
    },
    Travel: AttentionDef {
        icon: AssetRef("icons/attention/travel.png"),
    },
    Chance: AttentionDef {
        icon: AssetRef("icons/attention/chance.png"),
    },
    News: AttentionDef {
        icon: AssetRef("icons/attention/news.png"),
    },
    Talking: AttentionDef {
        icon: AssetRef("icons/attention/talking.png"),
    },
}
