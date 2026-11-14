use bevy::prelude::*;
use ui::egui::emath::TSTransform;
use ui::egui::text::{LayoutJob, TextFormat};
use ui::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use ui::face::{Face, TextRole};
use ui::glyph::Mark;
use ui::{dusk, ink};

use super::card::Countdown;
use super::{Parties, QuestId, QuestLog, QuestResult, ready_mark};
use crate::core::content::ImageRef;
use crate::core::content::{Content, Fixture};
use crate::core::gui;
use crate::systems::area_map::minimap;
use crate::systems::hud::{self, HudWidget};
use crate::systems::item::Inventory;
use crate::systems::player::session;
use crate::systems::settings::UserSettings;

const WIDTH: f32 = 244.0;
const UNDER_THE_MAP: f32 = 6.0;
const HEADING: f32 = 20.0;
const MARK: f32 = 16.0;
const DIAMOND: f32 = 4.0;
const CARET: f32 = 4.5;
const INDENT: f32 = 16.0;
const ROW_GAP: f32 = 2.0;
const ENTRY_GAP: f32 = 6.0;
const PLATE_PADDING: f32 = 8.0;
const MARK_GAP: f32 = 4.0;

pub const QUEST_TRACKER: &str = "quest_tracker";

pub struct QuestTrackerPlugin;

impl Plugin for QuestTrackerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Folded>();
    }
}

pub struct QuestTrackerWidget;

impl HudWidget for QuestTrackerWidget {
    fn fallback(&self, world: &World, screen: Rect) -> Pos2 {
        Pos2::new(
            screen.right() - WIDTH - hud::MARGIN,
            screen.top() + hud::MARGIN + minimap::resting_size(world).y + UNDER_THE_MAP,
        )
    }

    // Only the heading takes room, so only it takes the pointer: a click on what is listed under
    // it goes on to the world.
    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let entries = entries(world);
        if entries.is_empty() {
            return None;
        }
        let log = session::me(world)
            .and_then(|seen| world.get::<QuestLog>(seen))
            .cloned()
            .unwrap_or_default();
        let folded = world.resource::<Folded>().0;
        let backing = ui.painter().add(Shape::Noop);
        let heading = heading(ui, world, entries.len(), folded);
        let mut foot = heading.bottom();
        if !folded {
            for entry in &entries {
                foot = entry_rows(ui, world, &log, entry, Pos2::new(heading.left(), foot));
            }
        }
        let drawn = Rect::from_min_max(heading.min, Pos2::new(heading.right(), foot));
        let opacity = world.resource::<UserSettings>().tracker_backing();
        ui.painter().set(
            backing,
            dusk::plate_shape(drawn.expand(PLATE_PADDING), opacity.0 / 100.0),
        );
        Some(drawn)
    }
}

#[derive(Resource, Default)]
struct Folded(bool);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Plain,
    Done,
    Timed,
    Bad,
}

struct Entry {
    quest: QuestId,
    mark: Option<ImageRef>,
    timed: bool,
    rows: Vec<(String, Count, Tone)>,
}

enum Count {
    Hidden,
    Fixed(String),
    Live(Countdown),
}

impl Tone {
    // A tint goes with what the row's mark and words already say, and stays pale enough to read
    // on the edge every letter carries.
    fn color(self) -> Color32 {
        match self {
            Tone::Plain => Color32::WHITE,
            Tone::Done => Color32::from_rgb(0xDA, 0xF5, 0xDD),
            Tone::Timed => Color32::from_rgb(0xFF, 0xEB, 0xB8),
            Tone::Bad => Color32::from_rgb(0xFF, 0xD9, 0xD2),
        }
    }

    fn mark(self) -> Option<Mark> {
        match self {
            Tone::Plain => None,
            Tone::Done => Some(Mark::Check),
            Tone::Timed => Some(Mark::Hourglass),
            Tone::Bad => Some(Mark::Cross),
        }
    }
}

fn heading(ui: &mut Ui, world: &mut World, count: usize, folded: bool) -> Rect {
    let (rect, pressed) = ui.allocate_exact_size(Vec2::new(WIDTH, HEADING), Sense::click());
    let title = if folded {
        format!("Quests ({count})")
    } else {
        "Quests".to_owned()
    };
    let said = if folded {
        format!("{title} · show tracked quests")
    } else {
        format!("{title} · hide tracked quests")
    };
    pressed.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, said.clone()));
    if pressed.clicked() {
        world.resource_mut::<Folded>().0 = !folded;
    }
    let galley = ui
        .painter()
        .layout_no_wrap(title, TextRole::Section.font_id(), ink::GOLD2);
    let at = Align2::LEFT_CENTER.anchor_size(rect.left_center(), galley.size());
    dusk::lettering(
        ui.painter(),
        at.min,
        galley,
        dusk::EDGED,
        TSTransform::IDENTITY,
    );
    let tip = if folded { Vec2::RIGHT } else { Vec2::DOWN };
    caret(ui, rect.right_center() - Vec2::new(CARET + 2.0, 0.0), tip);
    rect
}

fn caret(ui: &Ui, at: Pos2, tip: Vec2) {
    let across = tip.rot90();
    let shape = |grown: f32, color: Color32| {
        let size = CARET + grown;
        Shape::convex_polygon(
            vec![
                at + tip * size,
                at - tip * size * 0.6 + across * size,
                at - tip * size * 0.6 - across * size,
            ],
            color,
            Stroke::NONE,
        )
    };
    ui.painter().add(shape(1.5, Color32::BLACK));
    ui.painter().add(shape(0.0, ink::GOLD2));
}

