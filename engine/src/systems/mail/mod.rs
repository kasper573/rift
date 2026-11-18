//! Mail: letters between players, each a title, a plain text and items attached, kept in
//! its receiver's inbox until they delete it. The system writes them too, to hand over what a
//! bag had no room for. Only a character receives one.

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
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::history::HistoryTopic;
use crate::systems::item::deliver::{self, Delivered, DeliveryReason};
use crate::systems::item::{self, ExchangeRefusal, Inventory, ItemStack};
use crate::systems::messages::{self, messages};
use crate::systems::notification::{self, Notification};
use crate::systems::player::RequestNeeds;
use crate::systems::player::snapshot::KeptAside;
use crate::systems::player::{self, PlayedBy};
use crate::systems::save::Unwritten;
use crate::systems::society::{self, AccountNumber, Settling, Society, SocietyAsk, SocietyMember};

messages! {
    /// What the game says of letters.
    pub struct MailMessages {
        /// What a player is told sending a letter that has no title.
        needs_title,
        /// What a player is told writing to someone who is neither a friend nor of their party.
        only_to_friends,
        /// What a player is told writing to `{name}`, who keeps them away.
        not_taking(name: Text),
        /// What a player is told writing to `{name}`, whose inbox has no room.
        full(name: Text),
        /// What a player is told sending more letters within a minute than the game allows.
        writing_too_fast,
        /// What a player is told as their letter to `{name}` is sent.
        sent(name: Text),
        /// What a player is told as a letter titled `{title}` arrives from `{name}`.
        arrived(name: Text, title: Text),
        /// What a player is told taking what is attached to a letter, as `{taken}` of `{attached}`
        /// stacks fit their bag.
        took_some(attached: Number, taken: Number),
        /// The title of the letter the game sends a player whose bag had no room for what they
        /// carried.
        did_not_fit_title,
        /// The text of the letter the game sends a player whose bag had no room for what they
        /// carried.
        did_not_fit_text,
        /// What became of what a bag had no room for, in a game that has letters: it is sent to its
        /// player in one.
        sent_in_a_letter,
        /// The name of the Mail page, on the Social tab.
        title,
        /// The hint, beside its key, that chooses a letter on the Mail page.
        hint_choose,
        /// The heading over the inbox: `{count}` letters held of the most it holds, `{room}`.
        inbox_heading(count: Number, room: Number),
        /// The button, and the action on a listed player, that begins a letter.
        write,
        /// What the page says while the server has not sent the inbox.
        reading_inbox,
        /// What the page says of an inbox with no letter.
        nothing_yet,
        /// What the page tells the player of who can fill an empty inbox.
        nothing_note,
        /// What a letter's row says of the `{count}` stacks attached to it.
        attached_count(count: Number),
        /// What a letter's row says of a letter not yet read.
        unread,
        /// What a letter's row says of a letter the game sent.
        system,
        /// What the open letter says of a letter the game sent.
        system_letter,
        /// What the open letter says of the player `{name}` who wrote it.
        from(name: Text),
        /// How a screen reader names a letter's row: `{title}` is its title, `{state}` what its row says.
        row_named(state: Text, title: Text),
        /// What the page says while no letter is chosen.
        choose_to_read,
        /// What the open letter says of who wrote it, `{from}`, and when, `{sent}`.
        sent_line(from: Text, sent: Text),
        /// What the open letter says while the server has not sent its text.
        opening,
        /// The heading, and the name for a screen reader, of the stacks attached to a letter.
        attached,
        /// What the open letter says of the stacks it holds of items the game lacks: `{stacks}` is how many.
        set_aside(stacks: Number),
        /// The button that answers a letter.
        reply,
        /// Why the player cannot answer a letter: `{name}` wrote it and is neither a friend nor in their party.
        cannot_reply(name: Text),
        /// Why the player cannot delete a letter that has stacks attached.
        take_first,
        /// The button that deletes a letter, and its confirmation.
        delete,
        /// The button that takes every stack attached to a letter.
        take_all,
        /// The title of the question whether to delete the letter titled `{title}`.
        delete_title(title: Text),
        /// What the question whether to delete a letter tells the player.
        delete_body,
        /// The heading over a letter being written.
        new_letter,
        /// The button that leaves the letter being written for the inbox.
        back_to_inbox,
        /// The label of the field that says who a letter is for.
        to,
        /// What the field says of who a letter is for while the player has no one to write to.
        no_receivers,
        /// What the field says of who a letter is for until one is chosen, and what sending says it needs.
        choose_receiver,
        /// What the list of the field says when no one matches what was typed.
        no_receiver_found,
        /// What the page tells a player who has no one to write to.
        receivers_note,
        /// The label of the field that holds a letter's title.
        title_label,
        /// How much of a field is written: `{count}` of the most it holds, `{most}`.
        written_of(count: Number, most: Number),
        /// The label of the field that holds a letter's text.
        text_label,
        /// How many stacks are attached, `{count}`, of the most a letter holds, `{most}`.
        attached_of(count: Number, most: Number),
        /// The button that sends a letter.
        send,
        /// Why a letter cannot be sent while it has no title.
        needs_a_title,
        /// The button that throws away the letter being written.
        discard,
        /// What a bag's slot says of a stack already attached to the letter being written.
        attached_slot,
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<InboxView>()
        .add_client_message::<SendLetter>()
        .add_client_message::<OpenLetter>()
        .add_client_message::<TakeAttached>()
        .add_client_message::<DeleteLetter>()
        .add_server_message::<LetterText>()
        .add_server_message::<LetterAnswered>();
}

/// How a game has letters work. A game without it has none.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailSettings {
    /// How many letters from players an inbox holds. Letters from the system are not counted.
    pub inbox: u32,
    /// How many stacks of items a letter carries at the most.
    pub attachments: u32,
    /// How many characters a title is long at the most.
    pub title_length: u32,
    /// How many characters a text is long at the most.
    pub text_length: u32,
    /// How many letters a player sends within a minute at the most.
    pub a_minute: u32,
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct LetterId(pub u64);

/// Who wrote a letter: the system, or a character, by the account that plays it. A letter
/// goes only to a character, so the system is never written to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LetterSender {
    #[serde(alias = "game")]
    System,
    #[serde(alias = "player")]
    Character(AccountId),
}

