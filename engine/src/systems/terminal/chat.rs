use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use bevy::prelude::*;
use bevy_terminal::{
    COMMAND_PREFIX, EMBED, LineKind, MAX_INPUT_LEN, TerminalCompletionRequest, TerminalCompletions,
    TerminalInput, TerminalLine, best_matches, command_line,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::card::CardAction;
use ui::egui;
use ui::egui::containers::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use ui::egui::text::LayoutJob;
use ui::egui::{
    Align, Align2, Color32, CursorIcon, FontId, Galley, Id, Layout, Order, Pos2, Rect, Sense,
    Stroke, TextFormat, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
};
use ui::face::TextRole;
use ui::glyph::Mark;
use ui::ink::{self, Ground};
use ui::panel::GripCorner;
use ui::suggest::{self, Suggestion};
use ui::{dusk, input};

use super::TerminalMessages;
use super::draft::ChatDraft;
use super::history::{InField, SentLines};
use super::{ChatSpeakers, LoggedLineId, TerminalLog};
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band, Tier};
use crate::core::math::Rgb;
use crate::core::text::SpanDef;
use crate::core::text::{self, LinkKey, StyledLine, StyledSpan, TextLook, TextPiece};
use crate::systems::GameSettings;
use crate::systems::actor::portrait::ActorPortraits;
use crate::systems::area_map::{self, MapPin, MapSight};
use crate::systems::chat::{
    ChatChannel, ChatChannels, ChatEmbed, ChatEmbedKind, ChatLocation, EMBED_ASK,
};
use crate::systems::command::search;
use crate::systems::hud::{self, SizedPiece};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::interface::InterfaceSound;
use crate::systems::item::ItemId;
use crate::systems::item::inline::{self, InlineItem};
use crate::systems::mention::{Mention, SuggestedMention};
use crate::systems::messages::{self, MessagePart};
use crate::systems::player::{ClientId, session};
use crate::systems::settings::{Placed, UserSettings};

pub const CHAT: &str = "chat";

const START: Vec2 = Vec2::new(310.0, 186.0);
/// Two lines and the row they are typed in.
const SMALLEST: Vec2 = Vec2::new(220.0, 96.0);
const BAR: f32 = 12.0;
const TABS: f32 = 22.0;
const TAB_PAD: f32 = 8.0;
const TAB_GAP: f32 = 4.0;
/// How far a speaker's portrait stands from their name.
const PORTRAIT_GAP: f32 = 4.0;
const ROW: f32 = 28.0;
const PAD: f32 = 10.0;
const LINE_GAP: f32 = 2.0;
const ROW_GAP: f32 = 4.0;
/// Kept free of lines whether or not the bar shows, so a line wraps alike at rest and focused.
const SCROLL_BAR: f32 = 10.0;
const GRIP: f32 = 14.0;
const HANDLE: Vec2 = Vec2::new(32.0, 2.0);
const PILL: Vec2 = Vec2::new(72.0, 20.0);
const PAGE_SHARE_OF_LOG: f32 = 0.8;
const ROOM_TO_TYPE_BESIDE_HINTS: f32 = 150.0;
const ROW_WASH: f32 = 0.55;
/// How far the icon of a thing a line holds stands from its name.
const ICON_GAP: f32 = 3.0;
/// How wide the name of a thing being looked for is typed.
const SEARCH_WIDTH: f32 = 110.0;
/// The least room the name looked for is typed in, where the line leaves little.
const SEARCH_LEAST: f32 = 60.0;

#[derive(Resource, Default)]
pub(crate) struct ChatView {
    draft: ChatDraft,
    /// The thing being looked for to set in the draft, once its kind is picked.
    seeking: Option<EmbedSearch>,
    /// The draft the server was last asked about, and whether its answer is still out: one
    /// question is out at a time, so typing never asks faster than the server answers.
    asked: Option<String>,
    waiting: bool,
    suggested: Option<TerminalCompletions>,
    selected: usize,
    typing: bool,
    /// Whether an input method is putting a character together, whose confirming key sends
    /// nothing.
    composing: bool,
    /// The channels the server last said the player is in: a tab each.
    channels: Vec<ChatChannel>,
    /// The tab shown, which what is typed goes to.
    shown: ChatChannel,
    reading_from: HashMap<ChatChannel, u64>,
    catching_up: bool,
    was_typing: bool,
    /// The covered things the player asked to see: which line holds each, and where in it.
    revealed: HashSet<(LoggedLineId, usize)>,
    /// The lines sent this session, which the recall keys bring back.
    sent: SentLines,
}

impl ChatView {
    /// What may be taken for the draft as it stands: an answer to an older draft is listed but
    /// offers nothing to take, as taking one of its options would undo what was typed since.
    fn current(&self) -> Option<&TerminalCompletions> {
        self.suggested
            .as_ref()
            .filter(|suggested| suggested.text == self.draft.text())
    }
}

/// A search for a thing to set in the draft before its letter `at`.
struct EmbedSearch {
    kind: ChatEmbedKind,
    at: usize,
    name: String,
}

/// Sets `embed` at the end of what the player is typing into chat, which neither sends it nor
/// gives chat the keyboard: a line can gather many things before it is sent.
pub fn add(world: &mut World, embed: ChatEmbed) {
    let content = world.resource::<Content>().clone();
    let name = embed.shown(&content).name;
    let Some(mut view) = world.get_resource_mut::<ChatView>() else {
        return;
    };
    if view.draft.text().chars().count() + name.chars().count() >= MAX_INPUT_LEN {
        return;
    }
    let caret = view.draft.add(embed, &name);
    view.sent.stop();
    if let Some(gui) = gui::context(world) {
        input::caret_to(&gui, field_id(), caret);
    }
}

/// Whether chat takes what the player sets in it: only while it shows.
pub fn takes_embeds(world: &World) -> bool {
    world.resource::<UserSettings>().chat_shown()
}

/// The offer a card makes in its band to set what it shows in chat.
pub fn card_action(words: &TerminalMessages) -> CardAction<'_> {
    CardAction {
        mark: Mark::Chat,
        label: words.add_to_chat(),
    }
}

/// Answers the offer a card made with the button `offer`: names it to a pointer resting on it,
/// and sets `embed` in chat once it is taken.
pub fn offered(world: &mut World, offer: Option<&egui::Response>, embed: ChatEmbed) {
    let Some(offer) = offer else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).terminal;
    hud::tooltip(offer, world, words.add_to_chat(), &[], None);
    if offer.clicked() {
        add(world, embed);
    }
}

/// Sets `embeds` in chat once the player asks for it on `response`, which shows them and must
/// sense clicks.
pub fn shortcut(
    world: &mut World,
    response: &egui::Response,
    embeds: impl IntoIterator<Item = ChatEmbed>,
) {
    if takes_embeds(world) && map::clicked(world, InputAction::AddToChat, response) {
        for embed in embeds {
            add(world, embed);
        }
    }
}

/// Gives the keyboard back from chat: from a line sent earlier to the one being typed first,
/// then from the search for a thing to the line it is set in, then from the line.
pub fn hand_back(world: &mut World) -> bool {
    let typed = world.resource_mut::<ChatView>().sent.give_back();
    if let Some(typed) = typed {
        world.resource_mut::<ChatView>().draft = typed.draft;
        if let Some(gui) = gui::context(world) {
            input::caret_to(&gui, field_id(), typed.caret);
        }
        return true;
    }
    if world.resource_mut::<ChatView>().seeking.take().is_some() {
        return true;
    }
    let had = std::mem::take(&mut world.resource_mut::<ChatView>().typing);
    if let Some(gui) = gui::context(world) {
        gui.memory_mut(|memory| memory.surrender_focus(field_id()));
    }
    had
}