fn entry_rows(ui: &mut Ui, world: &mut World, log: &QuestLog, entry: &Entry, at: Pos2) -> f32 {
    let content = world.resource::<Content>().clone();
    let mut top = at.y + ENTRY_GAP;
    let diamond = Pos2::new(at.x + DIAMOND + 2.0, top + TextRole::Body.size() * 0.62);
    for (grown, color) in [(1.5, Color32::BLACK), (0.0, ink::GOLD2)] {
        let corners = [Vec2::UP, Vec2::RIGHT, Vec2::DOWN, Vec2::LEFT]
            .map(|corner| diamond + corner * (DIAMOND + grown))
            .to_vec();
        ui.painter()
            .add(Shape::convex_polygon(corners, color, Stroke::NONE));
    }
    let mut left = at.x + INDENT;
    if let Some(image) = entry.mark.as_ref().and_then(|mark| gui::image(world, mark)) {
        let spot = Rect::from_min_size(Pos2::new(left, top + 1.0), Vec2::splat(MARK));
        ui.put(spot, image.widget(Vec2::splat(MARK)));
        left += MARK + 4.0;
    }
    let title = entry.quest.get(&content).title.clone();
    let id = ui.id().with(("tracked", entry.quest.index()));
    top = Line {
        at: Pos2::new(left, top),
        width: at.x + WIDTH - left,
        mark: None,
        text: &title,
        font: Face::BookBold.id(TextRole::Body.size()),
        color: Color32::WHITE,
    }
    .letter(ui, id.with("title"))
    .bottom();
    let rows = entry.rows.iter().map(|(label, count, tone)| {
        let count = match count {
            Count::Hidden => None,
            Count::Fixed(count) => Some(count.clone()),
            Count::Live(shows) => shows.label(log, entry.quest, super::card::clock_now()),
        };
        let said = match count {
            Some(count) => format!("{label} {count}"),
            None => label.clone(),
        };
        (said, *tone)
    });
    let left_on_the_clock = entry
        .timed
        .then(|| Countdown::LeftDetail.label(log, entry.quest, super::card::clock_now()))
        .flatten()
        .map(|left| (left, Tone::Timed));
    for (index, (said, tone)) in rows.chain(left_on_the_clock).enumerate() {
        top = Line {
            at: Pos2::new(at.x + INDENT, top + ROW_GAP),
            width: WIDTH - INDENT,
            mark: tone.mark(),
            text: &said,
            font: TextRole::Small.font_id(),
            color: tone.color(),
        }
        .letter(ui, id.with(index))
        .bottom();
    }
    top
}

struct Line<'a> {
    at: Pos2,
    width: f32,
    mark: Option<Mark>,
    text: &'a str,
    font: egui::FontId,
    color: Color32,
}

impl Line<'_> {
    fn letter(self, ui: &mut Ui, id: egui::Id) -> Rect {
        let mut job = LayoutJob::default();
        job.wrap.max_width = self.width;
        if let Some(mark) = self.mark {
            let icon = mark.icon();
            let icons = egui::FontId::new(self.font.size, icon.weight.family());
            let format = TextFormat {
                valign: egui::Align::Center,
                ..TextFormat::simple(icons, self.color)
            };
            job.append(icon.glyph, 0.0, format);
        }
        let lead = if self.mark.is_some() { MARK_GAP } else { 0.0 };
        job.append(self.text, lead, TextFormat::simple(self.font, self.color));
        let galley = ui.painter().layout_job(job);
        let rect = Rect::from_min_size(self.at, galley.size());
        ui.interact(rect, id, Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, self.text));
        dusk::lettering(
            ui.painter(),
            self.at,
            galley,
            dusk::EDGED,
            TSTransform::IDENTITY,
        );
        rect
    }
}

fn entries(world: &World) -> Vec<Entry> {
    let content = world.resource::<Content>();
    let parties = world.resource::<Parties>();
    let seen = session::me(world);
    let Some(log) = seen.and_then(|seen| world.get::<QuestLog>(seen)) else {
        return Vec::new();
    };
    let inventory = seen.and_then(|seen| world.get::<Inventory>(seen));
    log.tracked
        .iter()
        .filter_map(|&quest| {
            let def = quest.get(content);
            if let Some(active) = log.active(quest) {
                let timed = active.deadline.is_some();
                if active.ready(content, parties, inventory) && def.decision().is_none() {
                    return Some(Entry {
                        quest,
                        mark: Some(ready_mark(def).get(content).icon.clone()),
                        timed,
                        rows: vec![(def.hand_in_hint(parties, quest), Count::Hidden, Tone::Done)],
                    });
                }
                let progress = def.progress(content, parties, active, inventory);
                let rows = progress
                    .into_iter()
                    .map(|progress| {
                        let count = if progress.counted {
                            Count::Fixed(format!("{} of {}", progress.have, progress.need))
                        } else {
                            Count::Hidden
                        };
                        let tone = if progress.done() {
                            Tone::Done
                        } else {
                            Tone::Plain
                        };
                        (progress.label, count, tone)
                    })
                    .collect();
                return Some(Entry {
                    quest,
                    mark: None,
                    timed,
                    rows,
                });
            }
            let finished = log.finished(quest)?;
            let rows = match finished.result {
                QuestResult::Completed => vec![(
                    "Done for today".to_owned(),
                    Count::Live(Countdown::Resets),
                    Tone::Done,
                )],
                QuestResult::Failed => vec![
                    ("Failed".to_owned(), Count::Hidden, Tone::Bad),
                    (def.retry_hint(parties, quest), Count::Hidden, Tone::Plain),
                ],
            };
            Some(Entry {
                quest,
                mark: None,
                timed: false,
                rows,
            })
        })
        .collect()
}
