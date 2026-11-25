//! The dialogue as it is drawn over the world: the line on show and the choices to pick from, as
//! free outlined text over a scrim at the bottom of the screen. What it shows comes in whole, so
//! the game's conversation and a tool that draws every style the dialogue sets draw it alike.

use ui::egui;

use bevy::prelude::*;
use ui::dusk::Tally;
use ui::egui::emath::TSTransform;
use ui::egui::{
    Align, Color32, FontId, Id, LayerId, Layout, Pos2, Rect, Response, Sense, Shape, Ui, UiBuilder,
    Vec2, WidgetInfo, WidgetType,
};
use ui::face;
use ui::glyph::Mark;
use ui::ink::{self, Ground, Prose};
use ui::motion::SheetPose;
use ui::paint::TextShadow;
use ui::{dusk, paint};

use super::span::{Picture, SpeakerName};
use super::{ChoiceChip, ChoiceView};
use crate::core::content::Content;
use crate::core::gui::Band;
use crate::core::text::{KeyNames, LineText, SpanText, StyledLine, TextLook};
use crate::systems::hud::WindowKey;
use crate::systems::item::ItemId;
use crate::systems::item::inline::{self, InlineItem};
use crate::systems::messages;

/// What the dialogue shows at a moment.
pub struct DialogueSheet {
    /// Who says the line, by the name it is led by: none for a narrator.
    pub speaker: Option<String>,
    pub line: Option<LineText>,
    /// How many of the line's typing units are out: all of them when unset.
    pub typed: Option<usize>,
    /// Whether the line waits for the player to go on, which a mark under it asks.
    pub waits: bool,
    pub choices: Vec<SheetChoice>,
    /// What else the sheet shows before the conversation moves on.
    pub to_come: SheetToCome,
}

/// What a sheet keeps room for from the moment it shows, so it never grows or shrinks under the
/// windows and cards set beside it: it stands as tall as the tallest of its lines over the choices
/// that follow them.
#[derive(Clone, Default)]
pub struct SheetToCome {
    /// Each line said in turn on the sheet, led by who says it.
    pub lines: Vec<(Option<String>, LineText)>,
    /// The choices that show once the last line is out.
    pub choices: Vec<ChoiceView>,
}

pub struct SheetChoice {
    pub view: ChoiceView,
    /// Whether the player is on it.
    pub lit: bool,
    /// Whether it was just picked and turned down.
    pub refused: bool,
}

/// Where the sheet landed, and what each part of it took of the pointer.
pub struct SheetDrawn {
    pub rect: Rect,
    /// The ground of the sheet, which takes a click anywhere off its choices.
    pub held: Response,
    pub rows: Vec<RowDrawn>,
    /// How tall the sheet stood, which the next pass lays it out at.
    pub height: f32,
    /// Where everything the sheet shows before the conversation moves on stands: every line of its
    /// step, the choices that end it with the mark beside the one the player is on, and the mark
    /// that asks the player to go on, as they will stand from the step's first letter.
    pub content: Rect,
}

pub struct RowDrawn {
    pub response: Response,
    /// The item of the tag under the pointer, since the row takes every click.
    pub pointed_item: Option<(ItemId, Rect)>,
}

pub fn layer() -> LayerId {
    LayerId::new(Band::Stage.order(), Id::new(WindowKey::Stage.name()))
}