pub(super) fn suggested(
    mut answers: MessageReader<TerminalCompletions>,
    mut view: ResMut<ChatView>,
) {
    for answer in answers.read() {
        view.waiting = false;
        // An answer to a draft typed on since is listed until the answer to what was typed
        // since arrives, so the list stays up as the player types.
        if view.draft.text().trim_start().starts_with(COMMAND_PREFIX) {
            view.selected = 0;
            view.suggested = Some(answer.clone());
        }
    }
}

/// Keeps a tab for each channel the server says the player is in. A channel that is gone takes
/// its lines with it, and chat shows its first tab when the one shown went.
pub(super) fn channels(world: &mut World) {
    let Some(told) = session::my_character(world)
        .and_then(|me| me.get::<ChatChannels>().cloned())
        .filter(|told| told.0 != world.resource::<ChatView>().channels)
    else {
        return;
    };
    let gone: Vec<ChatChannel> = world
        .resource::<ChatView>()
        .channels
        .iter()
        .copied()
        .filter(|channel| !told.0.contains(channel))
        .collect();
    for channel in &gone {
        world
            .resource_mut::<TerminalLog>()
            .forget(channel.terminal());
    }
    let mut view = world.resource_mut::<ChatView>();
    for channel in &gone {
        view.reading_from.remove(channel);
    }
    if !told.0.contains(&view.shown) {
        view.shown = told.0.first().copied().unwrap_or_default();
    }
    view.channels = told.0;
}

/// Lets go of the lines sent, as the player leaves the game.
pub(super) fn forget(mut view: ResMut<ChatView>) {
    view.sent = SentLines::default();
}

/// What chat shows of its channels, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownChat {
    /// The tab shown, which what is typed goes to.
    pub shown: ChatChannel,
    /// A tab for each channel the player is in, in the order chat lists them.
    pub tabs: Vec<ShownChatTab>,
    /// What the field holds, the things it holds by their names: what is typed and not yet
    /// sent, or a line sent earlier that the player brought back.
    pub draft: String,
    /// Where among the lines sent earlier the field is, as line `at` back of `of`, while the
    /// player has brought one back.
    pub recalled: Option<ShownRecall>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ShownRecall {
    pub at: u32,
    pub of: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownChatTab {
    pub channel: ChatChannel,
    pub name: String,
}

pub fn shown(world: &World) -> ShownChat {
    let view = world.resource::<ChatView>();
    let content = world.resource::<Content>();
    ShownChat {
        draft: view.draft.text().to_owned(),
        recalled: view.sent.place().map(|(at, of)| ShownRecall { at, of }),
        shown: view.shown,
        tabs: view
            .channels
            .iter()
            .map(|&channel| ShownChatTab {
                channel,
                name: channel.id(content).get(content).name.clone(),
            })
            .collect(),
    }
}

pub(super) fn keys(world: &mut World) {
    let shown = world.resource::<UserSettings>().chat_shown();
    if shown && map::just_pressed(world, InputAction::FocusChat) {
        world.resource_mut::<ChatView>().typing = true;
    }
    let typing = world.resource::<ChatView>().typing;
    if typing && (!shown || session::is_dead(world)) {
        hand_back(world);
    }
}

pub(super) fn draw(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let settings = world.resource::<UserSettings>();
    if !settings.chat_shown() {
        return;
    }
    let background = settings.chat_background().0 / 100.0;
    let font = FontId::new(settings.chat_text_size().0, TextRole::Body.font_id().family);
    let covered = ChatEmbedKind::VARIANTS
        .iter()
        .copied()
        .filter(|&kind| !settings.shows_chat_embeds(kind))
        .collect();
    let mut look = ChatLook {
        background,
        font,
        embeds: EmbedLooks::default(),
        covered,
        channels: ChatChannel::VARIANTS
            .iter()
            .map(|&channel| ChannelLook::of(world, channel))
            .collect(),
    };
    let content = world.resource::<Content>().clone();
    let terminal = &messages::of(&content).terminal;
    let screen = gui.content_rect();
    let piece = SizedPiece {
        key: CHAT,
        of: Placed::Chat,
        start: Rect::from_min_size(
            Pos2::new(
                screen.left() + hud::MARGIN,
                screen.bottom() - hud::MARGIN - START.y,
            ),
            START,
        ),
        min: SMALLEST,
        max: screen.size() / 2.0,
        corner: GripCorner::TopRight,
    };
    let rect = hud::sized_rect(world, &gui, &piece);
    let mut view = std::mem::take(&mut *world.resource_mut::<ChatView>());
    look.embeds = EmbedLooks::of(world, &view);
    let typing = view.typing;
    let order = if typing {
        Band::Hud.order()
    } else {
        Order::Background
    };
    let shown = egui::Area::new(Id::new(("hud piece", CHAT)))
        .order(order)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .sense(if typing {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        })
        .show(&gui, |ui| {
            let (rect, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            plate(ui, rect, typing, look.background);
            let handles = typing.then(|| handles(ui, rect, terminal));
            let row = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - ROW), rect.max);
            let strip = Rect::from_min_max(
                Pos2::new(rect.left(), rect.top() + BAR),
                Pos2::new(rect.right(), rect.top() + BAR + TABS),
            );
            // The lines stand on the row they are typed in, so the tabs take room from the oldest
            // of them and move none.
            let top = if typing { strip.bottom() } else { strip.top() };
            let log = Rect::from_min_max(
                Pos2::new(rect.left() + PAD, top),
                Pos2::new(rect.right(), row.top() - ROW_GAP),
            );
            if typing {
                let was = view.shown;
                tabs(ui, world, &mut view, strip, &look);
                if view.shown != was {
                    InterfaceSound::UiPage.play(world);
                }
            }
            if let Some(asked) = lines(ui, world, &mut view, log, &look) {
                inspect(world, asked);
            }
            field_row(ui, world, &mut view, row, &look);
            handles
        });
    // While it is typed in, chat lies over every window, however lately one was clicked.
    if typing {
        gui.move_to_top(shown.response.layer_id);
    } else {
        gui::stack(&gui, Tier::Mid, shown.response.layer_id);
    }
    if let Some((mover, sizer)) = shown.inner {
        hud::grab_sized(world, &piece, rect, &mover, &sizer);
    }
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(CHAT.to_owned(), rect);
    ask(world, &mut view);
    *world.resource_mut::<ChatView>() = view;
}

struct ChatLook {
    background: f32,
    font: FontId,
    embeds: EmbedLooks,
    /// The kinds of thing the player has chat cover in its lines.
    covered: Vec<ChatEmbedKind>,
    /// How each channel the engine lists is told from the others, in their order.
    channels: Vec<ChannelLook>,
}

impl ChatLook {
    fn of(&self, channel: ChatChannel) -> Option<&ChannelLook> {
        self.channels.iter().find(|look| look.channel == channel)
    }
}

/// How each thing held by what is being typed looks this frame: in the field, an item keeps the
/// colour and icon of its inline item, as any other thing those of its name in a line.
#[derive(Default)]
struct EmbedLooks(HashMap<ChatEmbed, EmbedLook>);

struct EmbedLook {
    color: Color32,
    /// The thing's icon, once it is loaded.
    icon: Option<egui::TextureId>,
}

impl EmbedLooks {
    fn of(world: &mut World, view: &ChatView) -> EmbedLooks {
        let content = world.resource::<Content>().clone();
        let held: Vec<ChatEmbed> = view.draft.held().map(|(_, embed)| embed).collect();
        let mut looks = HashMap::new();
        for embed in held {
            if looks.contains_key(&embed) {
                continue;
            }
            let shown = embed.shown(&content);
            let Rgb([r, g, b]) = shown.color;
            let look = EmbedLook {
                color: Ground::Dusk.legible(Color32::from_rgb(r, g, b)),
                icon: gui::image(world, &shown.icon).map(|image| image.texture),
            };
            looks.insert(embed, look);
        }
        EmbedLooks(looks)
    }
}

