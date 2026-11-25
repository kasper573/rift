use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::card::{self, Card, CardAction, CardResponse, CardTilt, Plate};
use ui::controls::{self, Pill};
use ui::egui::{self, Align, Label, Layout, Rect, Response, Ui, Vec2};
use ui::face::{self, TextRole};
use ui::glyph::{self, CardKind, Mark};
use ui::ink;
use ui::motion;

use super::{
    ActiveQuest, ObjectiveProgress, Parties, QuestDef, QuestId, QuestLog, QuestMessages,
    QuestResult, resets_label, words,
};
use crate::core::card::CardStyleDef;
use crate::core::content::Content;
use crate::core::gui::{self, Band, ImageSource, WidgetRects};
use crate::core::script::ChoiceReveal;
use crate::core::stage::card::{self as card_stage, HungCardLamp, StagedCard};
use crate::core::time::UnixMillis;
use crate::systems::chat::ChatEmbed;
use crate::systems::dialogue::windows;
use crate::systems::dialogue::{Conversation, ConversationPanel};
use crate::systems::equipment::{Belongings, Equipment};
use crate::systems::hud::{self, WindowKey};
use crate::systems::item::card::CardLook;
use crate::systems::item::grid::item_grid;
use crate::systems::item::inline::{self, InlineItem, InlineLook};
use crate::systems::item::widget::{DrawnItem, answer_drawn, draw_tile};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::UserSettings;
use crate::systems::terminal::chat;

const MARK: f32 = 13.0;

pub struct QuestCardPlugin;

impl Plugin for QuestCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<hud::Pinned<QuestId>>()
            .init_resource::<OfferSpot>()
            .add_systems(
                Update,
                hud::raise_pinned::<QuestId>(WindowKey::QuestCard)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (draw_card, draw_offer)
                    .in_set(Band::Hud)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), hud::forget_pinned::<QuestId>);
    }
}

/// Pins the card of `quest`, dealt from `from`.
pub fn open(world: &mut World, quest: QuestId, from: Option<Rect>) {
    hud::pin(world, quest, from);
}

pub fn showing(world: &World) -> bool {
    hud::pinned::<QuestId>(world).is_some()
}

pub fn close(world: &mut World) -> bool {
    hud::unpin::<QuestId>(world)
}

/// How far the pinned card is turned toward the pointer.
pub fn tilt(world: &World) -> CardTilt {
    hud::pinned_tilt(world, WindowKey::QuestCard)
}

/// The lamp the pinned card is lit by, while it is on the stage.
pub fn lamp(world: &World) -> Option<HungCardLamp> {
    hud::pinned_lamp(world, WindowKey::QuestCard)
}

/// The quest whose card is pinned, and where the card is drawn.
pub fn shown(world: &World) -> Option<(QuestId, Rect)> {
    Some((
        hud::pinned::<QuestId>(world)?,
        hud::pinned_rect::<QuestId>(world)?,
    ))
}

/// How a quest's card is shown: at which `moment` of a conversation, when at one, what `action`
/// its band offers, and how it stands among the windows.
pub struct QuestCardLook<'a> {
    pub moment: Option<QuestMoment>,
    pub action: Option<CardAction<'a>>,
    pub look: CardLook,
}

/// A quest's card: its face, and whatever `plate` writes under the overview.
pub fn show<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    quest: QuestId,
    shown_as: QuestCardLook<'_>,
    plate: impl FnOnce(&mut Ui, &mut I, Plate),
) -> CardResponse {
    let QuestCardLook {
        moment,
        action,
        look,
    } = shown_as;
    let def = quest.get(content);
    let words = words(content);
    let band = match moment {
        Some(moment) => words.band(&def.category, moment.band(words)),
        None => def.category.clone(),
    };
    let icon = images.image(def.icon(content)).map(|image| image.texture);
    let mut shown = Card::new(CardKind::Quest, &band, &def.title);
    shown.dress = CardStyleDef::of(content, def.card_style).dress(images, None);
    shown.picture = icon;
    shown.actions = action.into_iter().collect();
    shown.close = look
        .closable
        .then_some(messages::of(content).interface.close());
    shown.depth = look.depth;
    shown.show(ui, |ui, written| {
        if written.name_cut {
            card::full_name(ui, &def.title);
        }
        card::overview(ui, &def.kinds(content));
        plate(ui, images, written);
    })
}