/// A letter in its receiver's inbox.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Letter {
    pub id: LetterId,
    pub to: AccountId,
    pub from: LetterSender,
    pub title: String,
    pub text: String,
    /// The stacks still attached. One naming an item the game no longer has waits set aside.
    #[serde(default, skip_serializing_if = "SavedList::is_empty")]
    pub attached: SavedList<ItemStack>,
    pub sent: UnixMillis,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
}

impl Record for Letter {
    // Stores already hold letters under this kind.
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
    /// How many letters from players each inbox holds, in memory or not.
    held: HashMap<AccountId, u32>,
    open: HashMap<AccountId, Inbox>,
    /// The accounts with an inbox in memory whose players do not play: each is let go of once
    /// the store holds it as memory does.
    unattended: BTreeSet<AccountId>,
    dealt: u64,
    /// When each account sent its last letters, within the minute a limit counts.
    sent: HashMap<AccountId, VecDeque<UnixMillis>>,
    reads: InboxReads,
    /// The accounts whose players see an inbox that changed since they were shown.
    changed: BTreeSet<AccountId>,
}

#[derive(Default)]
struct Inbox {
    letters: BTreeMap<LetterId, Letter>,
    /// Whether what the store holds of it has been taken in, so memory holds all of it.
    whole: bool,
    reading: bool,
    /// The letters put, changed or deleted since the store was last known to hold them as
    /// memory does.
    unwritten: BTreeSet<LetterId>,
}