/// Which letter of a laid-out line the name of a thing it holds starts at, and the thing's
/// icon to draw before it.
type EmbedIcon = (usize, egui::TextureId);

/// Draws the icon of each thing the line laid out as `galley` at `origin` holds, in the room
/// kept before its name.
fn embed_icons(painter: &egui::Painter, origin: Pos2, galley: &Galley, icons: &[EmbedIcon]) {
    for &(letter, icon) in icons {
        let Some(first) = letter_rect(galley, letter) else {
            continue;
        };
        let side = first.height();
        let at = Rect::from_min_size(
            origin + Vec2::new(first.left() - ICON_GAP - side, first.top()),
            Vec2::splat(side),
        );
        let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(icon, at, whole, Color32::WHITE);
    }
}

/// Where the letters `letters` of the text laid out as `galley` stand in it: a rect for each
/// row they run over.
fn letters_rects(galley: &Galley, letters: Range<usize>) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for at in letters.filter_map(|letter| letter_rect(galley, letter)) {
        match rects.last_mut() {
            Some(last) if last.top() == at.top() => *last = last.union(at),
            _ => rects.push(at),
        }
    }
    rects
}

/// Where letter `letter` of the text laid out as `galley` stands in it, as tall as its row.
fn letter_rect(galley: &Galley, letter: usize) -> Option<Rect> {
    let mut passed = 0;
    for placed in &galley.rows {
        let glyphs = &placed.row.glyphs;
        if let Some(glyph) = letter.checked_sub(passed).and_then(|at| glyphs.get(at)) {
            return Some(Rect::from_min_size(
                Pos2::new(placed.pos.x + glyph.pos.x, placed.pos.y),
                Vec2::new(glyph.advance_width, placed.row.size.y),
            ));
        }
        passed += glyphs.len() + usize::from(placed.ends_with_newline);
    }
    None
}

struct ChannelLook {
    channel: ChatChannel,
    name: String,
    color: Color32,
}

impl ChannelLook {
    fn of(world: &World, channel: ChatChannel) -> ChannelLook {
        let content = world.resource::<Content>();
        let def = channel.id(content).get(content);
        let [r, g, b] = def.color.0;
        ChannelLook {
            channel,
            name: def.name.clone(),
            color: ink::legible(Color32::from_rgb(r, g, b), dusk::FILL),
        }
    }
}

/// A tab for each channel the player is in, the one shown underlined.
fn tabs(ui: &mut Ui, world: &World, view: &mut ChatView, strip: Rect, look: &ChatLook) {
    let keys = world.resource::<InputMap>();
    let pressed = |action| {
        keys.shortcut(action)
            .is_some_and(|key| ui.input_mut(|input| input.consume_shortcut(&key)))
    };
    // A shortcut held with more modifiers than another's is asked first, as the lesser one
    // would take its press.
    let back = pressed(InputAction::PreviousChatTab);
    let on = !back && pressed(InputAction::NextChatTab);
    let place = view.channels.iter().position(|tab| *tab == view.shown);
    if let Some(place) = place.filter(|_| back || on) {
        let tabs = view.channels.len();
        let turned = if back { place + tabs - 1 } else { place + 1 };
        view.shown = view.channels[turned % tabs];
    }
    let mut left = strip.left() + PAD - TAB_PAD;
    for channel in view.channels.clone() {
        let Some(channel_look) = look.of(channel) else {
            continue;
        };
        let selected = channel == view.shown;
        let ink = if selected {
            dusk::TEXT
        } else {
            dusk::TEXT.gamma_multiply(0.65)
        };
        let name = ui
            .painter()
            .layout_no_wrap(channel_look.name.clone(), look.font.clone(), ink);
        let rect = Rect::from_min_size(
            Pos2::new(left, strip.top()),
            Vec2::new(name.size().x + 2.0 * TAB_PAD, strip.height()),
        );
        let words = &messages::of(world.resource::<Content>()).terminal;
        let label = words.chat_tab(&channel_look.name);
        let response = ui
            .interact(rect, Id::new(("chat tab", channel)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &label)
        });
        if response.clicked() {
            view.shown = channel;
        }
        let at = Pos2::new(rect.left() + TAB_PAD, rect.center().y - name.size().y / 2.0);
        ui.painter().galley(at, name, ink);
        if selected {
            ui.painter().hline(
                rect.shrink2(Vec2::new(TAB_PAD / 2.0, 0.0)).x_range(),
                strip.bottom() - 1.0,
                Stroke::new(2.0, dusk::TEXT),
            );
        }
        left = rect.right() + TAB_GAP;
    }
    ui.painter().hline(
        strip.x_range(),
        strip.bottom(),
        Stroke::new(1.0, dusk::BORDER),
    );
}

fn field_id() -> Id {
    Id::new("chat field")
}

fn plate(ui: &Ui, rect: Rect, typing: bool, background: f32) {
    if !typing {
        dusk::plate(ui.painter(), rect, background);
        return;
    }
    ui.painter().add(dusk::SHADOW.as_shape(rect, dusk::RADIUS));
    dusk::plate(ui.painter(), rect, 1.0);
    ui.painter().rect_stroke(
        rect,
        dusk::RADIUS,
        Stroke::new(1.0, dusk::BORDER),
        egui::StrokeKind::Inside,
    );
}

fn handles(ui: &mut Ui, rect: Rect, words: &TerminalMessages) -> (egui::Response, egui::Response) {
    let id = Id::new(("hud piece", CHAT));
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + BAR));
    let mover = ui
        .interact(bar, id.with("bar"), Sense::drag())
        .on_hover_cursor(CursorIcon::Grab);
    mover.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, words.move_chat()));
    let faint = dusk::TEXT.gamma_multiply(0.45);
    ui.painter()
        .rect_filled(Rect::from_center_size(bar.center(), HANDLE), 1.0, faint);
    ui.painter()
        .hline(rect.x_range(), bar.bottom(), Stroke::new(1.0, dusk::BORDER));
    let corner = Rect::from_min_max(
        Pos2::new(rect.right() - GRIP, rect.top()),
        Pos2::new(rect.right(), rect.top() + GRIP),
    );
    let sizer = ui
        .interact(corner, id.with("grip"), Sense::drag())
        .on_hover_cursor(CursorIcon::ResizeNeSw);
    sizer.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, words.resize_chat()));
    let ink = if sizer.hovered() || sizer.dragged() {
        dusk::TEXT
    } else {
        faint
    };
    let inset = Vec2::splat(2.0);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            corner.left_top() + Vec2::new(inset.x, inset.y),
            corner.right_top() + Vec2::new(-inset.x, inset.y),
            corner.right_bottom() - inset,
        ],
        ink,
        Stroke::NONE,
    ));
    (mover, sizer)
}