/// Draws `sheet` on its layer at the bottom of the screen, posed as `pose` has it, `height` tall
/// as the pass before found it, on a scrim that darkens the bottom of the screen so the text reads
/// over any picture.
pub fn draw(
    world: &mut World,
    gui: &egui::Context,
    sheet: &DialogueSheet,
    pose: SheetPose,
    height: f32,
) -> SheetDrawn {
    let screen = gui.content_rect();
    let width = TEXT_WIDTH.min(screen.width() * SCREEN_SHARE);
    let bottom = screen.bottom() - screen.height() * BOTTOM_SHARE;
    let rect = Rect::from_min_size(
        Pos2::new(screen.center().x - width / 2.0, bottom - height),
        Vec2::new(width, height),
    );
    gui.set_transform_layer(layer(), pose.transform(gui, rect, rect.min));
    let drawn = egui::Area::new(layer().id)
        .order(Band::Stage.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            let scrim = ui.painter().add(Shape::Noop);
            let (outer, held) = ui.allocate_exact_size(rect.size(), Sense::click());
            let mut rows = Vec::new();
            let written = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            outer.min,
                            Vec2::new(outer.width(), f32::INFINITY),
                        ))
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = ROW_GAP;
                        line(
                            ui,
                            world,
                            sheet.speaker.clone(),
                            sheet.line.as_ref(),
                            sheet.typed,
                        );
                        rows = choices(ui, world, &sheet.choices);
                    },
                )
                .response
                .rect;
            let mark = continue_mark_size(ui);
            let needed = (written.bottom() - outer.top()).max(room(ui, world, sheet, outer)) + mark;
            if (needed - height).abs() > 0.5 {
                gui.request_discard("the conversation grew or shrank");
            }
            if sheet.waits {
                let under_line = Pos2::new(outer.right(), written.bottom() + mark);
                continue_mark(ui, Rect::from_min_max(outer.min, under_line));
            }
            let backing = Rect::from_min_max(
                Pos2::new(
                    screen.left(),
                    screen.bottom() - screen.height() * ink::DIALOGUE_BACKING_REACH,
                ),
                screen.right_bottom(),
            );
            ui.painter()
                .set(scrim, paint::fade_up(backing, ink::DIALOGUE_BACKING));
            let chooses = !sheet.choices.is_empty() || !sheet.to_come.choices.is_empty();
            let beside = if chooses { lit_mark_reach(ui) } else { 0.0 };
            let content = Rect::from_min_max(
                Pos2::new(rect.left() - beside, rect.top()),
                Pos2::new(rect.right(), rect.top() + needed),
            );
            SheetDrawn {
                rect,
                held,
                rows,
                height: needed,
                content,
            }
        });
    drawn.inner
}

const TEXT_WIDTH: f32 = 800.0;
/// How far apart the line and the choices under it stand.
const ROW_GAP: f32 = 12.0;
const SCREEN_SHARE: f32 = 0.94;
/// How far above the bottom of the viewport the lowest line of the dialogue stands.
const BOTTOM_SHARE: f32 = 0.12;
/// How tall the mark that asks the player to go on stands, in the dialogue's ems.
const CONTINUE_MARK: f32 = 0.6;
/// How far apart the tags of a choice stand, in the dialogue's ems: well apart from the room
/// within a tag, so each mark reads with its own tag.
const TAG_GAP: f32 = 1.1;
/// How wide a tag's mark stands and how far it keeps from the rest of its tag, in the dialogue's
/// ems.
const TAG_MARK: f32 = 0.8;
const TAG_MARK_GAP: f32 = 0.1;
/// How far apart the chips of a choice's trade stand, how far what it takes stands from what it
/// gives, and how wide a chip's sign stands, in the dialogue's ems.
const TALLY_GAP: f32 = 0.3;
const TALLY_GROUPS_GAP: f32 = 1.1;
const TALLY_SIGN: f32 = 0.6;
/// How far left of a choice the mark of the one the player is on stands, in the dialogue's ems.
const LIT_MARK_GAP: f32 = 0.3;
const LIT_MARK: &str = "\u{bb}";
const LINE_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);

fn line(
    ui: &mut Ui,
    world: &mut World,
    speaker: Option<String>,
    words: Option<&LineText>,
    typed: Option<usize>,
) {
    let Some(words) = words else {
        return;
    };
    let styled = said(world, speaker, words);
    egui::Frame::new().inner_margin(LINE_MARGIN).show(ui, |ui| {
        let look = text_look(ui, ink::DIALOGUE_LINE);
        inline::show_styled(ui, world, &styled, look, typed);
    });
}

/// How tall the sheet stands for all it shows before the conversation moves on: its tallest line,
/// whole, over every choice to come, each laid out unseen at the width of `outer`.
fn room(ui: &mut Ui, world: &mut World, sheet: &DialogueSheet, outer: Rect) -> f32 {
    let unseen = |ui: &mut Ui, salt: usize| {
        ui.new_child(
            UiBuilder::new()
                .id_salt(("room", salt))
                .max_rect(Rect::from_min_size(
                    outer.min,
                    Vec2::new(outer.width(), f32::INFINITY),
                ))
                .layout(Layout::top_down(Align::Min))
                .invisible(),
        )
    };
    let tallest = std::iter::once((sheet.speaker.clone(), sheet.line.clone()))
        .chain(
            sheet
                .to_come
                .lines
                .iter()
                .map(|(speaker, words)| (speaker.clone(), Some(words.clone()))),
        )
        .enumerate()
        .map(|(salt, (speaker, words))| {
            let mut laid = unseen(ui, salt);
            line(&mut laid, world, speaker, words.as_ref(), None);
            laid.min_rect().height()
        })
        .fold(0.0, f32::max);
    if sheet.to_come.choices.is_empty() {
        return tallest;
    }
    let mut laid = unseen(ui, usize::MAX);
    let to_come: Vec<SheetChoice> = sheet
        .to_come
        .choices
        .iter()
        .map(|view| SheetChoice {
            view: view.clone(),
            lit: false,
            refused: false,
        })
        .collect();
    choices(&mut laid, world, &to_come);
    tallest + ROW_GAP + laid.min_rect().height()
}

