//! Messages: letters between players, each a title, a plain text and items attached, kept in
//! its receiver's inbox until they delete it. The game writes them too, to hand over what a
//! bag had no room for.

pub mod draft;
pub mod page;
pub mod reads;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use reads::InboxReads;

use crate::core::content::{Content, SavedList};
use crate::core::store::{Record, RecordId, RecordOwner};
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::account::AccountId;
use crate::systems::history::HistoryTopic;
use crate::systems::input::map::InputAction;
use crate::systems::item::{self, ExchangeRefusal, Inventory, ItemStack};
use crate::systems::notification::{self, Notification};
use crate::systems::player::RequestNeeds;
use crate::systems::player::snapshot::KeptAside;
use crate::systems::player::{self, PlayedBy};
use crate::systems::save::Unwritten;
use crate::systems::society::{self, AccountNumber, Settling, Society, SocietyAsk, SocietyMember};
use crate::systems::text::{LineText, SpanText};
use crate::systems::{GameSettings, GameWords};

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<InboxView>()
        .add_client_message::<SendMessage>()
        .add_client_message::<OpenMessage>()
        .add_client_message::<TakeAttached>()
        .add_client_message::<DeleteMessage>()
        .add_server_message::<MessageText>()
        .add_server_message::<MessageAnswered>();
}

/// How a game has messages work. A game without it has none.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSettings {
    /// How many messages from players an inbox holds. Messages from the game are not counted.
    pub inbox: u32,
    /// How many stacks of items a message carries at the most.
    pub attachments: u32,
    /// How many letters a title is long at the most.
    pub title_length: u32,
    /// How many letters a text is long at the most.
    pub text_length: u32,
    /// How many messages a player sends within a minute at the most.
    pub a_minute: u32,
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct MessageId(pub u64);

/// Who wrote a message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageSender {
    Player(AccountId),
    Game,
}

/// A message in its receiver's inbox.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InboxMessage {
    pub id: MessageId,
    pub to: AccountId,
    pub from: MessageSender,
    pub title: String,
    pub text: String,
    /// The stacks still attached. One naming an item the game no longer has waits set aside.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub attached: SavedList<ItemStack>,
    pub sent: UnixMillis,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
}

impl Record for InboxMessage {
    const KIND: &'static str = "message";

    fn id(&self) -> RecordId {
        record_id(self.id)
    }

    fn owner(&self) -> Option<RecordOwner> {
        Some(RecordOwner(self.to.0.clone()))
    }
}

/// Every inbox, as much of it as memory holds: the inboxes of accounts that play whole, and of
/// any other account only what no checkpoint has written yet.
#[derive(Default)]
pub struct Inboxes {
    /// How many messages from players each inbox holds, in memory or not.
    held: HashMap<AccountId, u32>,
    open: HashMap<AccountId, Inbox>,
    dealt: u64,
    /// When each account sent its last messages, within the minute a limit counts.
    sent: HashMap<AccountId, VecDeque<UnixMillis>>,
    reads: InboxReads,
    /// The accounts whose players see an inbox that changed since they were shown.
    changed: BTreeSet<AccountId>,
}

#[derive(Default)]
struct Inbox {
    messages: BTreeMap<MessageId, InboxMessage>,
    /// Whether what the store holds of it has been taken in, so memory holds all of it.
    whole: bool,
    reading: bool,
    /// The messages put, changed or deleted since the store was last known to hold them as
    /// memory does.
    unwritten: BTreeSet<MessageId>,
}

impl Inboxes {
    /// The inboxes of a server whose store holds `saved`, of which only the count for each
    /// account is kept, and which reads an inbox through `reads` as its account comes to play.
    pub fn counted(saved: impl IntoIterator<Item = InboxMessage>, reads: InboxReads) -> Inboxes {
        let mut inboxes = Inboxes {
            reads,
            ..Inboxes::default()
        };
        for message in saved {
            inboxes.dealt = inboxes.dealt.max(message.id.0);
            if matches!(message.from, MessageSender::Player(_)) {
                *inboxes.held.entry(message.to).or_default() += 1;
            }
        }
        inboxes
    }

    /// How many messages from players the inbox of `account` holds.
    pub fn held(&self, account: &AccountId) -> u32 {
        self.held.get(account).copied().unwrap_or_default()
    }