/// The lines of the tab shown while chat is typed in, and of every channel together at rest.
/// Answers the thing of a line the player asked to see.
fn lines(
    ui: &mut Ui,
    world: &mut World,
    view: &mut ChatView,
    room: Rect,
    look: &ChatLook,
) -> Option<AskedEmbed> {
    let me = session::my_id(world).map(|client| u64::from(client.0));
    let width = room.width() - SCROLL_BAR - PAD;
    let typing = view.typing;
    let shown = view.shown;
    let together = !typing && view.channels.len() > 1;
    let row_tall = ui.fonts_mut(|fonts| fonts.row_height(&look.font));
    let text_look = TextLook::new(look.font.size, dusk::TEXT)
        .on(Ground::Dusk)
        .shadowed(dusk::SMALL_TEXT_SHADOW);
    let content = world.resource::<Content>().clone();
    let log = world.resource::<TerminalLog>();
    view.revealed
        .retain(|(line, _)| log.numbered().any(|(kept, _)| kept == *line));
    let laid: Vec<LaidLine> = {
        let speakers = world.resource::<ChatSpeakers>();
        let portraits = world.resource::<ActorPortraits>();
        let revealed = &view.revealed;
        let of_tab = |line: &TerminalLine| !typing || line.channel == shown.terminal();
        log.numbered()
            .filter(|(_, line)| of_tab(line))
            .map(|(id, line)| {
                let said_on = ChatChannel::of(line.channel)
                    .and_then(|on| look.of(on))
                    .filter(|_| together);
                let speaker = typist(line).and_then(|typist| speakers.of(typist));
                let pictured = speaker.filter(|speaker| portraits.has(&speaker.appearance));
                let said = SaidLook {
                    on: said_on,
                    name_color: speaker.map(|speaker| speaker.name_color),
                    portrait: pictured.map(|_| row_tall),
                };
                let covers = |place: usize, kind: ChatEmbedKind| {
                    look.covered.contains(&kind) && !revealed.contains(&(id, place))
                };
                let (styled, embeds) = said_line(line, &said, &content, &covers);
                let portrait = pictured.and_then(|speaker| portraits.of(&speaker.appearance));
                let tall = text::measure(ui.ctx(), world, &styled, text_look, width).y;
                LaidLine {
                    id,
                    styled,
                    tall,
                    portrait,
                    embeds,
                }
            })
            .collect()
    };
    let tall: f32 = laid.iter().map(|line| line.tall + LINE_GAP).sum();
    let mut uncovered = Vec::new();
    let mut asked_for = None;
    let (source, bar) = if typing {
        (
            ScrollSource {
                scroll_bar: true,
                drag: DragScroll::Never,
                mouse_wheel: true,
            },
            ScrollBarVisibility::VisibleWhenNeeded,
        )
    } else {
        (ScrollSource::NONE, ScrollBarVisibility::AlwaysHidden)
    };
    let began = typing && !std::mem::replace(&mut view.was_typing, typing);
    let catch_up = !typing || began || std::mem::take(&mut view.catching_up);
    let paged = typing.then(|| {
        let keys = world.resource::<InputMap>();
        let turned = |action| {
            let pressed = keys
                .shortcut(action)
                .is_some_and(|key| ui.input_mut(|input| input.consume_shortcut(&key)));
            f32::from(u8::from(pressed))
        };
        turned(InputAction::PageBack) - turned(InputAction::PageForward)
    });
    let scrolled = ui
        .scope_builder(UiBuilder::new().max_rect(room), |ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for (state, strength) in [
                (&mut widgets.inactive, 0.3),
                (&mut widgets.hovered, 0.5),
                (&mut widgets.active, 0.6),
            ] {
                state.bg_fill = dusk::TEXT.gamma_multiply(strength);
                state.fg_stroke.color = state.bg_fill;
            }
            ui.visuals_mut().extreme_bg_color = Color32::TRANSPARENT;
            egui::ScrollArea::vertical()
                .id_salt(("chat log", typing.then_some(shown)))
                .auto_shrink(false)
                .stick_to_bottom(true)
                .scroll_source(source)
                .scroll_bar_visibility(bar)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add_space((room.height() - tall).max(0.0));
                    for line in laid {
                        let size = Vec2::new(width, line.tall + LINE_GAP);
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        if let Some(portrait) = line.portrait {
                            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            let at = Rect::from_min_size(rect.min, Vec2::splat(row_tall));
                            ui.painter().image(portrait, at, whole, Color32::WHITE);
                        }
                        let drawn = ui
                            .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                                text::show_linked(ui, world, &line.styled, text_look, None)
                            })
                            .inner;
                        inline::answer_items(ui, world, &drawn);
                        for (place, embed) in &line.embeds {
                            let (place, key) = (*place, embed_key(*place));
                            let rows: Vec<Rect> = drawn
                                .links
                                .iter()
                                .filter(|link| link.key == key)
                                .map(|link| link.row)
                                .collect();
                            let id = ui.id().with(("embed", line.id, place));
                            match embed {
                                LineEmbed::Covered => {
                                    if cover_bars(ui, world, id, &rows) {
                                        uncovered.push((line.id, place));
                                    }
                                }
                                LineEmbed::Shown {
                                    embed,
                                    name,
                                    speaker,
                                } => {
                                    if let Some(from) = embed_link(ui, id, &rows, name) {
                                        asked_for = Some(AskedEmbed {
                                            embed: *embed,
                                            from,
                                            speaker: speaker.clone(),
                                        });
                                    }
                                }
                                LineEmbed::Item => {}
                            }
                        }
                    }
                    if let Some(pages) = paged.filter(|pages| *pages != 0.0) {
                        ui.scroll_with_delta(Vec2::new(
                            0.0,
                            pages * PAGE_SHARE_OF_LOG * room.height(),
                        ));
                    }
                    if catch_up {
                        ui.scroll_to_cursor_animation(
                            Some(Align::BOTTOM),
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                })
        })
        .inner;
    view.revealed.extend(uncovered);
    let end = scrolled.content_size.y - scrolled.inner_rect.height();
    let reading = typing && scrolled.state.offset.y < end - 1.0;
    let log = world.resource::<TerminalLog>();
    let arrived = log.arrived(shown.terminal());
    if !typing {
        view.reading_from.clear();
    } else if !reading {
        view.reading_from.remove(&shown);
    }
    let since = if reading {
        arrived.saturating_sub(*view.reading_from.entry(shown).or_insert(arrived))
    } else {
        0
    };
    let unread = log
        .on(shown.terminal())
        .rev()
        .take(usize::try_from(since).unwrap_or(usize::MAX))
        .filter(|line| !typed_by(line, me))
        .count();
    if unread > 0 && unread_pill(ui, room, unread, &messages::of(&content).terminal).clicked() {
        view.catching_up = true;
    }
    asked_for
}

/// A thing of a line the player asked to see: where they clicked it, and who said the line.
struct AskedEmbed {
    embed: ChatEmbed,
    from: Rect,
    speaker: String,
}

/// Shows what a line holds as the interface shows such a thing anywhere: the card of a thing,
/// dealt from where the player clicked it, and a location on the map, pinned under the name of
/// whoever said it.
fn inspect(world: &mut World, asked: AskedEmbed) {
    let from = Some(asked.from);
    match asked.embed {
        ChatEmbed::Item(item) => Mention::Item(item).open(world, from),
        ChatEmbed::Quest(quest) => Mention::Quest(quest).open(world, from),
        ChatEmbed::Skill(skill) => Mention::Skill(skill).open(world, from),
        ChatEmbed::Location(location) => {
            let pin = MapPin {
                area: location.area,
                at: location.at(),
                name: asked.speaker,
            };
            area_map::open_on(world, MapSight::Pin(pin));
        }
    }
}

/// The client whose player typed `line`, for a line a player typed.
fn typist(line: &TerminalLine) -> Option<ClientId> {
    match &line.kind {
        LineKind::Said(speaker) => Some(ClientId(u32::try_from(speaker.typist?).ok()?)),
        LineKind::Notice | LineKind::Failed => None,
    }
}

