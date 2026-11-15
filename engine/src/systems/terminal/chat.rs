use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy_terminal::{
    COMMAND_PREFIX, LineKind, MAX_INPUT_LEN, TerminalCompletionRequest, TerminalCompletions,
    TerminalInput, TerminalLine,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::egui;
use ui::egui::containers::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use ui::egui::emath::TSTransform;
use ui::egui::text::LayoutJob;
use ui::egui::{
    Align, Align2, Color32, CursorIcon, FontId, Galley, Id, Layout, Order, Pos2, Rect, Sense,
    Stroke, TextFormat, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType,
};
use ui::face::TextRole;
use ui::ink::Ground;
use ui::panel::GripCorner;
use ui::suggest::{self, Suggestion};
use ui::{dusk, input};

use super::TerminalLog;
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band, GuiImage, Tier};
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::chat::{ChatChannel, ChatChannels};
use crate::systems::hud::{self, SizedPiece};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::player::session;
use crate::systems::settings::{Placed, UserSettings};

pub const CHAT: &str = "chat";

const START: Vec2 = Vec2::new(310.0, 186.0);
/// Two lines and the row they are typed in.
const SMALLEST: Vec2 = Vec2::new(220.0, 96.0);
const BAR: f32 = 12.0;
const TABS: f32 = 22.0;
const TAB_PAD: f32 = 8.0;
const TAB_GAP: f32 = 4.0;
/// How large a channel's sign is beside the letters it marks.
const SIGN_SHARE_OF_TEXT: f32 = 0.8;
const SIGN_GAP: f32 = 4.0;
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

#[derive(Resource, Default)]
pub(crate) struct ChatView {
    draft: String,
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
    /// How many lines had arrived on each channel when its tab was last shown.
    seen: HashMap<ChatChannel, u64>,
    reading_from: HashMap<ChatChannel, u64>,
    catching_up: bool,
    was_typing: bool,
}

impl ChatView {
    /// What is suggested for the draft as it stands: an answer to an older draft suggests
    /// nothing, as taking one of its options would undo what was typed since.
    fn current(&self) -> Option<&TerminalCompletions> {
        self.suggested
            .as_ref()
            .filter(|suggested| suggested.text == self.draft)
    }
}

pub fn hand_back(world: &mut World) -> bool {
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
        if answer.text == view.draft {
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
        view.seen.remove(channel);
        view.reading_from.remove(channel);
    }
    if !told.0.contains(&view.shown) {
        view.shown = told.0.first().copied().unwrap_or_default();
    }
    view.channels = told.0;
}

/// What chat shows of its channels, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownChat {
    /// The tab shown, which what is typed goes to.
    pub shown: ChatChannel,
    /// A tab for each channel the player is in, in the order chat lists them.
    pub tabs: Vec<ShownChatTab>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownChatTab {
    pub channel: ChatChannel,
    pub name: String,
    /// How many lines arrived on the channel since its tab was last shown.
    pub unread: u32,
}

pub fn shown(world: &World) -> ShownChat {
    let view = world.resource::<ChatView>();
    let content = world.resource::<Content>();
    let log = world.resource::<TerminalLog>();
    let me = session::my_id(world).map(|client| u64::from(client.0));
    ShownChat {
        shown: view.shown,
        tabs: view
            .channels
            .iter()
            .map(|&channel| ShownChatTab {
                channel,
                name: channel.id(content).get(content).name.clone(),
                unread: u32::try_from(unread(log, view, channel, me)).unwrap_or(u32::MAX),
            })
            .collect(),
    }
}

/// How many lines arrived on `channel` since its tab was last shown, the player's own aside.
fn unread(log: &TerminalLog, view: &ChatView, channel: ChatChannel, me: Option<u64>) -> usize {
    let seen = view.seen.get(&channel).copied().unwrap_or(0);
    let since = log.arrived(channel.terminal()).saturating_sub(seen);
    log.on(channel.terminal())
        .rev()
        .take(usize::try_from(since).unwrap_or(usize::MAX))
        .filter(|line| !typed_by(line, me))
        .count()
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
    let look = ChatLook {
        background,
        font,
        channels: ChatChannel::VARIANTS
            .iter()
            .map(|&channel| ChannelLook::of(world, channel))
            .collect(),
    };
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
            let handles = typing.then(|| handles(ui, rect));
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
                tabs(ui, world, &mut view, strip, &look);
            }
            lines(ui, world, &mut view, log, &look);
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
    /// How each channel the engine lists is told from the others, in their order.
    channels: Vec<ChannelLook>,
}