    /// The inbox of `account`, once memory holds all of it: the newest message first.
    pub fn of(&self, account: &AccountId) -> Option<impl Iterator<Item = &InboxMessage>> {
        let inbox = self.open.get(account).filter(|inbox| inbox.whole)?;
        Some(inbox.messages.values().rev())
    }

    /// Puts `message` in its receiver's inbox, under an id of its own.
    fn deliver(&mut self, message: InboxMessage, unwritten: &mut Unwritten) {
        self.dealt += 1;
        let message = InboxMessage {
            id: MessageId(self.dealt),
            ..message
        };
        if matches!(message.from, MessageSender::Player(_)) {
            *self.held.entry(message.to.clone()).or_default() += 1;
        }
        let inbox = self.open.entry(message.to.clone()).or_default();
        inbox.unwritten.insert(message.id);
        inbox.messages.insert(message.id, message.clone());
        self.changed.insert(message.to.clone());
        unwritten.put(message);
    }

    /// Changes the message `id` of the inbox of `account`, once memory holds all of it: the
    /// message as it then is.
    fn change(
        &mut self,
        account: &AccountId,
        id: MessageId,
        unwritten: &mut Unwritten,
        change: impl FnOnce(&mut InboxMessage),
    ) -> Option<&InboxMessage> {
        let inbox = self.open.get_mut(account).filter(|inbox| inbox.whole)?;
        let message = inbox.messages.get_mut(&id)?;
        change(message);
        inbox.unwritten.insert(id);
        unwritten.put(message.clone());
        self.changed.insert(account.clone());
        Some(message)
    }

    fn message(&self, account: &AccountId, id: MessageId) -> Option<&InboxMessage> {
        let inbox = self.open.get(account).filter(|inbox| inbox.whole)?;
        inbox.messages.get(&id)
    }

    fn delete(&mut self, account: &AccountId, id: MessageId, unwritten: &mut Unwritten) {
        let Some(inbox) = self.open.get_mut(account).filter(|inbox| inbox.whole) else {
            return;
        };
        let Some(message) = inbox.messages.remove(&id) else {
            return;
        };
        inbox.unwritten.insert(id);
        unwritten.remove::<InboxMessage>(record_id(id));
        if matches!(message.from, MessageSender::Player(_))
            && let Some(held) = self.held.get_mut(account)
        {
            *held = held.saturating_sub(1);
        }
        self.changed.insert(account.clone());
    }

    /// Has memory hold all of the inbox of `account`, who came to play: at once where there is
    /// no store to read, else once the store has answered.
    pub(crate) fn arrived(&mut self, account: &AccountId) {
        let inbox = self.open.entry(account.clone()).or_default();
        if inbox.whole || inbox.reading {
            return;
        }
        if self.reads.begin(account) {
            inbox.reading = true;
        } else {
            inbox.whole = true;
            self.changed.insert(account.clone());
        }
    }

    /// Takes in what the store answered, and lets go of the inboxes of accounts that are not
    /// `playing` once the store holds them as memory does. An inbox the store failed to answer
    /// for is asked for again.
    pub(crate) fn keep(&mut self, playing: impl Fn(&AccountId) -> bool, unwritten: &Unwritten) {
        for read in self.reads.finished() {
            let Some(inbox) = self.open.get_mut(&read.account) else {
                continue;
            };
            inbox.reading = false;
            let Ok(stored) = read.messages else {
                continue;
            };
            for message in stored {
                // Memory holds a message newer than the store does, or as the store does.
                if !inbox.unwritten.contains(&message.id) {
                    inbox.messages.entry(message.id).or_insert(message);
                }
            }
            inbox.whole = true;
            self.changed.insert(read.account);
        }
        let mut again = Vec::new();
        self.open.retain(|account, inbox| {
            inbox
                .unwritten
                .retain(|&id| unwritten.holds(&(InboxMessage::KIND, record_id(id))));
            let plays = playing(account);
            if plays && !inbox.whole && !inbox.reading {
                again.push(account.clone());
            }
            plays || !inbox.unwritten.is_empty()
        });
        for account in again {
            self.arrived(&account);
        }
    }

    pub(crate) fn changed(&mut self) -> BTreeSet<AccountId> {
        std::mem::take(&mut self.changed)
    }