/// The line as the player reads it, led by its speaker's name wherever the catalog's message puts
/// it. The whole message is laid out, so the line wraps for good from its first letter, and only
/// the spoken words type out.
fn said(world: &World, speaker: Option<String>, words: &LineText) -> StyledLine {
    let content = world.resource::<Content>();
    let words = LineText(vec![SpanText::Typed(words.0.clone())]);
    let Some(name) = speaker else {
        return words.styled(content);
    };
    messages::of(content)
        .dialogue
        .spoken(SpanText::from(SpeakerName(name)), words)
        .styled(content)
}

/// A conversation's text over the world: large for the viewport, in `color`, and outlined so it
/// reads over any scene.
fn text_look(ui: &Ui, color: Color32) -> TextLook {
    let size = text_size(ui);
    TextLook {
        line_height: Some(size * face::DIALOGUE_LEADING),
        ..TextLook::new(size, color)
            .on(Ground::Dusk)
            .medium()
            .shadowed(dusk::OUTLINED)
            .shadows_reaching(dusk::outline_reach(size))
    }
}

fn text_size(ui: &Ui) -> f32 {
    ui.ctx().content_rect().height() * face::DIALOGUE_SIZE_SHARE
}

fn choices(ui: &mut Ui, world: &mut World, choices: &[SheetChoice]) -> Vec<RowDrawn> {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let color = match (choice.refused, choice.view.locked, choice.lit) {
                    (true, ..) => dusk::DANGER,
                    (false, true, _) => ink::DIALOGUE_CHOICE_LOCKED,
                    (false, false, true) => ink::DIALOGUE_CHOICE_LIT,
                    (false, false, false) => ink::DIALOGUE_CHOICE,
                };
                let row = ui.push_id(("choice", index), |ui| {
                    choice_frame(ui, |ui| choice_row(ui, world, &choice.view, index, color))
                });
                let (label, pointed_item) = row.inner.inner;
                let response = row.inner.response;
                if choice.lit {
                    lit_mark(ui, response.rect, color);
                }
                response.widget_info(|| {
                    WidgetInfo::selected(WidgetType::SelectableLabel, true, choice.lit, &label)
                });
                RowDrawn {
                    response,
                    pointed_item,
                }
            })
            .collect()
    })
    .inner
}

/// A row of choices that takes the pointer, with nothing painted behind it.
fn choice_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let width = ui.available_width();
    ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        ui.set_width(width);
        egui::Frame::new()
            .inner_margin(LINE_MARGIN)
            .show(ui, add)
            .inner
    })
}

/// A choice numbered by its place in the list, with its tags.
fn choice_row(
    ui: &mut Ui,
    world: &mut World,
    choice: &ChoiceView,
    index: usize,
    color: Color32,
) -> (String, Option<(ItemId, Rect)>) {
    let content = world.resource::<Content>().clone();
    let pictured = match &choice.icon {
        Some(icon) => LineText(
            std::iter::once(Picture(icon.clone()).into())
                .chain(choice.label.0.iter().cloned())
                .collect(),
        ),
        None => choice.label.clone(),
    };
    let listed = messages::of(&content)
        .dialogue
        .choice(index as u32 + 1, pictured);
    let label = choice.label.words(&content, world.resource::<KeyNames>());
    let mut chip_item = None;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = text_size(ui) * TAG_GAP;
            chip_item = tags(ui, world, choice);
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                inline::show_line_with_tooltips(ui, world, &listed, text_look(ui, color));
            });
        });
    });
    (label, chip_item)
}

/// The mark left of the choice the player is on, drawn outside the column so lighting a row moves
/// nothing.
fn lit_mark(ui: &Ui, row: Rect, color: Color32) {
    let size = text_size(ui);
    let (galley, drawn) = lit_mark_galley(ui, color);
    let first_line = row.top() + f32::from(LINE_MARGIN.top) + size * face::DIALOGUE_LEADING / 2.0;
    let centre = Pos2::new(
        row.left() - size * LIT_MARK_GAP - drawn.width() / 2.0,
        first_line,
    );
    let at = centre - drawn.center().to_vec2();
    dusk::lettering(
        ui.painter(),
        at,
        galley,
        &outline(ui),
        TSTransform::IDENTITY,
    );
}

/// How far left of the column the mark of the choice the player is on reaches.
fn lit_mark_reach(ui: &Ui) -> f32 {
    let (_, drawn) = lit_mark_galley(ui, ink::DIALOGUE_CHOICE_LIT);
    text_size(ui) * LIT_MARK_GAP + drawn.width()
}

