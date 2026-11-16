use bevy::prelude::*;
use ui::egui::{Color32, Pos2, Rect, Ui, UiBuilder, Vec2};
use ui::face::{Face, TextRole};
use ui::ink::Ground;
use ui::tracker::{self, INDENT, TrackerHeading, TrackerLine, TrackerTone, WIDTH};

use super::card::Countdown;
use super::{Parties, QuestId, QuestLog, QuestResult, ready_mark};
use crate::core::content::ImageRef;
use crate::core::content::{Content, Fixture};
use crate::core::gui;
use crate::core::text::{self, TextLook};
use crate::systems::area_map::minimap;
use crate::systems::hud::{self, HudWidget};
use crate::systems::item::Inventory;
use crate::systems::player::session;
use crate::systems::settings::UserSettings;
use crate::systems::text::LineText;

const UNDER_THE_MAP: f32 = 6.0;
const MARK: f32 = 16.0;
/// The dark ring that keeps a row's letters readable over the world.
const HALO: Color32 = Color32::from_rgba_premultiplied(10, 7, 5, 210);

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
        let opacity = world.resource::<UserSettings>().tracker_backing();
        let heading = TrackerHeading {
            title: "Quests",
            listed: entries.len(),
            lists: "tracked quests",
        };
        let shown = tracker::show(ui, heading, folded, opacity.0 / 100.0, |ui, at| {
            entries.iter().fold(at.y, |foot, entry| {
                entry_rows(ui, world, &log, entry, Pos2::new(at.x, foot))
            })
        });
        if shown.toggled {
            world.resource_mut::<Folded>().0 = !folded;
        }
        Some(shown.drawn)
    }
}

#[derive(Resource, Default)]
struct Folded(bool);

struct Entry {
    quest: QuestId,
    mark: Option<ImageRef>,
    timed: bool,
    rows: Vec<(LineText, Count, TrackerTone)>,
}

enum Count {
    Hidden,
    Fixed(String),
    Live(Countdown),
}

fn entry_rows(ui: &mut Ui, world: &mut World, log: &QuestLog, entry: &Entry, at: Pos2) -> f32 {
    let content = world.resource::<Content>().clone();
    let mut top = at.y + tracker::ENTRY_GAP;
    tracker::bullet(ui, Pos2::new(at.x, top));
    let mut left = at.x + INDENT;
    if let Some(image) = entry.mark.as_ref().and_then(|mark| gui::image(world, mark)) {
        let spot = Rect::from_min_size(Pos2::new(left, top + 1.0), Vec2::splat(MARK));
        ui.put(spot, image.widget(Vec2::splat(MARK)));
        left += MARK + 4.0;
    }
    let title = entry.quest.get(&content).title.clone();
    let id = ui.id().with(("tracked", entry.quest.index()));
    top = TrackerLine {
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
            Some(count) => label.clone().then(format!(" {count}")),
            None => label.clone(),
        };
        (said, *tone)
    });
    let left_on_the_clock = entry
        .timed
        .then(|| Countdown::LeftDetail.label(log, entry.quest, super::card::clock_now()))
        .flatten()
        .map(|left| (LineText::plain(left), TrackerTone::Timed));
    for (said, tone) in rows.chain(left_on_the_clock) {
        let at = Pos2::new(at.x + INDENT, top + tracker::ROW_GAP);
        top = row(ui, world, at, &said, tone);
    }
    top
}

/// Letters a row of an entry from `at`, behind its tone's mark, and tells where it ends: what
/// it names shows the way the interface mentions it over the world.
fn row(ui: &mut Ui, world: &World, at: Pos2, said: &LineText, tone: TrackerTone) -> f32 {
    let size = TextRole::Small.size();
    let lead = tone
        .mark()
        .map_or(0.0, |mark| tracker::mark(ui, at, mark, tone.color(), size));
    let room = Rect::from_min_max(
        Pos2::new(at.x + lead, at.y),
        Pos2::new(at.x + WIDTH - INDENT, ui.max_rect().bottom().max(at.y)),
    );
    let look = TextLook::role(TextRole::Small, tone.color())
        .on(Ground::Dusk)
        .haloed(HALO);
    let mut within = ui.new_child(UiBuilder::new().max_rect(room));
    text::show(&mut within, world, &said.styled(), look, None)
        .rect
        .bottom()
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
                        rows: vec![(
                            def.hand_in_hint(content, parties, quest),
                            Count::Hidden,
                            TrackerTone::Done,
                        )],
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
                            TrackerTone::Done
                        } else {
                            TrackerTone::Plain
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
                    "Done for today".into(),
                    Count::Live(Countdown::Resets),
                    TrackerTone::Done,
                )],
                QuestResult::Failed => vec![
                    ("Failed".into(), Count::Hidden, TrackerTone::Bad),
                    (
                        def.retry_hint(content, parties, quest),
                        Count::Hidden,
                        TrackerTone::Plain,
                    ),
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