    /// Whether `account` has sent as many messages within the last minute as `most`.
    fn writes_too_fast(&mut self, account: &AccountId, most: u32, now: UnixMillis) -> bool {
        let Some(sent) = self.sent.get_mut(account) else {
            return most == 0;
        };
        while sent
            .front()
            .is_some_and(|&at| now.since(at) >= Seconds(60.0))
        {
            sent.pop_front();
        }
        sent.len() >= most as usize
    }
}

/// What a player asks of their messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxAsk {
    Send(SendMessage),
    Open(MessageId),
    Take(TakeAttached),
    Delete(MessageId),
    /// That what their bag had no room for as they came back be sent to them.
    Overflow,
}

/// A player sending a message, with stacks of their bag attached.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SendMessage {
    pub to: AccountNumber,
    pub title: String,
    pub text: String,
    pub attached: Vec<ItemStack>,
}

/// A player opening a message, which marks it read and has the server send its text.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenMessage(pub MessageId);

/// A player taking what is attached to a message: the stack at `stack`, or every stack.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeAttached {
    pub message: MessageId,
    pub stack: Option<u32>,
}

/// A player deleting a message nothing is attached to.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteMessage(pub MessageId);

/// The text of a message its receiver opened.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MessageText {
    pub id: MessageId,
    pub text: String,
}

/// Whether the message a player sent was taken.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageAnswered {
    pub sent: bool,
}

/// A player's inbox as their client knows it: every message but its text, the newest first.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct InboxView {
    /// Whether the server has the whole inbox in hand. Until then there is nothing to list.
    pub whole: bool,
    pub rows: Vec<InboxRow>,
    /// How many of them are from players, which is what an inbox has room for only so many of.
    pub held: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InboxRow {
    pub id: MessageId,
    /// The player who wrote it. A message from the game has none.
    pub from: Option<SocietyMember>,
    pub title: String,
    pub sent: UnixMillis,
    pub read: bool,
    pub attached: Vec<ItemStack>,
    /// How many more stacks are attached that name items the game no longer has.
    pub set_aside: u32,
}

impl InboxView {
    pub fn unread(&self) -> u32 {
        self.rows.iter().filter(|row| !row.read).count() as u32
    }
}

/// Hands what players of this world asked of their messages to society.
pub fn requests(world: &mut World) {
    fn hand<M: Message>(world: &mut World, ask: fn(M) -> InboxAsk) {
        player::player_requests::<M>(
            world,
            RequestNeeds::Nothing,
            move |world, player, message| {
                if let Some(by) = world.get::<PlayedBy>(player) {
                    society::ask(world, by.client, SocietyAsk::Inbox(ask(message)));
                }
                Ok(())
            },
        );
    }
    hand::<SendMessage>(world, InboxAsk::Send);
    hand::<OpenMessage>(world, |open| InboxAsk::Open(open.0));
    hand::<TakeAttached>(world, InboxAsk::Take);
    hand::<DeleteMessage>(world, |delete| InboxAsk::Delete(delete.0));
}

/// Asks that what a character's bag had no room for as it came back be sent to its player, in
/// a game that has messages.
pub fn overflow(
    world: &mut World,
    kept: &mut QueryState<(&KeptAside, &PlayedBy), Changed<KeptAside>>,
) {
    if world.resource::<GameSettings>().messages.is_none() {
        return;
    }
    let over: Vec<_> = kept
        .iter(world)
        .filter(|(kept, _)| kept.holds_overflow())
        .map(|(_, by)| by.client)
        .collect();
    for client in over {
        society::ask(world, client, SocietyAsk::Inbox(InboxAsk::Overflow));
    }
}

/// Shows the player of `character`, played for `account`, their inbox as `society` holds it.
pub(crate) fn show(world: &mut World, character: Entity, account: &AccountId, society: &Society) {
    if world.resource::<GameSettings>().messages.is_none() {
        return;
    }
    let inboxes = society.inboxes();
    let view = match inboxes.of(account) {
        Some(messages) => InboxView {
            whole: true,
            rows: messages.map(|message| row(society, message)).collect(),
            held: inboxes.held(account),
        },
        None => InboxView::default(),
    };
    if world.get::<InboxView>(character) != Some(&view) {
        world.entity_mut(character).insert(view);
    }
}