/// The mark of the choice the player is on, laid out, and the part of it its letter covers.
fn lit_mark_galley(ui: &Ui, color: Color32) -> (std::sync::Arc<egui::Galley>, Rect) {
    let font = FontId::new(text_size(ui), face::Face::Book.family());
    let galley = ui
        .painter()
        .layout_no_wrap(LIT_MARK.to_owned(), font, color);
    let drawn = if galley.mesh_bounds.is_positive() {
        galley.mesh_bounds
    } else {
        galley.rect
    };
    (galley, drawn)
}

/// What a choice needs, pays and gets, as tags set in the dialogue's own outlined text: what it
/// needs as a mark and its words, and what it trades as chips, what it takes kept apart from what
/// it gives.
fn tags(ui: &mut Ui, world: &mut World, choice: &ChoiceView) -> Option<(ItemId, Rect)> {
    let mut pointed = None;
    if let Some(warn) = &choice.warn {
        tag(
            ui,
            world,
            None,
            &LineText::plain(warn.clone()),
            dusk::DANGER,
        );
    }
    let mut gets_laid = false;
    for shown in choice.chips.iter().rev() {
        match shown {
            ChoiceChip::Needs { what, met } => {
                let (mark, color) = if *met {
                    (Mark::Check, Prose::Name.on(Ground::Dusk))
                } else {
                    (Mark::Lock, dusk::DANGER)
                };
                tag(ui, world, Some(mark), what, color);
            }
            ChoiceChip::Pays { item, count, have } => {
                if std::mem::take(&mut gets_laid) {
                    ui.add_space(text_size(ui) * (TALLY_GROUPS_GAP - TALLY_GAP));
                }
                let tally = if have >= count {
                    Tally::Gives
                } else {
                    Tally::Lacks
                };
                let paid = InlineItem::of(*item).count(*count).tallied();
                tallied(ui, world, tally, &paid.into());
            }
            ChoiceChip::Gets { item, count } => {
                gets_laid = true;
                let got = InlineItem::of(*item).count(*count).tallied();
                let response = tallied(ui, world, Tally::Gets, &got.into());
                if response.contains_pointer() {
                    pointed = Some((*item, response.rect));
                }
            }
        }
    }
    pointed
}

/// A chip of what a choice trades, laid from the right as the row's tags are: its line, then its
/// sign before it, in its tally's colour.
fn tallied(ui: &mut Ui, world: &mut World, tally: Tally, line: &LineText) -> Response {
    let size = text_size(ui);
    let color = tally.ink();
    let look = text_look(ui, color);
    let outline = outline(ui);
    let gap = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = size * TALLY_GAP;
    let response = dusk::tally(ui, tally, size, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = size * TAG_MARK_GAP;
            inline::show_line_with_tooltips(ui, world, line, look);
            let height = size * face::DIALOGUE_LEADING;
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(size * TALLY_SIGN, height), Sense::hover());
            let sign = Rect::from_center_size(rect.center(), Vec2::splat(size * TALLY_SIGN));
            dusk::mark(ui.painter(), tally.mark(), sign, color, &outline);
        });
    })
    .response;
    ui.spacing_mut().item_spacing.x = gap;
    response
}

/// A tag laid from the right, as the row's tags are: its line, then its mark before it.
fn tag(
    ui: &mut Ui,
    world: &mut World,
    mark: Option<Mark>,
    line: &LineText,
    color: Color32,
) -> Response {
    let size = text_size(ui);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = size * TAG_MARK_GAP;
        let response = inline::show_line_with_tooltips(ui, world, line, text_look(ui, color));
        if let Some(mark) = mark {
            let height = size * face::DIALOGUE_LEADING;
            let room = Vec2::new(size * TAG_MARK, height);
            let (rect, _) = ui.allocate_exact_size(room, Sense::hover());
            let rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
            dusk::mark(ui.painter(), mark, rect, color, &outline(ui));
        }
        response
    })
    .inner
}

fn continue_mark_size(ui: &Ui) -> f32 {
    text_size(ui) * CONTINUE_MARK
}

fn continue_mark(ui: &Ui, body: Rect) {
    let mark = Rect::from_min_max(body.max - Vec2::splat(continue_mark_size(ui)), body.max);
    dusk::mark(
        ui.painter(),
        Mark::CaretDown,
        mark,
        ink::DIALOGUE_LINE,
        &outline(ui),
    );
}

/// The outline round the dialogue's marks, reaching as far as round its letters.
fn outline(ui: &Ui) -> Vec<TextShadow> {
    dusk::reaching(dusk::OUTLINED, dusk::outline_reach(text_size(ui)))
}