/// Whether `line` is what this client's own player typed, which is never news to them.
fn key_hints(hints: &[(InputAction, &str)]) -> Vec<TextPiece> {
    hints
        .iter()
        .enumerate()
        .flat_map(|(place, (action, does))| {
            let gap = if place + 1 < hints.len() { "  " } else { "" };
            [
                map::input(*action),
                TextPiece::text(format!(" {does}{gap}")),
            ]
        })
        .collect()
}

fn typed_by(line: &TerminalLine, me: Option<u64>) -> bool {
    match &line.kind {
        LineKind::Said(speaker) => speaker.typist.is_some() && speaker.typist == me,
        LineKind::Notice | LineKind::Failed => false,
    }
}

fn unread_pill(ui: &mut Ui, room: Rect, unread: usize, words: &TerminalMessages) -> egui::Response {
    let rect = Rect::from_center_size(
        Pos2::new(room.center().x, room.bottom() - PILL.y / 2.0 - 4.0),
        PILL,
    );
    let label = words.unread(unread);
    let response = ui
        .interact(rect, Id::new("chat unread"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    ui.painter().rect_filled(rect, PILL.y / 2.0, dusk::NOTICE);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        &label,
        TextRole::Small.font_id(),
        dusk::FILL,
    );
    response
}

/// How a line a player typed is told from other players' lines and other channels'.
struct SaidLook<'a> {
    /// The channel it was said on, where chat shows every channel together.
    on: Option<&'a ChannelLook>,
    /// The colour the game gives its speaker's name, once the speaker is known.
    name_color: Option<Rgb>,
    /// How wide the speaker's portrait stands before the line, for a speaker who has one.
    portrait: Option<f32>,
}

/// A name and its colon tell speech from what the game itself says, and a player's name is in
/// the colour the game gives that player, behind their portrait, so a reader follows one
/// speaker through many lines. The name of the channel stands before what was said on it, so
/// the ink is never the only sign of where a line was said. An item the line holds is its inline
/// item, and any other thing is written by its name in its colour, behind its icon. Answers the
/// things the line holds, each by its place among them and a run of the line linked by
/// `embed_key`.
fn said_line(
    line: &TerminalLine,
    said: &SaidLook,
    content: &Content,
    covers: &dyn Fn(usize, ChatEmbedKind) -> bool,
) -> (StyledLine, Vec<(usize, LineEmbed)>) {
    let terminal = &messages::of(content).terminal;
    let mut styled = StyledLine::default();
    let mut embeds = Vec::new();
    let words = |styled: &mut StyledLine, text: &str, color: Color32| {
        styled
            .pieces
            .push(StyledSpan::plain(text.to_owned()).ink(color).into());
    };
    match &line.kind {
        LineKind::Said(speaker) => {
            let ink = match speaker.typist {
                Some(_) => said.name_color.map_or(dusk::CHARACTER, |Rgb([r, g, b])| {
                    ink::legible(Color32::from_rgb(r, g, b), dusk::FILL)
                }),
                None => dusk::TOWNSPERSON,
            };
            if let Some(wide) = said.portrait {
                styled.pieces.push(TextPiece::Room(wide + PORTRAIT_GAP));
            }
            let parts = match said.on {
                Some(on) => terminal.said_on_by((on.name.as_str(), on.color), (&speaker.name, ink)),
                None => terminal.said_by((speaker.name.as_str(), ink)),
            };
            let mut color = parts.iter().find_map(|part| match part {
                MessagePart::Argument((_, color)) => Some(*color),
                MessagePart::Words(_) => None,
            });
            for part in parts {
                match part {
                    MessagePart::Words(said) => {
                        words(&mut styled, &said, color.unwrap_or(dusk::TEXT))
                    }
                    MessagePart::Argument((name, ink)) => {
                        color = Some(ink);
                        words(&mut styled, name, ink);
                    }
                }
            }
            let mut held = line.embeds.iter();
            for (place, said_words) in line.text.split(EMBED).enumerate() {
                if let Some(place) = place.checked_sub(1) {
                    match held.next().and_then(|sent| ChatEmbed::read(content, sent)) {
                        Some(embed) if covers(place, embed.kind()) => {
                            styled.pieces.push(TextPiece::LinkStart {
                                key: embed_key(place),
                                name: terminal.covered().to_owned(),
                            });
                            words(&mut styled, terminal.covered(), Color32::TRANSPARENT);
                            styled.pieces.push(TextPiece::LinkEnd);
                            embeds.push((place, LineEmbed::Covered));
                        }
                        Some(ChatEmbed::Item(item)) => {
                            styled.pieces.extend(InlineItem::of(item).pieces(content));
                            embeds.push((place, LineEmbed::Item));
                        }
                        Some(embed) => {
                            let shown = embed.line(content);
                            styled.pieces.push(TextPiece::LinkStart {
                                key: embed_key(place),
                                name: shown.plain_words(content),
                            });
                            styled.pieces.extend(shown.styled(content).pieces);
                            styled.pieces.push(TextPiece::LinkEnd);
                            embeds.push((
                                place,
                                LineEmbed::Shown {
                                    embed,
                                    name: embed.shown(content).name,
                                    speaker: speaker.name.clone(),
                                },
                            ));
                        }
                        None => words(&mut styled, "?", dusk::TEXT),
                    }
                }
                words(&mut styled, said_words, dusk::TEXT);
            }
        }
        LineKind::Notice => words(&mut styled, &line.text, dusk::NOTICE),
        LineKind::Failed => words(&mut styled, &line.text, dusk::DANGER),
    }
    (styled, embeds)
}

/// The link a line's `place`th thing stands in.
fn embed_key(place: usize) -> LinkKey {
    struct Embed;
    LinkKey::new::<Embed>(place as u64)
}

/// A line of the log ready to draw.
struct LaidLine {
    id: LoggedLineId,
    styled: StyledLine,
    tall: f32,
    portrait: Option<egui::TextureId>,
    /// The things the line holds, by their place among its things.
    embeds: Vec<(usize, LineEmbed)>,
}

/// A thing a line holds, as chat shows it.
enum LineEmbed {
    /// Covered by chat until the player asks to see it.
    Covered,
    /// An item, which its inline item answers for.
    Item,
    /// Another thing, by its name: who said the line, for a location pinned on the map.
    Shown {
        embed: ChatEmbed,
        name: String,
        speaker: String,
    },
}

/// Draws the black bars over a thing chat covers, one for each row of the line its room runs
/// over. Whether the player asked to see the thing.
fn cover_bars(ui: &mut Ui, world: &World, key: Id, bars: &[Rect]) -> bool {
    let mut asked = false;
    let hint = messages::of(world.resource::<Content>())
        .terminal
        .covered_hint();
    for (row, bar) in bars.iter().enumerate() {
        let bar = bar.shrink2(Vec2::new(0.0, bar.height() * COVER_INSET));
        let response = ui
            .interact(bar, key.with(row), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, hint));
        ui.painter().rect_filled(bar, 2.0, Color32::BLACK);
        hud::tooltip(&response, world, hint, &[], None);
        asked |= response.clicked();
    }
    asked
}

