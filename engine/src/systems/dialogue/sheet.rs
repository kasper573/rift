//! The dialogue as it is drawn over the world: the line on show and the choices to pick from, as
//! free outlined text over a scrim at the bottom of the screen. What it shows comes in whole, so
//! the game's conversation and a tool that draws every style the dialogue sets draw it alike.

use ui::egui;

use bevy::prelude::*;
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
                        ui.spacing_mut().item_spacing.y = 12.0;
                        line(ui, world, sheet);
                        rows = choices(ui, world, &sheet.choices);
                    },
                )
                .response
                .rect;
            let needed = written.bottom() - outer.top() + continue_mark_size(ui);
            if (needed - height).abs() > 0.5 {
                gui.request_discard("the conversation grew or shrank");
            }
            if sheet.waits {
                continue_mark(ui, outer);
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
            SheetDrawn {
                rect,
                held,
                rows,
                height: needed,
            }
        });
    drawn.inner
}

const TEXT_WIDTH: f32 = 800.0;
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
/// How far left of a choice the mark of the one the player is on stands, in the dialogue's ems.
const LIT_MARK_GAP: f32 = 0.3;
const LIT_MARK: &str = "\u{bb}";
const LINE_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);

fn line(ui: &mut Ui, world: &mut World, sheet: &DialogueSheet) {
    let Some(words) = &sheet.line else {
        return;
    };
    let styled = said(world, sheet.speaker.clone(), words);
    egui::Frame::new().inner_margin(LINE_MARGIN).show(ui, |ui| {
        let look = text_look(ui, ink::DIALOGUE_LINE);
        inline::show_styled(ui, world, &styled, look, sheet.typed);
    });
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
    let font = FontId::new(size, face::Face::Book.family());
    let galley = ui
        .painter()
        .layout_no_wrap(LIT_MARK.to_owned(), font, color);
    let drawn = if galley.mesh_bounds.is_positive() {
        galley.mesh_bounds
    } else {
        galley.rect
    };
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

/// What a choice needs, pays and gets, as coloured tags set in the dialogue's own outlined text.
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
                let color = if have >= count {
                    Prose::Item.on(Ground::Dusk)
                } else {
                    dusk::DANGER
                };
                let paid = InlineItem::of(*item).count(*count).nameless();
                tag(ui, world, None, &paid.into(), color);
            }
            ChoiceChip::Gets { item, count } => {
                let got = InlineItem::of(*item).count(*count).nameless();
                let color = Prose::Name.on(Ground::Dusk);
                let response = tag(ui, world, Some(Mark::Plus), &got.into(), color);
                if response.contains_pointer() {
                    pointed = Some((*item, response.rect));
                }
            }
        }
    }
    pointed
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
