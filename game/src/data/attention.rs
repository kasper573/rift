use crate::core::assets::AssetRef;
use crate::systems::attention::AttentionDef;

crate::table! {
    QuestReady: AttentionDef {
        icon: AssetRef("icons/attention/quest_ready.png"),
        label: "Quest ready to hand in",
        on_plate: true,
    },
    QuestOffered: AttentionDef {
        icon: AssetRef("icons/attention/quest_offered.png"),
        label: "Quest for you",
        on_plate: true,
    },
    RepeatableReady: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_ready.png"),
        label: "Repeatable quest ready",
        on_plate: true,
    },
    RepeatableOffered: AttentionDef {
        icon: AssetRef("icons/attention/repeatable_offered.png"),
        label: "Repeatable quest available",
        on_plate: true,
    },
    QuestInProgress: AttentionDef {
        icon: AssetRef("icons/attention/quest_in_progress.png"),
        label: "Quest in progress",
        on_plate: true,
    },
    QuestLocked: AttentionDef {
        icon: AssetRef("icons/attention/quest_locked.png"),
        label: "Quest you can't take yet",
        on_plate: true,
    },
    Merchant: AttentionDef {
        icon: AssetRef("icons/attention/merchant.png"),
        label: "Merchant",
        on_plate: true,
    },
    Collector: AttentionDef {
        icon: AssetRef("icons/attention/collector.png"),
        label: "Collector",
        on_plate: true,
    },
    Innkeeper: AttentionDef {
        icon: AssetRef("icons/attention/innkeeper.png"),
        label: "Innkeeper",
        on_plate: true,
    },
    Travel: AttentionDef {
        icon: AssetRef("icons/attention/travel.png"),
        label: "Travel",
        on_plate: true,
    },
    Chance: AttentionDef {
        icon: AssetRef("icons/attention/chance.png"),
        label: "Games of chance",
        on_plate: true,
    },
    News: AttentionDef {
        icon: AssetRef("icons/attention/news.png"),
        label: "Something new to say",
        on_plate: true,
    },
    Talk: AttentionDef {
        icon: AssetRef("icons/attention/talk.png"),
        label: "Talk",
        on_plate: false,
    },
    Talking: AttentionDef {
        icon: AssetRef("icons/attention/talking.png"),
        label: "In a conversation",
        on_plate: true,
    },
}