impl ChatLook {
    fn of(&self, channel: ChatChannel) -> Option<&ChannelLook> {
        self.channels.iter().find(|look| look.channel == channel)
    }

    fn sign(&self) -> Vec2 {
        Vec2::splat(self.font.size * SIGN_SHARE_OF_TEXT)
    }
}

struct ChannelLook {
    channel: ChatChannel,
    name: String,
    sign: Option<GuiImage>,
    color: Color32,
}

impl ChannelLook {
    fn of(world: &mut World, channel: ChatChannel) -> ChannelLook {
        let content = world.resource::<Content>().clone();
        let def = channel.id(&content).get(&content);
        let [r, g, b] = def.color.0;
        ChannelLook {
            channel,
            name: def.name.clone(),
            sign: gui::image(world, &def.sign),
            color: Color32::from_rgb(r, g, b),
        }
    }

    fn paint_sign(&self, ui: &Ui, at: Rect, opacity: f32) {
        if let Some(sign) = &self.sign {
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter()
                .image(sign.texture, at, whole, self.color.gamma_multiply(opacity));
        }
    }
}

/// A tab for each channel the player is in, the one shown underlined and every other counting
/// the lines that arrived on it since it was.
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
    let log = world.resource::<TerminalLog>();
    let me = session::my_id(world).map(|client| u64::from(client.0));
    let sign = look.sign();
    let mut left = strip.left() + PAD - TAB_PAD;
    for channel in view.channels.clone() {
        let Some(channel_look) = look.of(channel) else {
            continue;
        };
        let selected = channel == view.shown;
        let waiting = unread(log, view, channel, me);
        let ink = if selected {
            dusk::TEXT
        } else {
            dusk::TEXT.gamma_multiply(0.65)
        };
        let name = ui
            .painter()
            .layout_no_wrap(channel_look.name.clone(), look.font.clone(), ink);
        let count = (waiting > 0).then(|| {
            ui.painter()
                .layout_no_wrap(waiting.to_string(), TextRole::Small.font_id(), dusk::FILL)
        });
        let pill = count
            .as_ref()
            .map(|count| Vec2::new(count.size().x + 10.0, count.size().y + 2.0));
        let wide = sign.x + SIGN_GAP + name.size().x + pill.map_or(0.0, |pill| SIGN_GAP + pill.x);
        let rect = Rect::from_min_size(
            Pos2::new(left, strip.top()),
            Vec2::new(wide + 2.0 * TAB_PAD, strip.height()),
        );
        let label = match waiting {
            0 => format!("{} chat", channel_look.name),
            waiting => format!("{} chat, {waiting} new", channel_look.name),
        };
        let response = ui
            .interact(rect, Id::new(("chat tab", channel)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &label)
        });
        if response.clicked() {
            view.shown = channel;
        }
        let middle = rect.center().y;
        let mut at = rect.left() + TAB_PAD;
        let sign_at = Rect::from_min_size(Pos2::new(at, middle - sign.y / 2.0), sign);
        channel_look.paint_sign(ui, sign_at, if selected { 1.0 } else { 0.65 });
        at += sign.x + SIGN_GAP;
        let name_wide = name.size().x;
        ui.painter()
            .galley(Pos2::new(at, middle - name.size().y / 2.0), name, ink);
        at += name_wide + SIGN_GAP;
        if let (Some(count), Some(pill)) = (count, pill) {
            let pill = Rect::from_min_size(Pos2::new(at, middle - pill.y / 2.0), pill);
            ui.painter()
                .rect_filled(pill, pill.height() / 2.0, dusk::NOTICE);
            ui.painter()
                .galley(pill.center() - count.size() / 2.0, count, dusk::FILL);
        }
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

