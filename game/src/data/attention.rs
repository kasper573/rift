use crate::core::assets::AssetRef;
use crate::systems::attention::AttentionDef;

crate::table! {
    QuestReady: AttentionDef {
        icon: AssetRef("icons/attention/quest_ready.png"),
        label: "Quest ready to hand in",
    },
    QuestOffered: AttentionDef {
        icon: AssetRef("icons/attention/quest_offered.png"),
        label: "Quest for you",
    },
    RepeatableReady: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_ready.png"),
        label: "Repeatable quest ready",
    },
    RepeatableOffered: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_offered.png"),
        label: "Repeatable quest available",
    },
    QuestInProgress: AttentionDef {
        icon: AssetRef("icons/attention/quest_in_progress.png"),
        label: "Quest in progress",
    },
    QuestLocked: AttentionDef {
        icon: AssetRef("icons/attention/quest_locked.png"),
        label: "Quest you can't take yet",
    },
    Merchant: AttentionDef {
        icon: AssetRef("icons/attention/merchant.png"),
        label: "Merchant",
    },
    Collector: AttentionDef {
        icon: AssetRef("icons/attention/collector.png"),
        label: "Collector",
    },
    Innkeeper: AttentionDef {
        icon: AssetRef("icons/attention/innkeeper.png"),
        label: "Innkeeper",
    },
    Travel: AttentionDef {
        icon: AssetRef("icons/attention/travel.png"),
        label: "Travel",
    },
    Chance: AttentionDef {
        icon: AssetRef("icons/attention/chance.png"),
        label: "Games of chance",
    },
    News: AttentionDef {
        icon: AssetRef("icons/attention/news.png"),
        label: "Something new to say",
    },
    Talking: AttentionDef {
        icon: AssetRef("icons/attention/talking.png"),
        label: "In a conversation",
    },
}
