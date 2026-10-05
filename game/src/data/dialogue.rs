use crate::core::time::Millis;
use crate::data::item::Id as ItemId;
use crate::data::memory::Id as MemoryId;
use crate::data::npc::Id as NpcId;
use crate::data::prop::Id as PropId;
use crate::systems::actor::bust::Face::{Generic, Individual};
use crate::systems::actor::bust::GenericExpression::{
    Angry, Happy, Neutral, Sad, Surprised, Thinking,
};
use crate::systems::actor::bust::IndividualExpression::{Counting, Laughing, Sleepy, Smirk, Smug};
use crate::systems::dialogue::Speaker::{Narrator, Npc, Player, Prop};
use crate::systems::dialogue::text::{Fx, Ink, Motion, Voice, plain, styled};
use crate::systems::dialogue::{Choice, DialogueNode, GotoNode, Line, Unmet};
use crate::systems::item::{GiveItems, ItemStack};
use crate::systems::memory::{Remember, RemembersAtLeast};
use crate::systems::quest::{AcceptQuest, FailQuest, OnQuest, QuestId, TurnIn};
use crate::systems::rule::Not;
use crate::systems::shop::CloseShop;
use crate::systems::stat::Heal;

crate::table! {
    TobbHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), cue: true, text: &[plain("Ho there! Name's Tobb. Been fishing this harbour since I could hold a rod.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), cue: false, text: &[plain("If it swims, I've caught it. If it talks, I've heard it.")] },
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
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), cue: false, text: &[plain("A boot, two crabs and a very rude gull.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Individual(Laughing)), cue: true, text: &[plain("Best day all week!")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Good luck out there.")], ..Choice::SAY }],
    },
    TobbAgain: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), cue: true, text: &[plain("Didn't I just see you? The fish won't catch themselves, you know.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Sorry, Tobb.")], ..Choice::SAY }],
    },
    TobbBack: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), cue: true, text: &[plain("Back again! Pull up a crate.")] }],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[
            Choice { label: &[plain("Caught anything good today?")], then: &[&GotoNode(Id::TobbCatch)], ..Choice::SAY },
            Choice { label: &[plain("See you around, Tobb.")], ..Choice::SAY },
        ],
    },
    TobbRegular: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), cue: true, text: &[plain("You've got patience, I'll give you that. Most folk don't come back to hear about crabs.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), cue: false, text: &[plain("Keep it up and I might just have something for you.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[Choice { label: &[plain("I'll hold you to that.")], ..Choice::SAY }],
    },
    TobbGift: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), cue: true, text: &[plain("Here. My "), styled("lucky lure", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), cue: false, text: &[plain("Don't lose it. It's caught more fish than I have.")] },
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
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), cue: true, text: &[styled("Pell", &[Fx::Ink(Ink::Name)]), plain(" loads his dice, you know. "), styled("Everyone says so.", &[Fx::Voice(Voice::Whisper)])] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Sad)), cue: false, text: &[plain("Nobody can prove it, mind.")] },
        ],
        enter: &[&Remember(MemoryId::TobbVisits)],
        topics: true,
        choices: &[Choice { label: &[plain("I'll keep an eye on him.")], ..Choice::SAY }],
    },
    TobbNews: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), cue: true, text: &[plain("Oi! Over here! You'll want to hear this.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), cue: false, text: &[styled("Orcs", &[Fx::Ink(Ink::Danger)]), plain(" were seen on the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" at dawn. Big ones, with a chief in a bone helmet.")] },
            Line { by: Player, face: Some(Generic(Thinking)), cue: true, text: &[plain("Thanks for the warning.")] },
        ],
        enter: &[&Remember(MemoryId::TobbNewsToday)],
        topics: false,
        choices: &[Choice { label: &[plain("I'll keep my eyes open.")], ..Choice::SAY }],
    },
    GrishaHello: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), cue: true, text: &[plain("Welcome to the "), styled("Driftwood Inn", &[Fx::Ink(Ink::Place)]), plain("! A bed, a bowl, and all the gossip you can stomach.")] }],
        enter: &[],
        topics: true,
        choices: GRISHA_CHOICES,
    },
    GrishaRegular: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), cue: true, text: &[plain("My favourite guest! Rest your bones. For you, three "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(".")] }],
        enter: &[],
        topics: true,
        choices: GRISHA_CHOICES,
    },
    GrishaRested: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), cue: false, text: &[plain("There. Slept like a log, you did.")] },
            Line { by: Player, face: Some(Generic(Happy)), cue: true, text: &[plain("Good as new.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Thanks, Grisha.")], ..Choice::SAY }],
    },
    GrishaRound: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("The room cheers. Someone starts a song about a mermaid and a very confused sailor.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Happy)), cue: true, text: &[plain("Keep that up and you'll be a regular in no time.")] },
        ],
        enter: &[],
        topics: false,
        choices: GRISHA_CHOICES,
    },
    GrishaRumourWren: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), cue: true, text: &[styled("Wren", &[Fx::Ink(Ink::Name)]), plain(" pays good tokens for bones. Don't ask what she does with them.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    GrishaRumourBram: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Grisha), face: Some(Generic(Sad)), cue: true, text: &[plain("Between you and me? "), styled("Bram", &[Fx::Ink(Ink::Name)]), plain("'s ferry hasn't left since the orcs came. He's waiting on a brave fool.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    MaraHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Happy)), cue: true, text: &[plain("Welcome, traveller! Fresh crates off the boat this morning.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(Smug)), cue: true, text: &[plain("And if you've got a strong arm, I might have a job that pays.")] },
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
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), cue: true, text: &[styled("Orcs", &[Fx::Ink(Ink::Danger)]), plain(". They come down from the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" at night and take whatever isn't nailed down.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(Counting)), cue: true, text: &[plain("Do you know what a crate of silk costs? I do. "), styled("To the copper.", &[Fx::Slow])] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    BramHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Neutral)), cue: true, text: &[plain("Captain Bram, of the "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Thinking)), cue: false, text: &[plain("She sails for the "), styled("forest shore", &[Fx::Ink(Ink::Place)]), plain(" when the tide is right. And when I say so.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    MaraShopping: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Mara), face: Some(Individual(Counting)), cue: true, text: &[plain("Take your time. Everything's priced fair. "), styled("Mostly.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("That's all, thanks.")], then: &[&CloseShop], ..Choice::SAY }],
    },
    WrenHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Wren), face: Some(Individual(Sleepy)), cue: true, text: &[plain("Bones. Wings. Nothing else.")] },
            Line { by: Npc(NpcId::Wren), face: Some(Generic(Thinking)), cue: false, text: &[plain("If you find any, you know where I am.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    PellHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Pell), face: Some(Individual(Smirk)), cue: true, text: &[plain("Care for a "), styled("little game", &[Fx::Motion(Motion::Wave)]), plain("? Ten "), styled("Gold", &[Fx::Ink(Ink::Item)]), styled(".", &[Fx::PauseAfter(Millis(600.0))]), styled(" I never cheat.", &[Fx::Voice(Voice::Whisper)])] },
            Line { by: Npc(NpcId::Pell), face: Some(Generic(Neutral)), cue: false, text: &[plain("The table's not set yet. Come back later, friend.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    IlsaHello: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ilsa), face: Some(Generic(Angry)), cue: true, text: &[plain("The forest road is "), styled("closed", &[Fx::Ink(Ink::Danger)]), plain(". Orders from the harbour master.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("I'll turn back.")], ..Choice::SAY }],
    },
    HarbourNotices: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("Notices flap on the post, nailed over older notices.")] },
            Line { by: Prop(PropId::HarbourNotices), face: None, cue: false, text: &[styled("FERRY SUSPENDED", &[Fx::Voice(Voice::Shout)]), plain(" until the orc trouble passes. By order of "), styled("Captain Bram", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Prop(PropId::HarbourNotices), face: None, cue: false, text: &[plain("WANTED: strong arms for the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(". Ask "), styled("Mara", &[Fx::Ink(Ink::Name)]), plain(" at the market.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Step away.")], ..Choice::SAY }],
    },
    TideChestFound: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("Wedged between the rocks, the chest is heavy with the tide's leavings.")] },
            Line { by: Narrator, face: None, cue: false, text: &[plain("You find five "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(" coins.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    TideChestEmpty: DialogueNode {
        lines: &[Line { by: Narrator, face: None, cue: false, text: &[plain("Only sand, and a crab that does not appreciate the visit. "), styled("The tide brings more each day.", &[Fx::Voice(Voice::Whisper)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    TideChestMap: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("You find five "), styled("Gold", &[Fx::Ink(Ink::Item)]), plain(" coins. Under them, a scrap of oilcloth is wrapped around a "), styled("tattered map", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Narrator, face: None, cue: false, text: &[styled("Unfold it from your bag to read it.", &[Fx::Voice(Voice::Whisper)])] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Close the lid.")], ..Choice::SAY }],
    },
    StandingStone: DialogueNode {
        lines: &[Line { by: Narrator, face: None, cue: false, text: &[plain("Moss has crept over the runes. Whoever carved them wanted them found, then forgot why.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Step away.")], ..Choice::SAY }],
    },
    MaraCold: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), cue: true, text: &[plain("I heard where my tusks went. "), styled("Back to the orcs.", &[Fx::Slow])] },
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Neutral)), cue: false, text: &[plain("Your coin still spends. Don't expect more than that.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
    TusksOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Angry)), cue: true, text: &[plain("Those "), styled("orcs", &[Fx::Ink(Ink::Danger)]), plain(" have cost me three crates this week. I want it to cost them something.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Individual(Counting)), cue: true, text: &[plain("Bring me five "), styled("Orc Tusks", &[Fx::Ink(Ink::Item)]), plain(". Put their "), styled("chief", &[Fx::Ink(Ink::Danger)]), plain(" in the sand and I'll make it worth your while.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Consider it done.")], then: &[&AcceptQuest(QuestId::TusksForTheChief)], ..Choice::SAY },
            Choice { label: &[plain("Not today.")], ..Choice::SAY },
        ],
    },
    TusksNotYet: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Mara), face: Some(Individual(Smug)), cue: true, text: &[plain("Five tusks and a dead chief. I don't pay for half a job.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("I'm on it.")], ..Choice::SAY }],
    },
    TusksThanks: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Surprised)), cue: true, text: &[plain("The chief too? You don't do things by halves.")] },
            Line { by: Npc(NpcId::Mara), face: Some(Generic(Happy)), cue: true, text: &[plain("Here's your purse, and take your pick of the good stock.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    UgraHello: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), cue: true, text: &[plain("You walk loud, outlander. The forest hears you long before I do.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), cue: false, text: &[plain("Speak, if you came to speak.")] },
        ],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("I'll leave you be.")], ..Choice::SAY }],
    },
    UgraFriend: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), cue: true, text: &[plain("Friend of the clan. The fire is yours to sit by.")] }],
        enter: &[],
        topics: true,
        choices: &[Choice { label: &[plain("Farewell, Ugra.")], ..Choice::SAY }],
    },
    UgraSilent: DialogueNode {
        lines: &[Line { by: Narrator, face: None, cue: false, text: &[plain("Ugra keeps her eyes on the fire. She has nothing to say to you.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave.")], ..Choice::SAY }],
    },
    PleaOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), cue: true, text: &[plain("You hunt for the trader woman. I smell her coin on you.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Sad)), cue: true, text: &[plain("Those tusks were cut from "), styled("our dead", &[Fx::Slow]), plain(". Hear me before you sell them.")] },
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
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), cue: true, text: &[plain("The tusks. Have you decided?")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    PleaAnswer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), cue: true, text: &[plain("Give me the tusks of my kin, and the clan will call you friend.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), cue: false, text: &[plain("Keep them, and you are "), styled("nothing", &[Fx::Ink(Ink::Danger)]), plain(" to us.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice {
                label: &[plain("Give her the tusks.")],
                requires: &[&OnQuest(QuestId::TusksForTheChief)],
                unmet: Unmet::ShowLocked,
                costs: &[ItemStack::new(ItemId::OrcTusk, 5)],
                then: &[
                    &TurnIn(QuestId::ShamansPlea),
                    &FailQuest(QuestId::TusksForTheChief),
                    &Remember(MemoryId::SidedWithOrcs),
                    &GotoNode(Id::UgraGrateful),
                ],
                warn: Some("Permanent · Tusks for the Chief fails, and Mara won't forget it"),
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
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Surprised)), cue: true, text: &[plain("You... truly?")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), cue: true, text: &[plain("Then the clan owes you. Come back when you need remedies, or work.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Rest easy, Ugra.")], ..Choice::SAY }],
    },
    UgraSpurned: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Angry)), cue: true, text: &[plain("Then go. "), styled("And do not come back.", &[Fx::Voice(Voice::Shout)])] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave.")], ..Choice::SAY }],
    },
    FallenOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Sad)), cue: true, text: &[plain("The "), styled("skeletons", &[Fx::Ink(Ink::Danger)]), plain(" by the shore were warriors once. They walk because no one sang for them.")] },
            Line { by: Npc(NpcId::Ugra), face: Some(Generic(Neutral)), cue: false, text: &[plain("Bring me eight of their "), styled("Bones", &[Fx::Ink(Ink::Item)]), plain(", and I will.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll bring them.")], then: &[&AcceptQuest(QuestId::RestForTheFallen)], ..Choice::SAY },
            Choice { label: &[plain("Another time.")], ..Choice::SAY },
        ],
    },
    FallenWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Thinking)), cue: true, text: &[plain("Eight bones. The song needs all of them.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("I'm gathering them.")], ..Choice::SAY }],
    },
    FallenThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Ugra), face: Some(Generic(Happy)), cue: true, text: &[plain("You carry them gently. "), styled("Good.", &[Fx::PauseAfter(Millis(500.0))]), plain(" Tonight, they sleep.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    LetterOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Thinking)), cue: true, text: &[plain("Do me a kindness? This letter needs to reach "), styled("Captain Bram", &[Fx::Ink(Ink::Name)]), plain(" on the "), styled("Gull", &[Fx::Ink(Ink::Name)]), plain(".")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Individual(Laughing)), cue: true, text: &[plain("My knees and that gangplank are not on speaking terms.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll take it.")], then: &[&AcceptQuest(QuestId::LetterForTheCaptain)], ..Choice::SAY },
            Choice { label: &[plain("Sorry, I'm busy.")], ..Choice::SAY },
        ],
    },
    LetterWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Neutral)), cue: true, text: &[plain("Bram's on his ship, at the end of the pier. Can't miss him. He's the one scowling.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("On my way.")], ..Choice::SAY }],
    },
    LetterThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Surprised)), cue: true, text: &[plain("A letter? From "), styled("Tobb", &[Fx::Ink(Ink::Name)]), plain("? He's never written a word in his life.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Never mind.")], ..Choice::SAY }],
    },
    BatsOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Angry)), cue: true, text: &[styled("Vampire bats", &[Fx::Ink(Ink::Danger)]), plain(" have taken the old belfry, and one of the devils made off with my "), styled("key", &[Fx::Ink(Ink::Item)]), plain(".")] },
            Line { by: Npc(NpcId::Bram), face: Some(Generic(Neutral)), cue: false, text: &[plain("Thin them out and bring the key back. Tobb vouches for you, so I'll pay.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll deal with them.")], then: &[&AcceptQuest(QuestId::BatsInTheBelfry)], ..Choice::SAY },
            Choice { label: &[plain("Not now.")], ..Choice::SAY },
        ],
    },
    BatsWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Thinking)), cue: true, text: &[plain("Still squeaking up there. Keep at it, and keep an eye out for that key.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Understood.")], ..Choice::SAY }],
    },
    BatsThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Bram), face: Some(Generic(Happy)), cue: true, text: &[plain("Quiet up there at last, and my key besides. You've earned this.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    LowTideOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Surprised)), cue: true, text: &[plain("Tide's turning and I've nothing in the basket! Three "), styled("Fish Steaks", &[Fx::Ink(Ink::Item)]), plain(", quick.")] },
            Line { by: Npc(NpcId::Tobb), face: Some(Generic(Happy)), cue: false, text: &[plain("Before the water's out, mind. "), styled("Ten minutes.", &[Fx::Ink(Ink::Danger)])] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll hurry.")], then: &[&AcceptQuest(QuestId::LowTide)], ..Choice::SAY },
            Choice { label: &[plain("Can't right now.")], ..Choice::SAY },
        ],
    },
    LowTideWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Generic(Sad)), cue: true, text: &[plain("The gulls are circling. Three steaks, quick as you can!")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Working on it.")], ..Choice::SAY }],
    },
    LowTideThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Tobb), face: Some(Individual(Laughing)), cue: true, text: &[plain("Just in time! Take some bait. You'll have the gulls following you for a week.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    TitheOffer: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Wren), face: Some(Individual(Sleepy)), cue: true, text: &[plain("Ten "), styled("Bones", &[Fx::Ink(Ink::Item)]), plain(" a day keeps the dead from wandering.")] },
            Line { by: Npc(NpcId::Wren), face: Some(Generic(Neutral)), cue: false, text: &[plain("Bring them, and I pay in "), styled("tokens", &[Fx::Ink(Ink::Item)]), plain(". Every day.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("I'll bring today's bones.")], then: &[&AcceptQuest(QuestId::BoneTithe)], ..Choice::SAY },
            Choice { label: &[plain("Maybe tomorrow.")], ..Choice::SAY },
        ],
    },
    TitheWaiting: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Wren), face: Some(Generic(Thinking)), cue: false, text: &[plain("Ten. Not nine.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Right.")], ..Choice::SAY }],
    },
    TitheThanks: DialogueNode {
        lines: &[Line { by: Npc(NpcId::Wren), face: Some(Generic(Happy)), cue: false, text: &[plain("Good. The dead stay down another night.")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Not yet.")], ..Choice::SAY }],
    },
    XMarksOffer: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("Salt has eaten most of the ink. A cross is scrawled beside a "), styled("standing stone", &[Fx::Ink(Ink::Place)]), plain(" in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(".")] },
            Line { by: Player, face: Some(Generic(Thinking)), cue: true, text: &[plain("Someone buried something there.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[
            Choice { label: &[plain("Follow the map.")], then: &[&AcceptQuest(QuestId::XMarksTheSpot)], ..Choice::SAY },
            Choice { label: &[plain("Fold it away.")], ..Choice::SAY },
        ],
    },
    XMarksWaiting: DialogueNode {
        lines: &[Line { by: Narrator, face: None, cue: false, text: &[plain("The cross sits beside a standing stone, somewhere in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(".")] }],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Fold it away.")], ..Choice::SAY }],
    },
    XMarksThanks: DialogueNode {
        lines: &[
            Line { by: Narrator, face: None, cue: false, text: &[plain("The stone matches the drawing, down to the crack in its side.")] },
            Line { by: Player, face: Some(Generic(Happy)), cue: true, text: &[plain("This is the spot.")] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Leave it buried.")], ..Choice::SAY }],
    },
    GrishaRumourChief: DialogueNode {
        lines: &[
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Thinking)), cue: true, text: &[plain("The orc "), styled("chief", &[Fx::Ink(Ink::Danger)]), plain("? Camps on the "), styled("north shore", &[Fx::Ink(Ink::Place)]), plain(" with his guard around him.")] },
            Line { by: Npc(NpcId::Grisha), face: Some(Generic(Neutral)), cue: false, text: &[plain("There's a shaman with that clan, out in the "), styled("Forest", &[Fx::Ink(Ink::Place)]), plain(". "), styled("They say she's not keen on the fighting.", &[Fx::Voice(Voice::Whisper)])] },
        ],
        enter: &[],
        topics: false,
        choices: &[Choice { label: &[plain("Goodbye.")], ..Choice::SAY }],
    },
}

static GRISHA_CHOICES: &[Choice] = &[
    Choice {
        label: &[plain("Rest until I'm mended.")],
        requires: &[&Not(&RemembersAtLeast(MemoryId::InnFavour, 3))],
        unmet: Unmet::Hide,
        costs: &[ItemStack::new(ItemId::Gold, 5)],
        then: &[&Heal::Fully, &GotoNode(Id::GrishaRested)],
        warn: None,
    },
    Choice {
        label: &[plain("Rest until I'm mended.")],
        requires: &[&RemembersAtLeast(MemoryId::InnFavour, 3)],
        unmet: Unmet::Hide,
        costs: &[ItemStack::new(ItemId::Gold, 3)],
        then: &[&Heal::Fully, &GotoNode(Id::GrishaRested)],
        warn: None,
    },
    Choice {
        label: &[plain("Buy a round for the room.")],
        costs: &[ItemStack::new(ItemId::Gold, 10)],
        then: &[&Remember(MemoryId::InnFavour), &GotoNode(Id::GrishaRound)],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Any rumours?")],
        requires: &[
            &Not(&RemembersAtLeast(MemoryId::InnFavour, 1)),
            &Not(&OnQuest(QuestId::TusksForTheChief)),
        ],
        unmet: Unmet::Hide,
        then: &[&GotoNode(Id::GrishaRumourWren)],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Any rumours?")],
        requires: &[
            &RemembersAtLeast(MemoryId::InnFavour, 1),
            &Not(&OnQuest(QuestId::TusksForTheChief)),
        ],
        unmet: Unmet::Hide,
        then: &[&GotoNode(Id::GrishaRumourBram)],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Any rumours?")],
        requires: &[&OnQuest(QuestId::TusksForTheChief)],
        unmet: Unmet::Hide,
        then: &[&GotoNode(Id::GrishaRumourChief)],
        ..Choice::SAY
    },
    Choice {
        label: &[plain("Goodbye.")],
        ..Choice::SAY
    },
];