fn handles(ui: &mut Ui, rect: Rect) -> (egui::Response, egui::Response) {
    let id = Id::new(("hud piece", CHAT));
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + BAR));
    let mover = ui
        .interact(bar, id.with("bar"), Sense::drag())
        .on_hover_cursor(CursorIcon::Grab);
    mover.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, "Move chat"));
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
    sizer.widget_info(|| WidgetInfo::labeled(WidgetType::ResizeHandle, true, "Resize chat"));
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
fn lines(ui: &mut Ui, world: &World, view: &mut ChatView, room: Rect, look: &ChatLook) {
    let log = world.resource::<TerminalLog>();
    let me = session::my_id(world).map(|client| u64::from(client.0));
    let width = room.width() - SCROLL_BAR - PAD;
    let typing = view.typing;
    let shown = view.shown;
    let of_tab = |line: &&TerminalLine| !typing || line.channel == shown.terminal();
    let laid: Vec<(Arc<Galley>, String, Option<&ChannelLook>)> = log
        .lines()
        .filter(of_tab)
        .map(|line| {
            let said_on = match line.kind {
                LineKind::Said(_) => ChatChannel::of(line.channel).and_then(|on| look.of(on)),
                LineKind::Notice | LineKind::Failed => None,
            };
            let job = line_job(line, said_on, look, width);
            let plain = job.text.clone();
            (ui.fonts_mut(|fonts| fonts.layout_job(job)), plain, said_on)
        })
        .collect();
    let tall: f32 = laid
        .iter()
        .map(|(galley, ..)| galley.size().y + LINE_GAP)
        .sum();
    let sign = look.sign();
    let row_tall = ui.fonts_mut(|fonts| fonts.row_height(&look.font));
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
                    for (galley, plain, said_on) in laid {
                        let size = Vec2::new(width, galley.size().y + LINE_GAP);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
                        response
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &plain));
                        if ui.is_rect_visible(rect) {
                            if let Some(said_on) = said_on {
                                let at = rect.min + Vec2::new(0.0, (row_tall - sign.y) / 2.0);
                                said_on.paint_sign(ui, Rect::from_min_size(at, sign), 1.0);
                            }
                            dusk::lettering(
                                ui.painter(),
                                rect.min,
                                galley,
                                dusk::SMALL_TEXT_SHADOW,
                                TSTransform::IDENTITY,
                            );
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
    let end = scrolled.content_size.y - scrolled.inner_rect.height();
    let reading = typing && scrolled.state.offset.y < end - 1.0;
    let arrived = log.arrived(shown.terminal());
    view.seen.insert(shown, arrived);
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
    if unread > 0 && unread_pill(ui, room, unread).clicked() {
        view.catching_up = true;
    }
}

/// Whether `line` is what this client's own player typed, which is never news to them.
fn typed_by(line: &TerminalLine, me: Option<u64>) -> bool {
    match &line.kind {
        LineKind::Said(speaker) => speaker.typist.is_some() && speaker.typist == me,
        LineKind::Notice | LineKind::Failed => false,
    }
}