impl Inboxes {
    /// The inboxes of a server whose store holds `saved`, of which only the count for each
    /// account is kept, and which reads an inbox through `reads` as its account comes to play.
    pub fn counted(saved: impl IntoIterator<Item = Letter>, reads: InboxReads) -> Inboxes {
        let mut inboxes = Inboxes {
            reads,
            ..Inboxes::default()
        };
        for letter in saved {
            inboxes.dealt = inboxes.dealt.max(letter.id.0);
            if matches!(letter.from, LetterSender::Character(_)) {
                *inboxes.held.entry(letter.to).or_default() += 1;
            }
        }
        inboxes
    }

    /// How many letters from players the inbox of `account` holds.
    pub fn held(&self, account: &AccountId) -> u32 {
        self.held.get(account).copied().unwrap_or_default()
    }

    /// The inbox of `account`, once memory holds all of it: the newest letter first.
    pub fn of(&self, account: &AccountId) -> Option<impl Iterator<Item = &Letter>> {
        let inbox = self.open.get(account).filter(|inbox| inbox.whole)?;
        Some(inbox.letters.values().rev())
    }

    /// Puts `letter` in its receiver's inbox, under an id of its own.
    fn deliver(&mut self, letter: Letter, unwritten: &mut Unwritten) {
        self.dealt += 1;
        let letter = Letter {
            id: LetterId(self.dealt),
            ..letter
        };
        if matches!(letter.from, LetterSender::Character(_)) {
            *self.held.entry(letter.to.clone()).or_default() += 1;
        }
        if !self.open.contains_key(&letter.to) {
            self.unattended.insert(letter.to.clone());
        }
        let inbox = self.open.entry(letter.to.clone()).or_default();
        inbox.unwritten.insert(letter.id);
        inbox.letters.insert(letter.id, letter.clone());
        self.changed.insert(letter.to.clone());
        unwritten.put(letter);
    }

    /// Changes the letter `id` of the inbox of `account`, once memory holds all of it: the
    /// letter as it then is.
    fn change(
        &mut self,
        account: &AccountId,
        id: LetterId,
        unwritten: &mut Unwritten,
        change: impl FnOnce(&mut Letter),
    ) -> Option<&Letter> {
        let inbox = self.open.get_mut(account).filter(|inbox| inbox.whole)?;
        let letter = inbox.letters.get_mut(&id)?;
        change(letter);
        inbox.unwritten.insert(id);
        unwritten.put(letter.clone());
        self.changed.insert(account.clone());
        Some(letter)
    }

    fn letter(&self, account: &AccountId, id: LetterId) -> Option<&Letter> {
        let inbox = self.open.get(account).filter(|inbox| inbox.whole)?;
        inbox.letters.get(&id)
    }

    fn delete(&mut self, account: &AccountId, id: LetterId, unwritten: &mut Unwritten) {
        let Some(inbox) = self.open.get_mut(account).filter(|inbox| inbox.whole) else {
            return;
        };
        let Some(letter) = inbox.letters.remove(&id) else {
            return;
        };
        inbox.unwritten.insert(id);
        unwritten.remove::<Letter>(record_id(id));
        if matches!(letter.from, LetterSender::Character(_))
            && let Some(held) = self.held.get_mut(account)
        {
            *held = held.saturating_sub(1);
        }
        self.changed.insert(account.clone());
    }