/// Lets the player ask to see a thing other than an item that a line shows, by a click on its
/// name or its icon, a rect for each row it runs over. Where it was clicked, once it is.
fn embed_link(ui: &mut Ui, key: Id, rows: &[Rect], name: &str) -> Option<Rect> {
    let mut clicked = None;
    for (row, at) in rows.iter().enumerate() {
        let response = ui
            .interact(*at, key.with(row), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
        if response.clicked() {
            clicked = Some(*at);
        }
    }
    clicked
}

/// How much of its row's height a bar leaves clear above and below itself.
const COVER_INSET: f32 = 0.12;

// Focus is asked for after the field draws, so the key that gave chat the keyboard is never
// read by the field as its own.
fn field_row(ui: &mut Ui, world: &mut World, view: &mut ChatView, row: Rect, look: &ChatLook) {
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let (terminal, chat_words) = (&messages.terminal, &messages.chat);
    let send = world
        .resource::<InputMap>()
        .shortcut(InputAction::SubmitText);
    let typing = view.typing;
    let field_id = field_id();
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let caret = input::caret(ui.ctx(), field_id).unwrap_or(usize::MAX);
    let found = view.seeking.as_ref().map(|seeking| found(world, seeking));
    let asked = (focused && found.is_none())
        .then(|| EmbedKindsAsked::of(&view.draft, caret))
        .flatten();
    let options = match (&found, &asked) {
        (Some(found), _) => found.len(),
        (None, Some(asked)) => asked.kinds.len(),
        (None, None) => view
            .current()
            .map_or(0, |suggested| suggested.options.len()),
    };
    view.selected = view.selected.min(options.saturating_sub(1));
    let mut taken = None;
    if focused || found.is_some() {
        let keys = world.resource::<InputMap>();
        // The key that sends a line picks the thing looked for instead, while one is.
        taken = search::keyed(ui, keys, options, &mut view.selected, found.is_some());
    }
    // Lists take the keys that move in them first, and an input method putting a character
    // together takes every key.
    let recalls = focused && options == 0 && found.is_none() && asked.is_none() && !view.composing;
    if recalls {
        recall(ui, world.resource::<InputMap>(), view, caret);
    }
    if typing {
        ui.painter()
            .hline(row.x_range(), row.top(), Stroke::new(1.0, dusk::BORDER));
    } else {
        // The one part of chat that takes a click at rest stands out from the lines above it.
        let corners = egui::CornerRadius {
            nw: 0,
            ne: 0,
            ..dusk::RADIUS
        };
        ui.painter()
            .rect_filled(row, corners, dusk::FILL.gamma_multiply(ROW_WASH));
    }
    let inner = row.shrink2(Vec2::new(PAD, 0.0));
    // Over a plate the world shows through, a hint at less than full ink is lost in the grass.
    let hint_ink = if typing {
        dusk::TEXT.gamma_multiply(0.8)
    } else {
        dusk::TEXT
    };
    let faint = TextLook::new(TextRole::Small.size(), hint_ink).on(Ground::Dusk);
    let sends = !view.draft.text().trim().is_empty();
    let recalled = view
        .sent
        .place()
        .map(|(at, of)| TextPiece::text(format!("{}  ", terminal.recalled(at, of))));
    let only = |hints: &[(InputAction, &str, bool)]| -> Vec<TextPiece> {
        let kept: Vec<(InputAction, &str)> = hints
            .iter()
            .filter(|(.., works)| *works)
            .map(|&(action, does, _)| (action, does))
            .collect();
        key_hints(&kept)
    };
    let hints = StyledLine::new(if found.is_some() {
        only(&[
            (InputAction::CompleteText, terminal.hint_pick(), options > 0),
            (InputAction::Dismiss, terminal.hint_back(), true),
        ])
    } else if typing {
        let browsing = view.sent.browsing();
        recalled
            .into_iter()
            .chain(only(&[
                (InputAction::SubmitText, terminal.hint_send(), sends),
                (
                    InputAction::RecallEarlier,
                    terminal.recall_earlier(),
                    recalls && view.sent.has_earlier(),
                ),
                (
                    InputAction::RecallLater,
                    terminal.recall_later(),
                    recalls && browsing,
                ),
                (
                    InputAction::NextChatTab,
                    terminal.hint_next_tab(),
                    view.channels.len() > 1,
                ),
                (InputAction::Dismiss, terminal.back_to_draft(), browsing),
                (InputAction::Dismiss, terminal.hint_cancel(), !browsing),
            ]))
            .collect()
    } else if session::is_dead(world) {
        Vec::new()
    } else if scope::reads(world, InputAction::FocusChat) {
        key_hints(&[(InputAction::FocusChat, terminal.hint_to_chat())])
    } else {
        vec![TextPiece::text(terminal.click_to_chat().to_owned())]
    });
    let hints_wide = text::measure(ui.ctx(), world, &hints, faint, f32::INFINITY).x;
    let hinted = !typing || inner.width() >= hints_wide + ROOM_TO_TYPE_BESIDE_HINTS;
    let typed_on = look.of(view.shown);
    if !typing && view.draft.text().is_empty() {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| text::show(ui, world, &hints, faint, None),
        );
    }
    let before = view.draft.text().to_owned();
    let field = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(typed_on) = typed_on.filter(|_| typing) {
                    ui.label(
                        egui::RichText::new(terminal.typing_on(&typed_on.name))
                            .font(look.font.clone())
                            .color(dusk::TEXT.gamma_multiply(0.75)),
                    );
                }
                let room = if typing && hinted {
                    ui.available_width() - hints_wide - PAD
                } else {
                    ui.available_width()
                };
                ui.visuals_mut().text_cursor.stroke.color = dusk::TEXT;
                let ChatView { draft, seeking, .. } = &mut *view;
                let field = match seeking {
                    Some(seeking) => sought_field(ui, world, draft, seeking, look, room.max(0.0)),
                    None => typed_field(ui, world, draft, look, room.max(0.0), send),
                };
                if typing && hinted {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        text::show(ui, world, &hints, faint, None)
                    });
                }
                field
            },
        )
        .inner;
    if view.draft.text() != before || field.changed() {
        view.selected = 0;
        view.sent.stop();
    }
    let kinds: Vec<String> = asked
        .iter()
        .flat_map(|asked| &asked.kinds)
        .map(|kind| format!("{EMBED_ASK}{}", kind.name()))
        .collect();
    let drawn: Vec<SuggestedMention> = found
        .iter()
        .flatten()
        .map(|found| {
            SuggestedMention::new(
                world,
                ui.ctx(),
                &found.embed.line(world.resource::<Content>()),
            )
        })
        .collect();
    let (heading, listed): (String, Vec<Suggestion>) = match (&found, &asked, &view.suggested) {
        (Some(found), ..) => {
            let kind = view
                .seeking
                .as_ref()
                .map_or("", |seeking| seeking.kind.hint(chat_words));
            let listed = found
                .iter()
                .zip(&drawn)
                .map(|(found, drawn)| Suggestion {
                    text: &found.name,
                    detail: "",
                    muted: false,
                    drawn: Some(drawn),
                })
                .collect();
            (terminal.show_kind(kind), listed)
        }
        (None, Some(_), _) => {
            let listed = kinds
                .iter()
                .map(|text| Suggestion {
                    text,
                    detail: "",
                    muted: false,
                    drawn: None,
                })
                .collect();
            (terminal.show_in_message().to_owned(), listed)
        }
        (None, None, Some(suggested)) if focused => {
            let listed = suggested
                .options
                .iter()
                .map(|option| Suggestion {
                    text: &option.text,
                    detail: &option.detail,
                    muted: option.muted,
                    drawn: None,
                })
                .collect();
            (suggested.usage.clone(), listed)
        }
        (None, None, _) => (String::new(), Vec::new()),
    };
    if !listed.is_empty() || !heading.is_empty() {
        let suggested = suggest::suggestions(
            ui.ctx(),
            field_id.with("suggestions"),
            field.rect,
            &heading,
            &listed,
            view.selected,
        );
        taken = taken.or(suggested);
    }
    match (taken, found, asked) {
        (Some(place), Some(found), _) => {
            if let (Some(found), Some(seeking)) = (found.get(place), view.seeking.take()) {
                let caret = view.draft.embed(seeking.at, found.embed, &found.name);
                input::caret_to(ui.ctx(), field_id, caret);
            }
        }
        (Some(place), None, Some(asked)) => {
            if let Some(&kind) = asked.kinds.get(place) {
                view.draft.cut(asked.letters.clone());
                let at = asked.letters.start;
                input::caret_to(ui.ctx(), field_id, at);
                match kind.table() {
                    Some(_) => {
                        view.seeking = Some(EmbedSearch {
                            kind,
                            at,
                            name: String::new(),
                        });
                    }
                    None => {
                        if let Some(here) = here(world) {
                            let name = here.shown(world.resource::<Content>()).name;
                            let caret = view.draft.embed(at, here, &name);
                            input::caret_to(ui.ctx(), field_id, caret);
                        }
                    }
                }
            }
        }
        (Some(place), None, None) => {
            let completed = view
                .current()
                .and_then(|suggested| Some(suggested.applied(suggested.options.get(place)?)));
            if let Some(completed) = completed {
                input::caret_to_end(ui.ctx(), field_id, &completed);
                view.draft.write(completed);
            }
        }
        (None, ..) => {}
    }
    if field.gained_focus() || field.clicked() {
        view.typing = true;
    }
    let (composed, away) = ui.input(|input| {
        let composed = input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Ime(_)));
        (composed, !input.focused)
    });
    let composing = view.composing || composed;
    view.composing = ui.input(|input| still_composing(view.composing, &input.events));
    let sent = view.seeking.is_none()
        && field.lost_focus()
        && send.is_some_and(|key| ui.input(|input| input.key_pressed(key.logical_key)))
        && !composing;
    if sent {
        let draft = view.draft.take();
        let kept = world
            .resource::<Content>()
            .built::<GameSettings>()
            .chat
            .kept_lines;
        view.sent.keep(draft.clone(), kept);
        if !draft.text().trim().is_empty() {
            let content = world.resource::<Content>();
            // A command is words alone, so what a line holds is sent with it only when said.
            let (text, embeds) = match command_line(draft.text().trim_start()) {
                Some(_) => (draft.text().to_owned(), Vec::new()),
                None => draft.sent(content),
            };
            world.write_message(TerminalInput {
                text,
                channel: view.shown.terminal(),
                embeds,
            });
        }
        view.typing = false;
    }
    let ours = [field_id, search_id()];
    let elsewhere = ui.memory(|memory| memory.focused().is_some_and(|held| !ours.contains(&held)));
    if elsewhere || away {
        view.typing = false;
        view.seeking = None;
    }
    let wanted = match view.seeking {
        Some(_) => search_id(),
        None => field_id,
    };
    if away {
        ui.memory_mut(|memory| ours.map(|id| memory.surrender_focus(id)));
    } else if view.typing && !ui.memory(|memory| memory.has_focus(wanted)) {
        ui.memory_mut(|memory| memory.request_focus(wanted));
    }
}