fn unread_pill(ui: &mut Ui, room: Rect, unread: usize) -> egui::Response {
    let rect = Rect::from_center_size(
        Pos2::new(room.center().x, room.bottom() - PILL.y / 2.0 - 4.0),
        PILL,
    );
    let label = format!("{unread} new");
    let response = ui
        .interact(rect, Id::new("chat unread"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    ui.painter().rect_filled(rect, PILL.y / 2.0, dusk::NOTICE);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("↓ {label}"),
        TextRole::Small.font_id(),
        dusk::FILL,
    );
    response
}

/// A name and its colon tell speech from what the game itself says, and the sign of its channel
/// stands before what was said on one, so the ink is never the only sign.
fn line_job(
    line: &TerminalLine,
    said_on: Option<&ChannelLook>,
    look: &ChatLook,
    width: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let format = |color: Color32| TextFormat {
        font_id: look.font.clone(),
        color,
        ..TextFormat::default()
    };
    match &line.kind {
        LineKind::Said(speaker) => {
            let ink = match speaker.typist {
                Some(_) => said_on.map_or(dusk::PLAYER, |said_on| said_on.color),
                None => dusk::TOWNSPERSON,
            };
            let lead = said_on.map_or(0.0, |_| look.sign().x + SIGN_GAP);
            job.append(&format!("{}:", speaker.name), lead, format(ink));
            job.append(&format!(" {}", line.text), 0.0, format(dusk::TEXT));
        }
        LineKind::Notice => job.append(&line.text, 0.0, format(dusk::NOTICE)),
        LineKind::Failed => job.append(&line.text, 0.0, format(dusk::DANGER)),
    }
    job
}

// Focus is asked for after the field draws, so the key that gave chat the keyboard is never
// read by the field as its own.
fn field_row(ui: &mut Ui, world: &mut World, view: &mut ChatView, row: Rect, look: &ChatLook) {
    let keys = world.resource::<InputMap>();
    let send = keys.shortcut(InputAction::SubmitText);
    let key = |action| keys.shortcut(action);
    let (take, previous, next) = (
        key(InputAction::CompleteText),
        key(InputAction::ChoosePrevious),
        key(InputAction::ChooseNext),
    );
    let typing = view.typing;
    let field_id = field_id();
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let options = view
        .current()
        .map_or(0, |suggested| suggested.options.len());
    let mut taken = None;
    if focused && options > 0 {
        let pressed = |shortcut: Option<egui::KeyboardShortcut>| {
            shortcut.is_some_and(|shortcut| ui.input_mut(|input| input.consume_shortcut(&shortcut)))
        };
        if pressed(previous) {
            view.selected = (view.selected + options - 1) % options;
        }
        if pressed(next) {
            view.selected = (view.selected + 1) % options;
        }
        if pressed(take) {
            taken = Some(view.selected);
        }
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
    let hints = StyledLine::new(if typing && view.channels.len() > 1 {
        vec![
            map::input(InputAction::SubmitText),
            TextPiece::text(" send  "),
            map::input(InputAction::NextChatTab),
            TextPiece::text(" next tab  "),
            map::input(InputAction::Dismiss),
            TextPiece::text(" cancel"),
        ]
    } else if typing {
        vec![
            map::input(InputAction::SubmitText),
            TextPiece::text(" send  "),
            map::input(InputAction::Dismiss),
            TextPiece::text(" cancel"),
        ]
    } else if scope::reads(world, InputAction::FocusChat) {
        vec![
            map::input(InputAction::FocusChat),
            TextPiece::text(" to chat"),
        ]
    } else {
        vec![TextPiece::text("Click to chat")]
    });
    let hints_wide = text::measure(ui.ctx(), world, &hints, faint, f32::INFINITY).x;
    let hinted = !typing || inner.width() >= hints_wide + ROOM_TO_TYPE_BESIDE_HINTS;
    let typed_on = look.of(view.shown);
    if !typing && view.draft.is_empty() {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| text::show(ui, world, &hints, faint, None),
        );
    }
    let field = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(typed_on) = typed_on.filter(|_| typing) {
                    let (sign, _) = ui.allocate_exact_size(look.sign(), Sense::hover());
                    typed_on.paint_sign(ui, sign, 1.0);
                    ui.label(
                        egui::RichText::new(format!("{}:", typed_on.name))
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
                let field = ui.add(
                    egui::TextEdit::singleline(&mut view.draft)
                        .id(field_id)
                        .frame(egui::Frame::NONE)
                        .font(look.font.clone())
                        .text_color(dusk::TEXT)
                        .desired_width(room.max(0.0))
                        .char_limit(MAX_INPUT_LEN)
                        .return_key(send)
                        .lock_focus(true),
                );
                if typing && hinted {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        text::show(ui, world, &hints, faint, None)
                    });
                }
                field
            },
        )
        .inner;
    if let Some(suggested) = view.current().filter(|_| focused) {
        let listed: Vec<Suggestion> = suggested
            .options
            .iter()
            .map(|option| Suggestion {
                text: &option.text,
                detail: &option.detail,
            })
            .collect();
        if !listed.is_empty() || !suggested.usage.is_empty() {
            let clicked = suggest::suggestions(
                ui.ctx(),
                field_id.with("suggestions"),
                field.rect,
                &suggested.usage,
                &listed,
                view.selected,
            );
            taken = taken.or(clicked);
        }
    }
    let completed = taken.and_then(|place| {
        let suggested = view.current()?;
        Some(suggested.applied(suggested.options.get(place)?))
    });
    if let Some(completed) = completed {
        view.draft = completed;
        input::caret_to_end(ui.ctx(), field_id, &view.draft);
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
    let sent = field.lost_focus()
        && send.is_some_and(|key| ui.input(|input| input.key_pressed(key.logical_key)))
        && !composing;
    if sent {
        let line = std::mem::take(&mut view.draft);
        if !line.trim().is_empty() {
            world.write_message(TerminalInput {
                text: line,
                channel: view.shown.terminal(),
            });
        }
        view.typing = false;
    }
    let elsewhere = ui.memory(|memory| memory.focused().is_some_and(|held| held != field_id));
    if elsewhere || away {
        view.typing = false;
    }
    if away {
        ui.memory_mut(|memory| memory.surrender_focus(field_id));
    } else if view.typing && !field.has_focus() {
        field.request_focus();
    }
}

fn still_composing(was: bool, events: &[egui::Event]) -> bool {
    events.iter().fold(was, |composing, event| match event {
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => !text.is_empty(),
        egui::Event::Ime(egui::ImeEvent::Commit(_)) => false,
        _ => composing,
    })
}

fn ask(world: &mut World, view: &mut ChatView) {
    if !view.draft.trim_start().starts_with(COMMAND_PREFIX) {
        view.asked = None;
        view.suggested = None;
        return;
    }
    if view.waiting || view.asked.as_ref() == Some(&view.draft) {
        return;
    }
    view.asked = Some(view.draft.clone());
    view.waiting = true;
    world.write_message(TerminalCompletionRequest {
        text: view.draft.clone(),
    });
}
