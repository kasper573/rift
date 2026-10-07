use crate::core::math::Percent;
use crate::core::time::Millis;
use crate::data::area::Id as AreaId;
use crate::data::expression::Id as ExpressionId;
use crate::data::item::Id as ItemId;
use crate::data::memory::Id as MemoryId;
use crate::data::notification::Id as NotificationId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::actor::bust::Face::{Generic, Individual};
use crate::systems::actor::bust::GenericExpression::{
    Angry, Happy, Neutral, Sad, Surprised, Thinking,
};
use crate::systems::area::{MarkerName, Travel};
use crate::systems::dialogue::ChoiceReveal::{Costs, Locked, Needs};
use crate::systems::dialogue::Speaker::{Narrator, Npc, Prop};
use crate::systems::dialogue::{Choice, DialogueNode, Gamble, GotoNode, Line};
use crate::systems::equipment::Wearing;
use crate::systems::item::{GiveItems, Holding, ItemStack};
use crate::systems::job::MinLevel;
use crate::systems::memory::{Remember, Remembers, RemembersAtLeast};
use crate::systems::notification::Notify;
use crate::systems::npc::{ShownFor, SpawnNear, SpawnNpcs};
use crate::systems::quest::{
    AcceptQuest, FailQuest, OnQuest, QuestId, QuestOnCooldown, QuestResetsIn, TurnIn,
};
use crate::systems::rule::Not;
use crate::systems::rule::Outcome;
use crate::systems::stat::Heal;
use crate::systems::text::{Fx, Ink, Motion, Voice, fill, plain, styled};

const DICE: Gamble = Gamble {
    odds: Percent(45.0),
    won: &[
        &GiveItems(&[ItemStack::new(ItemId::Gold, 20)]),
        &GotoNode(Id::PellLoses),
    ],
    lost: &[&GotoNode(Id::PellWins)],
};

const SAIL: &[&dyn Outcome] = &[
    &Notify(NotificationId::GullSails),
    &Travel {
        to: AreaId::Forest,
        at: MarkerName("ferry-dock"),
    },
];

const PASS: GiveItems = GiveItems(&[ItemStack::new(ItemId::RoadPass, 1)]);