    /// Has memory hold all of the inbox of `account`, who came to play: at once where there is
    /// no store to read, else once the store has answered.
    pub(crate) fn arrived(&mut self, account: &AccountId) {
        self.unattended.remove(account);
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

    /// Notes that `account` plays no longer: memory lets go of its inbox once the store holds it
    /// as memory does.
    pub(crate) fn left(&mut self, account: &AccountId) {
        if self.open.contains_key(account) {
            self.unattended.insert(account.clone());
        }
    }

    /// Takes in what the store answered, and lets go of the inboxes of accounts that do not
    /// play once the store holds them as memory does. An inbox the store failed to answer for
    /// is asked for again while its account plays.
    pub(crate) fn keep(&mut self, unwritten: &Unwritten) {
        for read in self.reads.finished() {
            let Some(inbox) = self.open.get_mut(&read.account) else {
                continue;
            };
            inbox.reading = false;
            match read.letters {
                Ok(stored) => {
                    for letter in stored {
                        // Memory holds a letter newer than the store does, or as the store does.
                        if !inbox.unwritten.contains(&letter.id) {
                            inbox.letters.entry(letter.id).or_insert(letter);
                        }
                    }
                    inbox.whole = true;
                    self.changed.insert(read.account);
                }
                Err(_) if !self.unattended.contains(&read.account) => self.arrived(&read.account),
                Err(_) => {}
            }
        }
        let open = &mut self.open;
        self.unattended.retain(|account| {
            let written = open.get_mut(account).is_none_or(|inbox| {
                inbox
                    .unwritten
                    .retain(|&id| unwritten.holds(&(Letter::KIND, record_id(id))));
                inbox.unwritten.is_empty()
            });
            if written {
                open.remove(account);
            }
            !written
        });
    }

    pub(crate) fn changed(&mut self) -> BTreeSet<AccountId> {
        std::mem::take(&mut self.changed)
    }

    /// Whether `account` has sent as many letters within the last minute as `most`.
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

/// What a player asks of their letters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxAsk {
    Send(SendLetter),
    Open(LetterId),
    Take(TakeAttached),
    Delete(LetterId),
    /// That what their bag had no room for as they came back be sent to them.
    Overflow,
}

/// A player sending a letter, with stacks of their bag attached.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SendLetter {
    pub to: AccountNumber,
    pub title: String,
    pub text: String,
    pub attached: Vec<ItemStack>,
}

/// A player opening a letter, which marks it read and has the server send its text.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenLetter(pub LetterId);

/// A player taking what is attached to a letter: some of one stack, or every stack.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeAttached {
    pub letter: LetterId,
    pub stack: Option<TakenStack>,
}

/// `count` of the stack at `index` of what is attached to a letter.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakenStack {
    pub index: u32,
    pub count: u32,
}

/// A player deleting a letter nothing is attached to.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteLetter(pub LetterId);

/// The text of a letter its receiver opened.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LetterText {
    pub id: LetterId,
    pub text: String,
}

/// Whether the letter a player sent was taken.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LetterAnswered {
    pub sent: bool,
}

/// A player's inbox as their client knows it: every letter but its text, the newest first.
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
    pub id: LetterId,
    pub from: InboxSender,
    pub title: String,
    pub sent: UnixMillis,
    pub read: bool,
    pub attached: Vec<ItemStack>,
    /// How many more stacks are attached that name items the game no longer has.
    pub set_aside: u32,
}

/// Who wrote a letter, as its receiver sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum InboxSender {
    System,
    Character(SocietyMember),
}

impl InboxView {
    pub fn unread(&self) -> u32 {
        self.rows.iter().filter(|row| !row.read).count() as u32
    }
}

/// Hands what players of this world asked of their letters to society.
pub fn requests(world: &mut World) {
    fn hand<M: Message>(world: &mut World, ask: fn(M) -> InboxAsk) {
        player::player_requests::<M>(
            world,
            RequestNeeds::Nothing,
            move |world, player, letter| {
                if let Some(by) = world.get::<PlayedBy>(player) {
                    society::ask(world, by.client, SocietyAsk::Inbox(ask(letter)));
                }
                Ok(())
            },
        );
    }
    hand::<SendLetter>(world, InboxAsk::Send);
    hand::<OpenLetter>(world, |open| InboxAsk::Open(open.0));
    hand::<TakeAttached>(world, InboxAsk::Take);
    hand::<DeleteLetter>(world, |delete| InboxAsk::Delete(delete.0));
}