/// Sends `to` a message from the game with `stacks` attached, whether they play or not.
pub fn from_game(
    society: &mut Society,
    unwritten: &mut Unwritten,
    now: UnixMillis,
    to: &AccountId,
    said: (&str, &str),
    stacks: Vec<ItemStack>,
) {
    let message = InboxMessage {
        id: MessageId(0),
        to: to.clone(),
        from: MessageSender::Game,
        title: said.0.to_owned(),
        text: said.1.to_owned(),
        attached: stacks.into(),
        sent: now,
        read: false,
    };
    society.inboxes.deliver(message, unwritten);
}

/// Whether `by` may write to `to`: only to a friend or a member of their party.
pub fn may_write(society: &Society, by: &AccountId, to: &AccountId) -> bool {
    by != to && (society.friends().are(by, to) || society.parties().together(by, to))
}

/// Does what `by` asked of their messages, under the game `settings` are of.
pub(crate) fn settle(
    settling: &mut Settling,
    settings: &GameSettings,
    by: &AccountId,
    ask: InboxAsk,
) {
    let Some(rules) = &settings.messages else {
        return;
    };
    match ask {
        InboxAsk::Send(message) => {
            let sent = send(settling, settings, rules, by, message);
            settling.send(by, MessageAnswered { sent });
        }
        InboxAsk::Open(id) => {
            let inboxes = &mut settling.society.inboxes;
            let opened = inboxes.change(by, id, settling.unwritten, |message| message.read = true);
            if let Some(text) = opened.map(|message| message.text.clone()) {
                settling.send(by, MessageText { id, text });
            }
        }
        InboxAsk::Take(take) => self::take(settling, settings, by, take),
        InboxAsk::Delete(id) => {
            let inboxes = &mut settling.society.inboxes;
            if inboxes
                .message(by, id)
                .is_some_and(|message| message.attached.is_empty())
            {
                inboxes.delete(by, id, settling.unwritten);
            }
        }
        InboxAsk::Overflow => {
            let Some((world, character)) = settling.reach(by) else {
                return;
            };
            let content = world.resource::<Content>().clone();
            let stacks = world
                .get_mut::<KeptAside>(character)
                .map(|mut kept| kept.take_overflow(&content))
                .unwrap_or_default();
            if stacks.is_empty() {
                return;
            }
            let said = (
                settings.words.did_not_fit_title.as_str(),
                settings.words.did_not_fit_text.as_str(),
            );
            from_game(
                settling.society,
                settling.unwritten,
                settling.now,
                by,
                said,
                stacks,
            );
        }
    }
}

/// Sends the message `by` wrote: whether it was. Everything is checked before a stack leaves the
/// sender's bag, and from there to the receiver's inbox nothing can fail.
fn send(
    settling: &mut Settling,
    settings: &GameSettings,
    rules: &MessageSettings,
    by: &AccountId,
    message: SendMessage,
) -> bool {
    let Some(to) = settling.society.accounts().numbered(message.to).cloned() else {
        return false;
    };
    let words = &settings.words;
    let name = settling.name(&to);
    let society = &mut *settling.society;
    let title = message.title.trim();
    let fits = title.chars().count() <= rules.title_length as usize
        && message.text.chars().count() <= rules.text_length as usize
        && message.attached.len() <= rules.attachments as usize
        && message.attached.iter().all(|stack| stack.count > 0);
    let refusal = if !fits {
        return false;
    } else if title.is_empty() {
        Some(words.message_needs_title.clone())
    } else if !may_write(society, by, &to) {
        Some(words.only_to_friends.clone())
    } else if society.blocks().blocks(&to, by) {
        Some(words.not_taking_messages.replace("{name}", &name))
    } else if society.inboxes.held(&to) >= rules.inbox {
        Some(words.inbox_full.replace("{name}", &name))
    } else if society
        .inboxes
        .writes_too_fast(by, rules.a_minute, settling.now)
    {
        Some(words.writing_too_fast.clone())
    } else {
        None
    };
    if let Some(why) = refusal {
        settling.refuse(by, why);
        return false;
    }
    let Some((world, character)) = settling.reach(by) else {
        return false;
    };
    let content = world.resource::<Content>().clone();
    let stays = message
        .attached
        .iter()
        .find(|stack| stack.item.get(&content).bound());
    let taken = match stays {
        Some(bound) => Err(words
            .stays_with_you
            .replace("{item}", &bound.item.get(&content).name)),
        None => item::effects::exchange(world, character, &message.attached, &[]),
    };
    if let Err(why) = taken {
        notification::error(world, character, why);
        return false;
    }
    let sender = settling.name(by);
    let society = &mut *settling.society;
    society
        .inboxes
        .sent
        .entry(by.clone())
        .or_default()
        .push_back(settling.now);
    let letter = InboxMessage {
        id: MessageId(0),
        to: to.clone(),
        from: MessageSender::Player(by.clone()),
        title: title.to_owned(),
        text: message.text,
        attached: message.attached.into(),
        sent: settling.now,
        read: false,
    };
    society.inboxes.deliver(letter, settling.unwritten);
    let sent = words.message_sent.replace("{name}", &name);
    settling.tell(by, Notification::feed(HistoryTopic::Notification, sent));
    let arrived = words
        .message_arrived
        .replace("{name}", &sender)
        .replace("{title}", title);
    let arrived = keyed(&arrived, InputAction::OpenSocial);
    settling.tell(&to, Notification::feed(HistoryTopic::Notification, arrived));
    true
}