crate::table! {
    TobbHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), text: &[plain("Ho there! Name's Tobb. Been fishing this harbour since I could hold a rod.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), text: &[plain("If it swims, I've caught it. If it talks, I've heard it.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[
            Choice { label: &[plain("Caught anything good today?")], then: &[&GotoNode(Id::TobbCatch)], ..Choice::SAY },
            Choice { label: &[plain("See you around, Tobb.")], ..Choice::SAY },
        ],
    },
    TobbCatch: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), text: &[plain("A boot, two crabs and a very rude gull.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Individual(ExpressionId::TobbLaughing)), text: &[plain("Best day all week!")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Good luck out there.")], ..Choice::SAY }],
    },
    TobbAgain: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), text: &[plain("Didn't I just see you? The fish won't catch themselves, you know.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Sorry, Tobb.")], ..Choice::SAY }],
    },
    TobbBack: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), text: &[plain("Back again! Pull up a crate.")] }],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[
            Choice { label: &[plain("Caught anything good today?")], then: &[&GotoNode(Id::TobbCatch)], ..Choice::SAY },
            Choice { label: &[plain("See you around, Tobb.")], ..Choice::SAY },
        ],
    },
    TobbRegular: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), text: &[plain("You've got patience, I'll give you that. Most folk don't come back to hear about crabs.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), text: &[plain("Keep it up and I might just have something for you.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[Choice { label: &[plain("I'll hold you to that.")], ..Choice::SAY }],
    },
    TobbGift: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), text: &[plain("Here. My "), styled("lucky lure", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), text: &[plain("Don't lose it. It's caught more fish than I have.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[
            Choice {
                label: &[plain("Thank you, Tobb.")],
                then: &[&GiveItems(&[ItemStack::new(ItemId::LuckyLure, 1)]), &Remember(MemoryId::TobbGaveLure)],
                ..Choice::SAY
            },
            Choice { label: &[plain("Maybe later.")], ..Choice::SAY },
        ],
    },
    TobbGossip: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), text: &[styled("Pell", &[Fx::Ink(Ink::Name)]), plain(" loads his dice, you know. "), styled("Everyone says so.", &[Fx::Voice(Voice::Whisper)])] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Sad)), text: &[plain("Nobody can prove it, mind.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[Choice { label: &[plain("I'll keep an eye on him.")], ..Choice::SAY }],
    },
    TobbNews: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), text: &[plain("Oi! Over here! You'll want to hear this.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), text: &[styled("Orcs", &[Fx::Ink(Ink::Danger)]), plain(" were seen on the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" at dawn. Big ones, with a chief in a bone helmet.")] },
        ],
        enter: &[&Remember(MemoryId::TobbNewsToday)],
        topics: false,
        choices: &[Choice { label: &[plain("Thanks for the warning. I'll keep my eyes open.")], ..Choice::SAY }],
    },
    GrishaHello: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), text: &[plain("Welcome to the "), styled("Driftwood Inn", &[Fx::Ink(Ink::Place)]), plain("! A bed, a bowl, and all the gossip you can stomach.")] }],
        enter: &[],
        topics: true,
        choices: GRISHA_CHOICES,
    },
    GrishaRegular: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), text: &[plain("My favourite guest! Rest your bones. For you, three "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(".")] }],
        enter: &[],
        topics: true,
        choices: GRISHA_CHOICES,
    },
    GrishaRested: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), text: &[plain("There. Slept like a log, you did.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Good as new. Thanks, Grisha.")], ..Choice::SAY }],
    },
    GrishaRound: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("The room cheers. Someone starts a song about a mermaid and a very confused sailor.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), text: &[plain("Keep that up and you'll be a regular in no time.")] },
        ],
        enter: &[],
        topics: false,
        choices: GRISHA_CHOICES,
    },
    GrishaAnythingElse: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), text: &[plain("Suit yourself. Anything else?")] }],
        enter: &[],
        topics: true,
        choices: GRISHA_CHOICES,
    },
    GrishaChat: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), text: &[plain("Ask away. Gossip's free, the ale isn't.")] }],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Tell me about this island.")], then: &[&GotoNode(Id::GrishaIsland)], ..Choice::SAY },
            Choice {
                label: &[plain("Any rumours?")],
                requires: &[
                    &Not(&RemembersAtLeast(MemoryId::InnFavour, 1)),
                    &Not(&OnQuest(QuestId::TusksForTheChief)),
                ],
                then: &[&GotoNode(Id::GrishaRumourWren)],
                ..Choice::SAY
            },
            Choice {
                label: &[plain("Any rumours?")],
                requires: &[
                    &RemembersAtLeast(MemoryId::InnFavour, 1),
                    &Not(&OnQuest(QuestId::TusksForTheChief)),
                ],
                then: &[&GotoNode(Id::GrishaRumourBram)],
                ..Choice::SAY
            },
            Choice {
                label: &[plain("Any rumours?")],
                requires: &[&OnQuest(QuestId::TusksForTheChief)],
                then: &[&GotoNode(Id::GrishaRumourChief)],
                ..Choice::SAY
            },
            Choice { label: &[plain("Never mind.")], then: &[&GotoNode(Id::GrishaAnythingElse)], ..Choice::SAY },
        ],
    },
    GrishaIsland: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("Grisha polishes a mug that will never be clean.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), text: &[plain("This rock? Fishers, a market, my "), styled("inn", &[Fx::Ink(Ink::Place)]), plain(", and "), styled("Bram", &[Fx::Ink(Ink::Name)]), plain("'s ferry to the "), styled("forest shore", &[Fx::Ink(Ink::Place)]), plain(".")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), text: &[plain("Past the town it's sand and "), styled("skeletons", &[Fx::Ink(Ink::Danger)]), plain(", and "), styled("orcs", &[Fx::Ink(Ink::Danger)]), plain(" up the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(". "), styled("Ilsa", &[Fx::Ink(Ink::Name)]), plain(" minds the "), styled("forest road", &[Fx::Ink(Ink::Place)]), plain(".")] },
        ],
        enter: &[],
        topics: false,
        choices: GRISHA_CHAT_DONE,
    },
    GrishaRumourWren: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), text: &[styled("Wren", &[Fx::Ink(Ink::Name)]), plain(" pays good tokens for bones. Don't ask what she does with them.")] }],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("What does she do with them?")], then: &[&GotoNode(Id::GrishaWrenSecret)], ..Choice::SAY },
            Choice { label: &[plain("Ok.")], then: &[&GotoNode(Id::GrishaChat)], ..Choice::SAY },
        ],
    },
    GrishaWrenSecret: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Angry)), text: &[plain("What did I "), styled("just", &[Fx::Voice(Voice::Shout)]), plain(" say?")] },
            Line { by: Narrator, face: None, text: &[plain("She leans over the bar anyway.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), text: &[styled("Soup, or charms. Depends who's asking, and who's buying.", &[Fx::Voice(Voice::Whisper)])] },
        ],
        enter: &[],
        topics: false,
        choices: GRISHA_CHAT_DONE,
    },
    GrishaRumourBram: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Sad)), text: &[plain("Between you and me? "), styled("Bram", &[Fx::Ink(Ink::Name)]), plain("'s ferry hasn't left since the orcs came. He's waiting on a brave fool.")] }],
        enter: &[],
        topics: false,
        choices: GRISHA_CHAT_DONE,
    },
    MaraHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Happy)), text: &[plain("Welcome, traveller! Fresh crates off the boat this morning.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(ExpressionId::MaraSmug)), text: &[plain("And if you've got a strong arm, I might have a job that pays.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[
            Choice { label: &[plain("What happened to the docks?")], then: &[&GotoNode(Id::MaraDocks)], ..Choice::SAY },
            Choice { label: &[plain("Goodbye.")], ..Choice::SAY },
        ],
    },
    MaraDocks: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), text: &[styled("Orcs", &[Fx::Ink(Ink::Danger)]), plain(". They come down from the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" at night and take whatever isn't nailed down.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(ExpressionId::MaraCounting)), text: &[plain("Do you know what a crate of silk costs? I do. "), styled("To the copper.", &[Fx::Slow])] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    BramHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Neutral)), text: &[plain("Captain Bram, of the "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Thinking)), text: &[plain("She sails for the "), styled("forest shore", &[Fx::Ink(Ink::Place)]), plain(" at the bell. Twenty "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(", or show me a pass.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[
            Choice { label: &[plain("Sail to the forest.")], costs: &[ItemStack::new(ItemId::Gold, 20)], then: SAIL, reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("Ilsa gave me this pass.")], requires: &[&Holding(ItemStack::new(ItemId::RoadPass, 1))], then: SAIL, reveal: &[Needs], ..Choice::SAY },
            Choice { label: &[plain("Goodbye.")], ..Choice::SAY },
        ],
    },
    MaraShopping: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Mara), face: Some(Individual(ExpressionId::MaraCounting)), text: &[plain("Take your time. Everything's priced fair. "), styled("Mostly.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("That's all, thanks.")], ..Choice::SAY }],
    },
    WrenShopping: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Wren), face: Some(Individual(ExpressionId::WrenSleepy)), text: &[plain("On the counter. I'll count.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("That's all.")], ..Choice::SAY }],
    },
    UgraShopping: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), text: &[plain("Roots and bitter water. They mend what the forest breaks.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Farewell, Ugra.")], ..Choice::SAY }],
    },
    HonestyBoxShopping: DialogueNode {
        lines: &[Line { by: Narrator, face: None, text: &[plain("Fish steaks wrapped in leaves, and a slot for coins. A note in "), styled("Tobb", &[Fx::Ink(Ink::Name)]), plain("'s hand: "), styled("pay what's written.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Step away.")], ..Choice::SAY }],
    },
    WrenHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Wren), face: Some(Individual(ExpressionId::WrenSleepy)), text: &[plain("Bones. Wings. Nothing else.")] },
            Line { by: Npc(NpcId::Wren), face: Some(Generic(Thinking)), text: &[plain("If you find any, you know where I am.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[
            Choice { label: &[plain("When can I bring more bones?")], requires: &[&QuestOnCooldown(QuestId::BoneTithe)], then: &[&GotoNode(Id::TitheTomorrow)], ..Choice::SAY },
            Choice { label: &[plain("Goodbye.")], ..Choice::SAY },
        ],
    },
    TitheTomorrow: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Wren), face: Some(Individual(ExpressionId::WrenSleepy)), text: &[plain("The dead are down for today. Come back in "), fill(&QuestResetsIn(QuestId::BoneTithe)), plain(".")] },
            Line { by: Npc(NpcId::Wren), face: Some(Generic(Thinking)), text: &[plain("Ten "), styled("Bones", &[Fx::Ink(Ink::Item)]), plain(". Not nine.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("I'll be back.")], ..Choice::SAY }],
    },
    PellHello: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Pell), face: Some(Individual(ExpressionId::PellSmirk)), text: &[plain("Care for a "), styled("little game", &[Fx::Motion(Motion::Wave)]), plain("? Ten "), styled("Gold", &[Fx::Ink(Ink::Item)]), styled(".", &[Fx::PauseAfter(Millis(600.0))]), styled(" I never cheat.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: true,
        choices: &[
            Choice { label: &[plain("Roll the dice.")], costs: &[ItemStack::new(ItemId::Gold, 10)], then: &[&DICE], reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("Not today.")], ..Choice::SAY },
        ],
    },
    PellWins: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Pell), face: Some(Generic(Happy)), text: &[plain("The house wins! The dice are "), styled("fickle friends", &[Fx::Motion(Motion::Wave)]), plain(", eh?")] }],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Roll again.")], costs: &[ItemStack::new(ItemId::Gold, 10)], then: &[&DICE], reveal: &[Costs], ..Choice::SAY },
            Choice {
                label: &[plain("You're cheating.")],
                then: &[
                    &Remember(MemoryId::PellGrudge),
                    &Notify(NotificationId::PellCallsGuards),
                    &SpawnNpcs { npc: NpcId::HarbourGuard, count: 2, near: SpawnNear::Speaker, shown: ShownFor::You },
                ],
                ..Choice::SAY
            },
            Choice { label: &[plain("Leave.")], ..Choice::SAY },
        ],
    },
    PellLoses: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Pell), face: Some(Generic(Surprised)), text: &[plain("Bah! Beginner's luck. Twenty "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(", as promised.")] }],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Roll again.")], costs: &[ItemStack::new(ItemId::Gold, 10)], then: &[&DICE], reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("I'll quit while I'm ahead.")], ..Choice::SAY },
        ],
    },
    PellGrudging: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Pell), face: Some(Generic(Angry)), text: &[plain("You've got "), styled("some nerve", &[Fx::Voice(Voice::Shout)]), plain(", showing your face at my table.")] }],
        enter: &[],
        topics: true,
        choices: &[
            Choice {
                label: &[plain("Then let's settle this.")],
                then: &[
                    &Remember(MemoryId::PellFighting),
                    &Notify(NotificationId::PellDrawsKnife),
                    &SpawnNpcs { npc: NpcId::PellHostile, count: 1, near: SpawnNear::Speaker, shown: ShownFor::You },
                ],
                ..Choice::SAY
            },
            Choice { label: &[plain("I'm leaving.")], ..Choice::SAY },
        ],
    },
    IlsaHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Angry)), text: &[plain("The forest road is "), styled("closed", &[Fx::Ink(Ink::Danger)]), plain(". Orders from the harbour master.")] },
            Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Thinking)), text: &[plain("Seasoned fighters excepted. Everyone else turns back.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[
            Choice { label: &[plain("I'm ready for the forest road.")], requires: &[&MinLevel(3)], then: &[&PASS, &GotoNode(Id::IlsaLetsYouPass)], reveal: &[Locked, Needs], ..Choice::SAY },
            Choice { label: &[plain("The Orc Chief won't trouble anyone now.")], requires: &[&Wearing(ItemId::TribalHelmet)], then: &[&PASS, &GotoNode(Id::IlsaLetsYouPass)], ..Choice::SAY },
            Choice { label: &[plain("Here, for your trouble.")], costs: &[ItemStack::new(ItemId::Gold, 20)], then: &[&PASS, &Remember(MemoryId::BribedIlsa), &GotoNode(Id::IlsaBribed)], reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("Would a fish change your mind?")], costs: &[ItemStack::new(ItemId::FishSteak, 1)], then: &[&PASS, &GotoNode(Id::IlsaFish)], reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("Ugra's clan walks with me.")], requires: &[&Remembers(MemoryId::SidedWithOrcs)], then: &[&PASS, &GotoNode(Id::IlsaUneasy)], ..Choice::SAY },
            Choice { label: &[plain("Never mind.")], ..Choice::SAY },
        ],
    },
    IlsaHalt: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Angry)), text: &[styled("Halt!", &[Fx::Voice(Voice::Shout), Fx::Motion(Motion::Shake)]), plain(" Nobody takes the "), styled("forest road", &[Fx::Ink(Ink::Place)]), plain(" without a pass.")] },
            Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Thinking)), text: &[plain("Unless you're seasoned enough to survive what's out there.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I have a Road Pass.")], requires: &[&Holding(ItemStack::new(ItemId::RoadPass, 1))], then: &[&GotoNode(Id::IlsaLetsYouPass)], reveal: &[Needs], ..Choice::SAY },
            Choice { label: &[plain("I'm ready for the forest road.")], requires: &[&MinLevel(3)], then: &[&PASS, &GotoNode(Id::IlsaLetsYouPass)], reveal: &[Locked, Needs], ..Choice::SAY },
            Choice { label: &[plain("Here, for your trouble.")], costs: &[ItemStack::new(ItemId::Gold, 20)], then: &[&PASS, &Remember(MemoryId::BribedIlsa), &GotoNode(Id::IlsaBribed)], reveal: &[Costs], ..Choice::SAY },
            Choice { label: &[plain("I'll turn back.")], ..Choice::SAY },
        ],
    },
    IlsaLetsYouPass: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Neutral)), text: &[plain("Fine. Keep that "), styled("pass", &[Fx::Ink(Ink::Item)]), plain(" on you, and show it to "), styled("Bram", &[Fx::Ink(Ink::Name)]), plain(" if you're bound for the shore.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Thank you.")], ..Choice::SAY }],
    },
    IlsaBribed: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Happy)), text: &[plain("Pleasure doing business. Here's your "), styled("pass", &[Fx::Ink(Ink::Item)]), plain(". "), styled("I never saw a coin.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    IlsaFish: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Surprised)), text: &[plain("Is that "), styled("smoked", &[Fx::Slow]), plain("? Fine. Take the "), styled("pass", &[Fx::Ink(Ink::Item)]), plain(", and not a word to the harbour master.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Enjoy it.")], ..Choice::SAY }],
    },
    IlsaUneasy: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Thinking)), text: &[plain("The "), styled("orcs", &[Fx::Ink(Ink::Danger)]), plain(" let you walk through their camp? Then they won't touch you on the road. Take the "), styled("pass", &[Fx::Ink(Ink::Item)]), plain(", and go.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Thank you.")], ..Choice::SAY }],
    },
    IlsaPassHolder: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Neutral)), text: &[plain("You've got your "), styled("pass", &[Fx::Ink(Ink::Item)]), plain(". The road's yours.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    HarbourBoard: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("Notices flap on the post, nailed over older notices.")] },
            Line { by: Prop(PropId::HarbourBoard), face: None, text: &[styled("FERRY SUSPENDED", &[Fx::Voice(Voice::Shout)]), plain(" until the orc trouble passes. By order of "), styled("Captain Bram", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Prop(PropId::HarbourBoard), face: None, text: &[plain("WANTED: strong arms for the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(". Ask "), styled("Mara", &[Fx::Ink(Ink::Name)]), plain(" at the market.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Step away.")], ..Choice::SAY }],
    },
    TideChestFound: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("Wedged between the rocks, the chest is heavy with the tide's leavings.")] },
            Line { by: Narrator, face: None, text: &[plain("You find five "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(" coins.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    TideChestEmpty: DialogueNode {
        lines: &[Line { by: Narrator, face: None, text: &[plain("Only sand, and a crab that does not appreciate the visit. "), styled("The tide brings more each day.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    TideChestMap: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("You find five "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(" coins. Under them, a scrap of oilcloth is wrapped around a "), styled("tattered map", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Narrator, face: None, text: &[styled("Unfold it from your bag to read it.", &[Fx::Voice(Voice::Whisper)])] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    StandingStone: DialogueNode {
        lines: &[Line { by: Narrator, face: None, text: &[plain("Moss has crept over the runes. Whoever carved them wanted them found, then forgot why.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Step away.")], ..Choice::SAY }],
    },
    MaraCold: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), text: &[plain("I heard where my tusks went. "), styled("Back to the orcs.", &[Fx::Slow])] },
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Neutral)), text: &[plain("Your coin still spends. Don't expect more than that.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    TusksOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), text: &[plain("Those "), styled("orcs", &[Fx::Ink(Ink::Danger)]), plain(" have cost me three crates this week. I want it to cost them something.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(ExpressionId::MaraCounting)), text: &[plain("Bring me five "), styled("Orc Tusks", &[Fx::Ink(Ink::Item)]), plain(". Put their "), styled("chief", &[Fx::Ink(Ink::Danger)]), plain(" in the sand and I'll make it worth your while.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Consider it done.")], then: &[&AcceptQuest(QuestId::TusksForTheChief)], ..Choice::SAY },
            Choice { label: &[plain("Not today.")], ..Choice::SAY },
        ],
    },
    TusksNotYet: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Mara), face: Some(Individual(ExpressionId::MaraSmug)), text: &[plain("Five tusks and a dead chief. I don't pay for half a job.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("I'm on it.")], ..Choice::SAY }],
    },
    TusksThanks: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Surprised)), text: &[plain("The chief too? You don't do things by halves.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Happy)), text: &[plain("Here's your purse, and take your pick of the good stock.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    UgraHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), text: &[plain("You walk loud, outlander. The forest hears you long before I do.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), text: &[plain("Speak, if you came to speak.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("I'll leave you be.")], ..Choice::SAY }],
    },
    UgraFriend: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), text: &[plain("Friend of the clan. The fire is yours to sit by.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Farewell, Ugra.")], ..Choice::SAY }],
    },
    UgraSilent: DialogueNode {
        lines: &[Line { by: Narrator, face: None, text: &[plain("Ugra keeps her eyes on the fire. She has nothing to say to you.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave.")], ..Choice::SAY }],
    },
    PleaOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), text: &[plain("You hunt for the trader woman. I smell her coin on you.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Sad)), text: &[plain("Those tusks were cut from "), styled("our dead", &[Fx::Slow]), plain(". Hear me before you sell them.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice {
                label: &[plain("I'm listening.")],
                then: &[&AcceptQuest(QuestId::ShamansPlea), &GotoNode(Id::PleaAnswer)],
                ..Choice::SAY
            },
            Choice { label: &[plain("Not now.")], ..Choice::SAY },
        ],
    },
    PleaWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), text: &[plain("The tusks. Have you decided?")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    PleaAnswer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), text: &[plain("Give me the tusks of my kin, and the clan will call you friend.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), text: &[plain("Keep them, and you are "), styled("nothing", &[Fx::Ink(Ink::Danger)]), plain(" to us.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice {
                label: &[plain("Give her the tusks.")],
                requires: &[&OnQuest(QuestId::TusksForTheChief)],
                costs: &[ItemStack::new(ItemId::OrcTusk, 5)],
                then: &[
                    &TurnIn(QuestId::ShamansPlea),
                    &FailQuest(QuestId::TusksForTheChief),
                    &Remember(MemoryId::SidedWithOrcs),
                    &GotoNode(Id::UgraGrateful),
                ],
                reveal: &[Locked, Needs, Costs],
                warn: Some("Permanent · Tusks for the Chief fails"),
            },
            Choice {
                label: &[plain("Keep them.")],
                then: &[&TurnIn(QuestId::ShamansPlea), &Remember(MemoryId::SpurnedUgra), &GotoNode(Id::UgraSpurned)],
                warn: Some("Permanent · Ugra will never speak to you again"),
                ..Choice::SAY
            },
            Choice { label: &[plain("Let me think on it.")], ..Choice::SAY },
        ],
    },
    UgraGrateful: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Surprised)), text: &[plain("You... truly?")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), text: &[plain("Then the clan owes you. Come back when you need remedies, or work.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Rest easy, Ugra.")], ..Choice::SAY }],
    },
    UgraSpurned: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), text: &[plain("Then go. "), styled("And do not come back.", &[Fx::Voice(Voice::Shout)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave.")], ..Choice::SAY }],
    },
    FallenOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Sad)), text: &[plain("The "), styled("skeletons", &[Fx::Ink(Ink::Danger)]), plain(" by the shore were warriors once. They walk because no one sang for them.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), text: &[plain("Bring me eight of their "), styled("Bones", &[Fx::Ink(Ink::Item)]), plain(", and I will.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll bring them.")], then: &[&AcceptQuest(QuestId::RestForTheFallen)], ..Choice::SAY },
            Choice { label: &[plain("Another time.")], ..Choice::SAY },
        ],
    },
    FallenWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), text: &[plain("Eight bones. The song needs all of them.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("I'm gathering them.")], ..Choice::SAY }],
    },
    FallenThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), text: &[plain("You carry them gently. "), styled("Good.", &[Fx::PauseAfter(Millis(500.0))]), plain(" Tonight, they sleep.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    LetterOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), text: &[plain("Do me a kindness? This letter needs to reach "), styled("Captain Bram", &[Fx::Ink(Ink::Name)]), plain(" on the "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Individual(ExpressionId::TobbLaughing)), text: &[plain("My knees and that gangplank are not on speaking terms.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll take it.")], then: &[&AcceptQuest(QuestId::LetterForTheCaptain)], ..Choice::SAY },
            Choice { label: &[plain("Sorry, I'm busy.")], ..Choice::SAY },
        ],
    },
    LetterWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), text: &[plain("Bram's on his ship, at the end of the pier. Can't miss him. He's the one scowling.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("On my way.")], ..Choice::SAY }],
    },
    LetterThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Surprised)), text: &[plain("A letter? From "), styled("Tobb", &[Fx::Ink(Ink::Name)]), plain("? He's never written a word in his life.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Never mind.")], ..Choice::SAY }],
    },
    BatsOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Angry)), text: &[styled("Vampire bats", &[Fx::Ink(Ink::Danger)]), plain(" have taken the old belfry, and one of the devils made off with my "), styled("key", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Neutral)), text: &[plain("Thin them out and bring the key back. Tobb vouches for you, so I'll pay.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll deal with them.")], then: &[&AcceptQuest(QuestId::BatsInTheBelfry)], ..Choice::SAY },
            Choice { label: &[plain("Not now.")], ..Choice::SAY },
        ],
    },
    BatsWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Thinking)), text: &[plain("Still squeaking up there. Keep at it, and keep an eye out for that key.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Understood.")], ..Choice::SAY }],
    },
    BatsThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Happy)), text: &[plain("Quiet up there at last, and my key besides. You've earned this.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    LowTideOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), text: &[plain("Tide's turning and I've nothing in the basket! Three "), styled("Fish Steaks", &[Fx::Ink(Ink::Item)]), plain(", quick.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), text: &[plain("Before the water's out, mind. "), styled("Ten minutes.", &[Fx::Ink(Ink::Danger)])] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll hurry.")], then: &[&AcceptQuest(QuestId::LowTide)], ..Choice::SAY },
            Choice { label: &[plain("Can't right now.")], ..Choice::SAY },
        ],
    },
    LowTideWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Sad)), text: &[plain("The gulls are circling. Three steaks, quick as you can!")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Working on it.")], ..Choice::SAY }],
    },
    LowTideThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Individual(ExpressionId::TobbLaughing)), text: &[plain("Just in time! Take some bait. You'll have the gulls following you for a week.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    TitheOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Wren), face: Some(Individual(ExpressionId::WrenSleepy)), text: &[plain("Ten "), styled("Bones", &[Fx::Ink(Ink::Item)]), plain(" a day keeps the dead from wandering.")] },
            Line { by: Npc(NpcId::Wren), face: Some(Generic(Neutral)), text: &[plain("Bring them, and I pay in "), styled("tokens", &[Fx::Ink(Ink::Item)]), plain(". Every day.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll bring today's bones.")], then: &[&AcceptQuest(QuestId::BoneTithe)], ..Choice::SAY },
            Choice { label: &[plain("Maybe tomorrow.")], ..Choice::SAY },
        ],
    },
    TitheWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Wren), face: Some(Generic(Thinking)), text: &[plain("Ten. Not nine.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Right.")], ..Choice::SAY }],
    },
    TitheThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Wren), face: Some(Generic(Happy)), text: &[plain("Good. The dead stay down another night.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    XMarksOffer: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("Salt has eaten most of the ink. A cross is scrawled beside a "), styled("standing stone", &[Fx::Ink(Ink::Place)]), plain(" in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(".")] },
            Line { by: Narrator, face: None, text: &[plain("Someone buried something there.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Follow the map.")], then: &[&AcceptQuest(QuestId::XMarksTheSpot)], ..Choice::SAY },
            Choice { label: &[plain("Fold it away.")], ..Choice::SAY },
        ],
    },
    XMarksWaiting: DialogueNode {
        lines: &[Line { by: Narrator, face: None, text: &[plain("The cross sits beside a standing stone, somewhere in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(".")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Fold it away.")], ..Choice::SAY }],
    },
    XMarksThanks: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, text: &[plain("The stone matches the drawing, down to the crack in its side.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave it buried.")], ..Choice::SAY }],
    },
    GrishaRumourChief: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), text: &[plain("The orc "), styled("chief", &[Fx::Ink(Ink::Danger)]), plain("? Camps on the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" with his guard around him.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), text: &[plain("There's a shaman with that clan, out in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(". "), styled("They say she's not keen on the fighting.", &[Fx::Voice(Voice::Whisper)])] },
        ],
        enter: &[],
        topics: false,
        choices: GRISHA_CHAT_DONE,
    },
}