fn search_id() -> Id {
    Id::new("chat embed search")
}

/// The field the line is typed in: its words as they are typed, and the things it holds by
/// their names in their colours, each behind its icon.
fn typed_field(
    ui: &mut Ui,
    world: &mut World,
    draft: &mut ChatDraft,
    look: &ChatLook,
    room: f32,
    send: Option<egui::KeyboardShortcut>,
) -> egui::Response {
    let id = field_id();
    let side = ui.fonts_mut(|fonts| fonts.row_height(&look.font));
    let held: Vec<(Range<usize>, ChatEmbed)> = draft.held().collect();
    let mut icons = Vec::new();
    let mut layouter = |ui: &Ui, typed: &dyn egui::TextBuffer, _wrap: f32| {
        let (job, at) = draft_job(typed.as_str(), &held, look, side);
        icons = at;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut text = draft.text().to_owned();
    let shown = egui::TextEdit::singleline(&mut text)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(look.font.clone())
        .text_color(dusk::TEXT)
        .desired_width(room)
        .char_limit(MAX_INPUT_LEN)
        .return_key(send)
        .lock_focus(true)
        .layouter(&mut layouter)
        .show(ui);
    let painter = ui.painter().with_clip_rect(shown.text_clip_rect);
    embed_icons(&painter, shown.galley_pos, &shown.galley, &icons);
    let items = held_items(&shown.galley, shown.galley_pos, &held, side);
    item_tooltips(ui, world, id, &items);
    let caret = shown.cursor_range.map(|range| range.primary.index.0);
    let moved = if text == draft.text() {
        None
    } else {
        draft.retyped(&text, caret.unwrap_or(usize::MAX))
    };
    let rest = moved.or(caret).map(|caret| draft.kept_out(caret));
    if rest != caret
        && let Some(rest) = rest
    {
        input::caret_to(ui.ctx(), id, rest);
    }
    shown.response.response
}

/// The search for a thing to set in the line, typed in the line where the thing will stand:
/// the words before it, the name being looked for, and the words after, all within `room`.
fn sought_field(
    ui: &mut Ui,
    world: &mut World,
    draft: &ChatDraft,
    seeking: &mut EmbedSearch,
    look: &ChatLook,
    room: f32,
) -> egui::Response {
    let side = ui.fonts_mut(|fonts| fonts.row_height(&look.font));
    let letters: Vec<char> = draft.text().chars().collect();
    let at = seeking.at.min(letters.len());
    let before: Vec<(Range<usize>, ChatEmbed)> =
        draft.held().filter(|(held, _)| held.end <= at).collect();
    let after: Vec<(Range<usize>, ChatEmbed)> = draft
        .held()
        .filter(|(held, _)| held.start >= at)
        .map(|(held, embed)| (held.start - at..held.end - at, embed))
        .collect();
    let laid = |ui: &Ui, text: String, held: &[(Range<usize>, ChatEmbed)]| {
        let (job, icons) = draft_job(&text, held, look, side);
        (ui.fonts_mut(|fonts| fonts.layout_job(job)), icons)
    };
    // What does not fit is cut at the far end from the search, which stays in sight. Answers
    // where the items it holds stand, as far as they are in sight.
    let words = |ui: &mut Ui,
                 (galley, icons): (Arc<Galley>, Vec<EmbedIcon>),
                 held: &[(Range<usize>, ChatEmbed)],
                 room: f32| {
        let size = Vec2::new(galley.size().x.min(room.max(0.0)), galley.size().y);
        if size.x <= 0.0 {
            return Vec::new();
        }
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let from = Pos2::new(rect.right() - galley.size().x, rect.top());
        let painter = ui.painter().with_clip_rect(rect);
        embed_icons(&painter, from, &galley, &icons);
        let items = held_items(&galley, from, held, side)
            .into_iter()
            .filter_map(|(at, item)| {
                Some((at.intersect(rect), item)).filter(|(at, _)| at.is_positive())
            })
            .collect();
        painter.galley(from, galley, dusk::TEXT);
        items
    };
    ui.spacing_mut().item_spacing.x = 2.0;
    let starts = ui.cursor().left();
    let kind = format!("{}:", seeking.kind.name());
    let kind_wide = ui
        .painter()
        .layout_no_wrap(kind.clone(), look.font.clone(), dusk::NOTICE)
        .size()
        .x;
    let said_before = laid(ui, letters[..at].iter().collect(), &before);
    let around = kind_wide + 4.0 * ICON_GAP;
    let search = (room - around - said_before.0.size().x).clamp(SEARCH_LEAST, SEARCH_WIDTH);
    let mut items = words(ui, said_before, &before, room - around - search);
    let field = egui::Frame::new()
        .fill(dusk::FILL)
        .stroke(Stroke::new(1.0, dusk::BORDER))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(ICON_GAP as i8, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = ICON_GAP;
            ui.label(
                egui::RichText::new(kind)
                    .font(look.font.clone())
                    .color(dusk::NOTICE),
            );
            ui.add(
                egui::TextEdit::singleline(&mut seeking.name)
                    .id(search_id())
                    .frame(egui::Frame::NONE)
                    .font(look.font.clone())
                    .text_color(dusk::TEXT)
                    .desired_width(search)
                    .lock_focus(true),
            )
        })
        .inner;
    let left = room - (ui.cursor().left() - starts);
    let said_after = laid(ui, letters[at..].iter().collect(), &after);
    items.extend(words(ui, said_after, &after, left));
    item_tooltips(ui, world, search_id(), &items);
    field
}