/// The body of a quest's offer: what it asks, and what it gives when the quest lets on. Answers
/// the rewards shown, for a reader who may inspect them.
pub fn offer<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    parties: &Parties,
    quest: QuestId,
    belongings: Belongings<'_>,
    plate: Plate,
) -> Vec<DrawnItem> {
    let reader = QuestReader {
        log: &QuestLog::default(),
        belongings,
    };
    let shown = terms(ui, images, content, parties, quest, reader, plate);
    card::foot(ui, words(content).offer_foot());
    shown
}

/// Who reads a quest's card: the quests they hold and have finished, and what they carry.
#[derive(Clone, Copy)]
struct QuestReader<'a> {
    log: &'a QuestLog,
    belongings: Belongings<'a>,
}

/// What a quest asks, as far as its reader is with it, and what it gives when the quest lets
/// on. Answers the rewards shown, for a reader who may inspect them.
fn terms<I: ImageSource + ?Sized>(
    ui: &mut Ui,
    images: &mut I,
    content: &Content,
    parties: &Parties,
    quest: QuestId,
    reader: QuestReader<'_>,
    plate: Plate,
) -> Vec<DrawnItem> {
    let def = quest.get(content);
    let fresh = ActiveQuest::fresh(content, quest);
    let progress = def.progress(
        content,
        parties,
        reader.log.active(quest).unwrap_or(&fresh),
        reader.belongings,
    );
    let finished = reader.log.finished(quest).map(|finished| finished.result);
    let standing = match finished {
        Some(QuestResult::Completed) => Standing::Done,
        Some(QuestResult::Failed) => Standing::Failed,
        None => Standing::Open,
    };
    let words = words(content);
    card::overview(ui, &def.meta(content, parties, quest));
    card::heading(ui, plate, words.card_objectives());
    let mut shown = Vec::new();
    for (index, mut progress) in progress.into_iter().enumerate() {
        progress.counted &= finished.is_none();
        shown.extend(
            ui.push_id(("objective", index), |ui| {
                objective_row(ui, images, content, &progress, standing)
            })
            .inner,
        );
    }
    if def.reveals(ChoiceReveal::Gains) {
        card::heading(ui, plate, words.card_rewards());
        shown.extend(reward_chips(ui, images, content, def));
        if !def.pick_one().is_empty() && finished.is_none() {
            card::heading(ui, plate, words.card_pick_one());
            shown.extend(picks(ui, images, content, def));
        }
    }
    shown
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestMoment {
    Offer,
    TurnIn,
    Decision,
}

impl QuestMoment {
    pub fn band(self, words: &QuestMessages) -> &str {
        match self {
            QuestMoment::Offer => words.moment_offer(),
            QuestMoment::TurnIn => words.moment_turn_in(),
            QuestMoment::Decision => words.moment_decision(),
        }
    }
}

fn moment(world: &World) -> Option<(QuestId, QuestMoment)> {
    let seen = session::me(world)?;
    let panel = world.get::<Conversation>(seen)?.panel.as_ref()?;
    Some(match *panel {
        ConversationPanel::QuestOffer(quest) => (quest, QuestMoment::Offer),
        ConversationPanel::QuestTurnIn(quest) => (quest, QuestMoment::TurnIn),
        ConversationPanel::QuestDecision(quest) => (quest, QuestMoment::Decision),
    })
}

/// The card of the quest a conversation puts to the player, which they move like any card.
fn draw_offer(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(shows) = moment(world) else {
        *world.resource_mut::<OfferSpot>() = OfferSpot::None;
        return;
    };
    let (quest, moment) = shows;
    let key = WindowKey::QuestOffer;
    let placed = world
        .resource::<UserSettings>()
        .widget_pos(key.name())
        .is_some();
    let start = if placed {
        let size = hud::drawn_rect(&gui, key)
            .map_or(Vec2::new(card::WIDTH, card::HEIGHT), |rect| rect.size());
        windows::seat(world, &gui, key, size, false).min
    } else {
        offer_spot(world, &gui, shows)
    };
    let rest = hud::placed_spot(world, &gui, key, start);
    let shown = egui::Area::new(hud::panel_id(key))
        .order(Band::Hud.order())
        .fixed_pos(rest.min)
        .constrain(false)
        .fade_in(false)
        .show(&gui, |ui| show_offer(ui, world, quest, moment));
    hud::grab_spot(world, key.name(), &shown.inner, shown.response.rect);
    hud::mark_drawn(world, key);
    world
        .resource_mut::<WidgetRects>()
        .0
        .insert(key.name().to_owned(), shown.response.rect);
}

/// Where the card of an offer stands until the player places it: in sight of the people talking
/// where it can be. It is worked out once, when the card has been drawn for this offer and its size
/// is known, and kept while the offer shows, so it never moves with the camera.
fn offer_spot(world: &mut World, gui: &egui::Context, shows: (QuestId, QuestMoment)) -> egui::Pos2 {
    let key = WindowKey::QuestOffer;
    let size = |gui: &egui::Context| {
        hud::drawn_rect(gui, key).map_or(Vec2::new(card::WIDTH, card::HEIGHT), |rect| rect.size())
    };
    let spot = *world.resource::<OfferSpot>();
    match spot {
        OfferSpot::At(of, at) if of == shows => {
            // The row keeps the card's place, so the conversation's other windows stand where
            // they would beside it.
            windows::seat(world, gui, key, size(gui), true);
            at
        }
        OfferSpot::Measured(of) if of == shows => {
            let at = windows::seat_in_sight_of_bodies(world, gui, key, size(gui)).min;
            *world.resource_mut::<OfferSpot>() = OfferSpot::At(shows, at);
            at
        }
        _ => {
            *world.resource_mut::<OfferSpot>() = OfferSpot::Measured(shows);
            gui.request_discard("a quest offer's card measured before it is placed");
            windows::seat(world, gui, key, size(gui), true).min
        }
    }
}

/// How far the card of the offer on show has come to its spot: drawn once so its size is known,
/// then standing where it was worked out to.
#[derive(Resource, Default, Clone, Copy)]
enum OfferSpot {
    #[default]
    None,
    Measured((QuestId, QuestMoment)),
    At((QuestId, QuestMoment), egui::Pos2),
}

fn show_offer(ui: &mut Ui, world: &mut World, quest: QuestId, moment: QuestMoment) -> Response {
    let me = session::me(world);
    let bag = me.and_then(|me| world.get::<Inventory>(me)).cloned();
    let worn = me.and_then(|me| world.get::<Equipment>(me)).cloned();
    let belongings = Belongings::of(bag.as_ref(), worn.as_ref());
    let content = world.resource::<Content>().clone();
    let parties = world.resource::<Parties>().clone();
    let painted = ui.layer_id();
    let since = motion::mark(ui.ctx(), painted);
    let shown_as = QuestCardLook {
        moment: Some(moment),
        action: Some(chat::card_action(&messages::of(&content).terminal)),
        look: CardLook::DOCKED,
    };
    let shown = show(ui, world, &content, quest, shown_as, |ui, world, plate| {
        let shown = match moment {
            QuestMoment::Offer => offer(ui, world, &content, &parties, quest, belongings, plate),
            QuestMoment::TurnIn => turn_in(ui, world, &content, quest, belongings, plate),
            QuestMoment::Decision => {
                decision(ui, &content, quest);
                Vec::new()
            }
        };
        answer_drawn(world, shown);
    });
    chat::offered(world, shown.actions.first(), ChatEmbed::Quest(quest));
    let staged = StagedCard {
        key: ui.id().with("quest offer card"),
        runs: motion::lifted(ui.ctx(), painted, since),
        flat: shown.rect,
        pose: None,
        turns: world.resource::<UserSettings>().cards_tilt(),
        material: CardStyleDef::of(&content, quest.get(&content).card_style)
            .material
            .clone(),
    };
    card_stage::stage(world, ui.ctx(), staged, painted);
    shown.band
}

fn draw_card(world: &mut World) {
    let material = |world: &World, quest: &QuestId| {
        let content = world.resource::<Content>();
        CardStyleDef::of(content, quest.get(content).card_style)
            .material
            .clone()
    };
    hud::draw_pinned::<QuestId>(
        world,
        WindowKey::QuestCard,
        material,
        |ui, world, quest, depth| {
            let quest = *quest;
            let content = world.resource::<Content>().clone();
            let parties = world.resource::<Parties>().clone();
            let me = session::me(world);
            let log = me
                .and_then(|me| world.get::<QuestLog>(me))
                .cloned()
                .unwrap_or_default();
            let bag = me.and_then(|me| world.get::<Inventory>(me)).cloned();
            let worn = me.and_then(|me| world.get::<Equipment>(me)).cloned();
            let shown_as = QuestCardLook {
                moment: None,
                action: Some(chat::card_action(&messages::of(&content).terminal)),
                look: CardLook {
                    depth,
                    ..CardLook::PINNED
                },
            };
            let shown = show(ui, world, &content, quest, shown_as, |ui, world, plate| {
                let reader = QuestReader {
                    log: &log,
                    belongings: Belongings::of(bag.as_ref(), worn.as_ref()),
                };
                let shown = terms(ui, world, &content, &parties, quest, reader, plate);
                answer_drawn(world, shown);
            });
            chat::offered(world, shown.actions.first(), ChatEmbed::Quest(quest));
            shown
        },
    );
}

fn turn_in(
    ui: &mut Ui,
    images: &mut World,
    content: &Content,
    quest: QuestId,
    belongings: Belongings<'_>,
    plate: Plate,
) -> Vec<DrawnItem> {
    let def = quest.get(content);
    let words = words(content);
    let mut shown = Vec::new();
    if def.reveals(ChoiceReveal::Costs) {
        card::heading(ui, plate, words.card_hand_in());
        for (index, &stack) in def.hand_in().iter().enumerate() {
            let held = belongings.count(stack.item);
            shown.push(
                ui.push_id(("hand in", index), |ui| {
                    hand_in_row(ui, images, content, stack, held)
                })
                .inner,
            );
        }
    }
    if !def.reveals(ChoiceReveal::Gains) {
        return shown;
    }
    card::heading(ui, plate, words.card_rewards());
    shown.extend(reward_chips(ui, images, content, def));
    if !def.pick_one().is_empty() {
        card::foot(ui, words.pick_foot());
    }
    shown
}

fn decision(ui: &mut Ui, content: &Content, quest: QuestId) {
    let def = quest.get(content);
    let paths = def.decision().unwrap_or_default();
    ui.columns(paths.len().max(1), |columns| {
        for (column, path) in columns.iter_mut().zip(paths) {
            column.label(face::bold(&path.choice).size(TextRole::CardBody.size()));
            for mean in &path.means {
                column.horizontal_wrapped(|ui| {
                    glyph::show(ui, Mark::Dot, 8.0, ink::INK2);
                    ui.add(Label::new(TextRole::CardBody.rich(mean.as_str())).wrap());
                });
            }
        }
    });
}

fn hand_in_row(
    ui: &mut Ui,
    images: &mut World,
    content: &Content,
    stack: ItemStack,
    held: u32,
) -> DrawnItem {
    let short = held < stack.count;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if short { ink::RUBRIC } else { ink::INK3 };
            ui.label(TextRole::Small.rich(words(content).held(held)).color(color));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let item = InlineItem::from(stack);
                let look = InlineLook::new(TextRole::CardBody, ink::INK);
                DrawnItem::Inline(inline::draw(ui, images, content, item, look), item)
            })
            .inner
        })
        .inner
    })
    .inner
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    Open,
    Done,
    Failed,
}