/// Asks that what a character's bag had no room for as it came back be sent to its player, in
/// a game that has letters.
pub fn overflow(
    world: &mut World,
    kept: &mut QueryState<(&KeptAside, &PlayedBy), Changed<KeptAside>>,
) {
    if world.resource::<GameSettings>().mail.is_none() {
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
    if world.resource::<GameSettings>().mail.is_none() {
        return;
    }
    let inboxes = society.inboxes();
    let view = match inboxes.of(account) {
        Some(letters) => InboxView {
            whole: true,
            rows: letters.filter_map(|letter| row(society, letter)).collect(),
            held: inboxes.held(account),
        },
        None => InboxView::default(),
    };
    if world.get::<InboxView>(character) != Some(&view) {
        world.entity_mut(character).insert(view);
    }
}

/// Sends `to` a letter from the system with `stacks` attached, whether they play or not.
pub fn from_system(
    society: &mut Society,
    unwritten: &mut Unwritten,
    now: UnixMillis,
    to: &AccountId,
    said: (&str, &str),
    stacks: impl Into<SavedList<ItemStack>>,
) {
    let letter = Letter {
        id: LetterId(0),
        to: to.clone(),
        from: LetterSender::System,
        title: said.0.to_owned(),
        text: said.1.to_owned(),
        attached: stacks.into(),
        sent: now,
        read: false,
    };
    society.inboxes.deliver(letter, unwritten);
}

/// Whether `by` may write to `to`: only to a friend or a member of their party.
pub fn may_write(society: &Society, by: &AccountId, to: &AccountId) -> bool {
    by != to && society.close(by, to)
}

/// Does what `by` asked of their letters, under the game `content` is of.
pub(crate) fn settle(settling: &mut Settling, content: &Content, by: &AccountId, ask: InboxAsk) {
    let Some(rules) = &content.built::<GameSettings>().mail else {
        return;
    };
    match ask {
        InboxAsk::Send(letter) => {
            let sent = send(settling, content, rules, by, letter);
            settling.send(by, LetterAnswered { sent });
        }
        InboxAsk::Open(id) => {
            let inboxes = &mut settling.society.inboxes;
            let opened = inboxes.change(by, id, settling.unwritten, |letter| letter.read = true);
            if let Some(text) = opened.map(|letter| letter.text.clone()) {
                settling.send(by, LetterText { id, text });
            }
        }
        InboxAsk::Take(take) => self::take(settling, content, by, take),
        InboxAsk::Delete(id) => {
            let inboxes = &mut settling.society.inboxes;
            if inboxes
                .letter(by, id)
                .is_some_and(|letter| letter.attached.is_empty())
            {
                inboxes.delete(by, id, settling.unwritten);
            }
        }
        InboxAsk::Overflow => {
            let Some((world, character)) = settling.reach(by) else {
                return;
            };
            let stacks = world
                .get_mut::<KeptAside>(character)
                .map(|mut kept| kept.take_overflow(content))
                .unwrap_or_default();
            if stacks.is_empty() {
                return;
            }
            deliver::to_account(settling, content, by, &stacks, DeliveryReason::KeptAside);
        }
    }
}

/// Sends the letter `by` wrote: whether it was. Everything is checked before a stack leaves the
/// sender's bag, and from there to the receiver's inbox nothing can fail.
fn send(
    settling: &mut Settling,
    content: &Content,
    rules: &MailSettings,
    by: &AccountId,
    letter: SendLetter,
) -> bool {
    let Some(to) = settling.society.accounts().numbered(letter.to).cloned() else {
        return false;
    };
    let messages = &messages::of(content).mail;
    let name = settling.name(&to);
    let society = &mut *settling.society;
    let title = letter.title.trim();
    let fits = title.chars().count() <= rules.title_length as usize
        && letter.text.chars().count() <= rules.text_length as usize
        && letter.attached.len() <= rules.attachments as usize
        && letter.attached.iter().all(|stack| stack.count > 0);
    let refusal = if !fits {
        return false;
    } else if title.is_empty() {
        Some(messages.needs_title().to_owned())
    } else if !may_write(society, by, &to) {
        Some(messages.only_to_friends().to_owned())
    } else if society.blocks().blocks(&to, by) {
        Some(messages.not_taking(&name))
    } else if society.inboxes.held(&to) >= rules.inbox {
        Some(messages.full(&name))
    } else if society
        .inboxes
        .writes_too_fast(by, rules.a_minute, settling.now)
    {
        Some(messages.writing_too_fast().to_owned())
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
    if let Err(why) = item::effects::give_away(world, character, &letter.attached) {
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
    let letter = Letter {
        id: LetterId(0),
        to: to.clone(),
        from: LetterSender::Character(by.clone()),
        title: title.to_owned(),
        text: letter.text,
        attached: letter.attached.into(),
        sent: settling.now,
        read: false,
    };
    society.inboxes.deliver(letter, settling.unwritten);
    let sent = messages.sent(&name);
    settling.tell(by, Notification::feed(HistoryTopic::Notification, sent));
    let arrived = messages.arrived(&sender, title);
    settling.tell(&to, Notification::feed(HistoryTopic::Notification, arrived));
    true
}

/// Hands `by` what is attached to a letter of theirs, as much of it as their bag has room for:
/// each stack is tried on its own, and what does not fit stays attached.
fn take(settling: &mut Settling, content: &Content, by: &AccountId, take: TakeAttached) {
    let Some(letter) = settling.society.inboxes.letter(by, take.letter) else {
        return;
    };
    let attached: Vec<ItemStack> = letter.attached.to_vec();
    let (wanted, others): (Vec<ItemStack>, Vec<ItemStack>) = match take.stack {
        None => (attached, Vec::new()),
        Some(part) => {
            let mut others = attached;
            let Some(stack) = others.get_mut(part.index as usize) else {
                return;
            };
            let taken = part.count.min(stack.count);
            stack.count -= taken;
            (vec![ItemStack::new(stack.item, taken)], others)
        }
    };
    if wanted.iter().all(|stack| stack.count == 0) {
        return;
    }
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let Some(mut bag) = world.get::<Inventory>(character).cloned() else {
        return;
    };
    let Delivered { kept, sent: left } = bag.give_what_fits(content, &wanted);
    let taken = kept.len();
    if taken > 0 {
        world.entity_mut(character).insert(bag);
    }
    if !left.is_empty() {
        let why = if taken == 0 {
            ExchangeRefusal::NoRoom { slots: 1 }.line(content)
        } else {
            messages::of(content)
                .mail
                .took_some(wanted.len(), taken)
                .into()
        };
        notification::error(world, character, why);
    }
    if taken == 0 {
        return;
    }
    let mut still: Vec<ItemStack> = match take.stack {
        None => left,
        Some(part) => {
            let mut still = others;
            if let Some(stack) = still.get_mut(part.index as usize) {
                stack.count += left.iter().map(|stack| stack.count).sum::<u32>();
            }
            still.retain(|stack| stack.count > 0);
            still
        }
    };
    settling
        .society
        .inboxes
        .change(by, take.letter, settling.unwritten, |letter| {
            letter.attached.clear();
            letter.attached.append(&mut still);
        });
}

/// The row of `letter`, where the society knows who wrote it.
fn row(society: &Society, letter: &Letter) -> Option<InboxRow> {
    Some(InboxRow {
        id: letter.id,
        from: match &letter.from {
            LetterSender::Character(account) => InboxSender::Character(society.member(account)?),
            LetterSender::System => InboxSender::System,
        },
        title: letter.title.clone(),
        sent: letter.sent,
        read: letter.read,
        attached: letter.attached.to_vec(),
        set_aside: letter.attached.set_aside().len() as u32,
    })
}

fn record_id(id: LetterId) -> RecordId {
    RecordId(id.0.to_string())
}