/// Hands `by` what is attached to a message of theirs, as much of it as their bag has room for:
/// each stack is tried on its own, and what does not fit stays attached.
fn take(settling: &mut Settling, settings: &GameSettings, by: &AccountId, take: TakeAttached) {
    let Some(message) = settling.society.inboxes.message(by, take.message) else {
        return;
    };
    let attached: Vec<ItemStack> = message.attached.to_vec();
    let (wanted, others): (Vec<ItemStack>, Vec<ItemStack>) = match take.stack {
        None => (attached, Vec::new()),
        Some(index) => {
            let mut others = attached;
            if index as usize >= others.len() {
                return;
            }
            (vec![others.remove(index as usize)], others)
        }
    };
    if wanted.is_empty() {
        return;
    }
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let Some(mut bag) = world.get::<Inventory>(character).cloned() else {
        return;
    };
    let left = bag.give_what_fits(&content, &wanted);
    let taken = wanted.len() - left.len();
    if taken > 0 {
        world.entity_mut(character).insert(bag);
    }
    if !left.is_empty() {
        let why = if taken == 0 {
            ExchangeRefusal::NoRoom { slots: 1 }.describe(&content)
        } else {
            settings
                .words
                .took_some
                .replace("{taken}", &taken.to_string())
                .replace("{attached}", &wanted.len().to_string())
        };
        notification::error(world, character, why);
    }
    if taken == 0 {
        return;
    }
    let mut still: Vec<ItemStack> = match take.stack {
        None => left,
        Some(index) => {
            let mut still = others;
            still.splice(index as usize..index as usize, left);
            still
        }
    };
    settling
        .society
        .inboxes
        .change(by, take.message, settling.unwritten, |message| {
            message.attached.clear();
            message.attached.append(&mut still);
        });
}

fn row(society: &Society, message: &InboxMessage) -> InboxRow {
    InboxRow {
        id: message.id,
        from: match &message.from {
            MessageSender::Player(account) => society.member(account),
            MessageSender::Game => None,
        },
        title: message.title.clone(),
        sent: message.sent,
        read: message.read,
        attached: message.attached.to_vec(),
        set_aside: message.attached.set_aside().len() as u32,
    }
}

/// `said`, with the key of `action` wherever it says `{key}`.
fn keyed(said: &str, action: InputAction) -> LineText {
    let mut spans = Vec::new();
    for (index, part) in said.split("{key}").enumerate() {
        if index > 0 {
            spans.push(SpanText::Input(action));
        }
        if !part.is_empty() {
            spans.push(SpanText::Text {
                text: part.to_owned(),
                fx: Vec::new(),
            });
        }
    }
    LineText(spans)
}

fn record_id(id: MessageId) -> RecordId {
    RecordId(id.0.to_string())
}

/// What the game's words must say of messages, checked as the game loads.
pub(crate) fn words(words: &GameWords) -> [(&'static str, &String); 11] {
    [
        ("message_needs_title", &words.message_needs_title),
        ("only_to_friends", &words.only_to_friends),
        ("not_taking_messages", &words.not_taking_messages),
        ("inbox_full", &words.inbox_full),
        ("writing_too_fast", &words.writing_too_fast),
        ("stays_with_you", &words.stays_with_you),
        ("message_sent", &words.message_sent),
        ("message_arrived", &words.message_arrived),
        ("took_some", &words.took_some),
        ("did_not_fit_title", &words.did_not_fit_title),
        ("did_not_fit_text", &words.did_not_fit_text),
    ]
}