/// Where each item `held` by what is typed stands in its field, laid out as `galley` at
/// `origin`: its name, and the icon of `side` before it.
fn held_items(
    galley: &Galley,
    origin: Pos2,
    held: &[(Range<usize>, ChatEmbed)],
    side: f32,
) -> Vec<(Rect, ItemId)> {
    held.iter()
        .filter_map(|(letters, embed)| match embed {
            ChatEmbed::Item(item) => Some((letters, *item)),
            _ => None,
        })
        .flat_map(|(letters, item)| {
            letters_rects(galley, letters.clone())
                .into_iter()
                .enumerate()
                .map(move |(row, at)| {
                    let icon = if row == 0 { side + 2.0 * ICON_GAP } else { 0.0 };
                    let at = Rect::from_min_max(at.min - Vec2::new(icon, 0.0), at.max);
                    (at.translate(origin.to_vec2()), item)
                })
        })
        .collect()
}

/// Shows the tooltip of each item of a field at its place, leaving the click to the field, which
/// puts the caret there.
fn item_tooltips(ui: &mut Ui, world: &mut World, field: Id, items: &[(Rect, ItemId)]) {
    for (index, &(at, item)) in items.iter().enumerate() {
        let response = ui.interact(at, field.with(("held item", index)), Sense::hover());
        inline::tooltip(world, &response, InlineItem::of(item));
    }
}

/// What is typed laid out for its field: its words in the field's ink, and the things it holds
/// by their names in their colours, with the icons to draw before them.
fn draft_job(
    text: &str,
    held: &[(Range<usize>, ChatEmbed)],
    look: &ChatLook,
    icon_side: f32,
) -> (LayoutJob, Vec<EmbedIcon>) {
    let plain = TextFormat {
        font_id: look.font.clone(),
        color: dusk::TEXT,
        ..TextFormat::default()
    };
    let letters: Vec<char> = text.chars().collect();
    let words = |letters: &[char]| letters.iter().collect::<String>();
    let mut job = LayoutJob::default();
    let mut icons = Vec::new();
    let mut at = 0;
    for (held, embed) in held {
        // The field shows a key's letter a frame before the draft takes it in.
        let (Some(embed), true) = (
            look.embeds.0.get(embed),
            at <= held.start && held.end <= letters.len(),
        ) else {
            continue;
        };
        job.append(&words(&letters[at..held.start]), 0.0, plain.clone());
        let starts = job.text.chars().count();
        let format = TextFormat {
            color: embed.color,
            ..plain.clone()
        };
        job.append(
            &words(&letters[held.clone()]),
            icon_side + 2.0 * ICON_GAP,
            format,
        );
        if let Some(icon) = embed.icon {
            icons.push((starts, icon));
        }
        at = held.end;
    }
    job.append(&words(&letters[at..]), 0.0, plain);
    (job, icons)
}

/// The kinds of thing a line can hold that the word at the caret asks for: one that begins a
/// word with the sign for it, outside a command.
struct EmbedKindsAsked {
    /// The letters of the word, which a kind picked takes the place of.
    letters: Range<usize>,
    kinds: Vec<ChatEmbedKind>,
}

impl EmbedKindsAsked {
    fn of(draft: &ChatDraft, caret: usize) -> Option<EmbedKindsAsked> {
        if draft.text().trim_start().starts_with(COMMAND_PREFIX) {
            return None;
        }
        let letters: Vec<char> = draft.text().chars().collect();
        let caret = caret.min(letters.len());
        let from = letters[..caret]
            .iter()
            .rposition(|letter| letter.is_whitespace())
            .map_or(0, |space| space + 1);
        if from >= caret || letters[from] != EMBED_ASK {
            return None;
        }
        if draft
            .held()
            .any(|(held, _)| held.start < caret && from < held.end)
        {
            return None;
        }
        let typed: String = letters[from + 1..caret].iter().collect();
        let names = ChatEmbedKind::VARIANTS.iter().map(|kind| kind.name());
        let kinds: Vec<ChatEmbedKind> = best_matches(&typed, names, ChatEmbedKind::VARIANTS.len())
            .into_iter()
            .map(|place| ChatEmbedKind::VARIANTS[place])
            .collect();
        (!kinds.is_empty()).then_some(EmbedKindsAsked {
            letters: from..caret,
            kinds,
        })
    }
}

/// Where the player stands, as a line holds it.
fn here(world: &World) -> Option<ChatEmbed> {
    ChatLocation::of(world, session::me(world)?).map(ChatEmbed::Location)
}

/// A thing a search found: what it is, and how the list of what was found names it.
struct FoundEmbed {
    embed: ChatEmbed,
    name: String,
}

/// The things of the kind looked for whose names match what is typed best, as a command line
/// lists them for an input of their table.
fn found(world: &World, seeking: &EmbedSearch) -> Vec<FoundEmbed> {
    let content = world.resource::<Content>();
    let Some(table) = seeking.kind.table() else {
        return Vec::new();
    };
    search::matching(world, table, &seeking.name)
        .into_iter()
        .filter_map(|option| {
            Some(FoundEmbed {
                embed: ChatEmbed::of(content, seeking.kind, option.file()?)?,
                name: option.text,
            })
        })
        .collect()
}

fn still_composing(was: bool, events: &[egui::Event]) -> bool {
    events.iter().fold(was, |composing, event| match event {
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => !text.is_empty(),
        egui::Event::Ime(egui::ImeEvent::Commit(_)) => false,
        _ => composing,
    })
}

/// Brings back a line sent earlier, or one later, as the player presses the recall keys.
fn recall(ui: &mut Ui, keys: &InputMap, view: &mut ChatView, caret: usize) {
    let pressed = |ui: &mut Ui, action: InputAction| {
        keys.shortcut(action)
            .is_some_and(|shortcut| ui.input_mut(|input| input.consume_shortcut(&shortcut)))
    };
    let shown = if pressed(ui, InputAction::RecallEarlier) {
        let typed = InField {
            draft: view.draft.clone(),
            caret,
        };
        view.sent.earlier(&typed)
    } else if pressed(ui, InputAction::RecallLater) {
        view.sent.later()
    } else {
        None
    };
    if let Some(shown) = shown {
        view.draft = shown.draft;
        view.suggested = None;
        input::caret_to(ui.ctx(), field_id(), shown.caret);
    }
}

/// Asks the server what a command being typed could go on with. A line brought back from those
/// sent earlier asks nothing until it is edited, so the list stays shut and the recall keys keep
/// stepping through history.
fn ask(world: &mut World, view: &mut ChatView) {
    let draft = view.draft.text();
    if view.sent.browsing() {
        view.suggested = None;
        return;
    }
    if !draft.trim_start().starts_with(COMMAND_PREFIX) {
        view.asked = None;
        view.suggested = None;
        return;
    }
    if view.waiting || view.asked.as_deref() == Some(draft) {
        return;
    }
    let text = draft.to_owned();
    view.asked = Some(text.clone());
    view.waiting = true;
    world.write_message(TerminalCompletionRequest { text });
}