pub(super) fn objective_row(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    progress: &ObjectiveProgress,
    standing: Standing,
) -> Vec<DrawnItem> {
    let standing = match standing {
        Standing::Open if progress.done() => Standing::Done,
        other => other,
    };
    let (mark, color) = match standing {
        Standing::Open => (Mark::Flag, ink::INK),
        Standing::Done => (Mark::Check, ink::INK),
        Standing::Failed => (Mark::Cross, ink::INK3),
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if progress.counted {
                ui.label(
                    TextRole::CardBody
                        .rich(words(content).progress(progress.have, progress.need))
                        .color(color),
                );
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                glyph::show(ui, mark, MARK, color);
                let look = InlineLook::new(TextRole::CardBody, color);
                inline::draw_line(ui, images, content, &progress.label, look)
                    .into_iter()
                    .map(|(response, item)| DrawnItem::Inline(response, item))
                    .collect()
            })
            .inner
        })
        .inner
    })
    .inner
}

pub(super) fn reward_chips(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<DrawnItem> {
    ui.horizontal_wrapped(|ui| {
        if def.xp() > 0 {
            let xp = words(content).reward_xp(def.xp());
            controls::pill(ui, &xp, Pill::Gold);
        }
        def.rewards()
            .map(|stack| item_chip(ui, images, content, stack))
            .collect()
    })
    .inner
}

pub(super) fn picks(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    def: &QuestDef,
) -> Vec<DrawnItem> {
    let picks = def.pick_one();
    let mut drawn = Vec::new();
    item_grid(ui, picks.len() as u32, |ui, place| {
        let stack = picks[place as usize];
        drawn.push(
            ui.push_id(("pick", place), |ui| draw_tile(ui, images, content, stack))
                .inner,
        );
    });
    drawn
}

pub(super) fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.add(Label::new(face::small_faint(text.into())).wrap());
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Countdown {
    #[default]
    Left,
    LeftDetail,
    Resets,
    ResetsDetail,
}

impl Countdown {
    pub(super) fn label(
        self,
        content: &Content,
        log: &QuestLog,
        quest: QuestId,
        now: UnixMillis,
    ) -> Option<String> {
        let words = words(content);
        match self {
            Countdown::Left => log
                .active(quest)?
                .left(now)
                .map(|left| messages::of(content).time.countdown(left)),
            Countdown::LeftDetail => log
                .active(quest)?
                .left(now)
                .map(|left| words.clock_left(messages::of(content).time.countdown(left))),
            Countdown::Resets => log
                .finished(quest)?
                .resets_in(now)
                .map(|left| resets_label(content, left)),
            Countdown::ResetsDetail => log
                .finished(quest)?
                .resets_in(now)
                .map(|left| words.done_today_resets(resets_label(content, left))),
        }
    }
}

pub(super) fn clock_now() -> UnixMillis {
    crate::core::time::LocalClock::of_this_machine().now
}

pub(super) fn time_left(content: &Content, log: &QuestLog, quest: QuestId, now: UnixMillis) -> f32 {
    let left = log
        .active(quest)
        .and_then(|active| active.left(now))
        .map_or(0.0, |left| left.0);
    let limit = quest.get(content).time_limit().map_or(1.0, |limit| limit.0);
    (left / limit).clamp(0.0, 1.0)
}

/// A reward, as an inline item on a gold pill.
fn item_chip(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    content: &Content,
    stack: ItemStack,
) -> DrawnItem {
    let item = InlineItem::from(stack);
    let look = InlineLook::new(TextRole::Pill, Pill::Gold.color());
    let shown = controls::pill_with(ui, Pill::Gold, |ui| {
        inline::draw(ui, images, content, item, look)
    });
    DrawnItem::Inline(shown.inner, item)
}