static GRISHA_CHOICES: &[Choice] = &[
    Choice {
        label: &[plain("Rest until I'm mended.")],
        requires: &[&Not(&RemembersAtLeast(MemoryId::InnFavour, 3))],
        costs: &[ItemStack::new(ItemId::Gold, 5)],
        then: &[&Heal::Fully, &GotoNode(Id::GrishaRested)],
        reveal: &[Costs],
        warn: None,
    },
    Choice {
        label: &[plain("Rest until I'm mended.")],
        requires: &[&RemembersAtLeast(MemoryId::InnFavour, 3)],
        costs: &[ItemStack::new(ItemId::Gold, 3)],
        then: &[&Heal::Fully, &GotoNode(Id::GrishaRested)],
        reveal: &[Costs],
        warn: None,
    },
    Choice {
        label: &[plain("Buy a round for the room.")],
        costs: &[ItemStack::new(ItemId::Gold, 10)],
        then: &[&Remember(MemoryId::InnFavour), &GotoNode(Id::GrishaRound)],
        reveal: &[Costs],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Chat.")],
        then: &[&GotoNode(Id::GrishaChat)],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Goodbye.")],
        ..Choice::SAY
    },
];

static GRISHA_CHAT_DONE: &[Choice] = &[Choice {
    label: &[plain("Ok.")],
    then: &[&GotoNode(Id::GrishaChat)],
    ..Choice::SAY
}];
