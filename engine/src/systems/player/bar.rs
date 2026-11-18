use bevy::prelude::*;
use ui::egui::{self, Align, Align2, Id, Order, Pos2, Rect, Sense, Ui, UiBuilder, Vec2};
use ui::face::{Face, TextRole};
use ui::glyph::{self, Halo};
use ui::{controls, globe, ink, paint, slip};

use crate::core::content::Content;
use crate::core::gui::{self, Tier};
use crate::core::stage::globe::{self as lit_globe, Globe};
use crate::systems::dialogue::stage;
use crate::systems::hud::menu::{self, MenuTabName};
use crate::systems::hud::{self, HudWidget};
use crate::systems::player::PlayerMessages;
use crate::systems::player::session;
use crate::systems::settings::UserSettings;
use crate::systems::skill::bar::{self, BarSlots};
use crate::systems::stat::{self, Vital};
use crate::systems::{class, messages, status};

pub const PLAYER_BAR: &str = "player_bar";

pub struct PlayerBarWidget;

impl HudWidget for PlayerBarWidget {
    fn fallback(&self, world: &World, screen: Rect) -> Pos2 {
        let mana = character_sheet(world).is_some_and(|sheet| sheet.mana.is_some());
        let size = Layout::of(Pos2::ZERO, BarSlots::resting_size(), mana)
            .whole
            .size();
        Pos2::new(
            screen.center().x - size.x / 2.0,
            screen.bottom() - size.y - BOTTOM,
        )
    }

    // The dialogue stands where the bar rests.
    fn hidden(&self, world: &World) -> bool {
        stage::leavable(world)
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let now = ui.input(|input| input.time);
        let sheet = current_sheet(world, now)?;
        let slots = BarSlots::shown(world, ui.ctx());
        let layout = Layout::of(ui.cursor().min, slots.size(), sheet.mana.is_some());
        ui.allocate_rect(layout.whole, Sense::hover());
        slip::paint(ui.painter(), layout.paper);
        if let Some(experience) = &sheet.experience {
            experience_row(ui, world, layout.experience, experience);
        }
        ui.scope_builder(UiBuilder::new().max_rect(layout.slots), |ui| {
            slots.show(ui, world);
        });
        bar::reshape_grip(ui, world, layout.held);
        vital_globe(ui, world, layout.health, Vital::Health, sheet.health, now);
        if let (Some(at), Some(mana)) = (layout.mana, sheet.mana) {
            vital_globe(ui, world, at, Vital::Mana, mana, now);
        }
        let outside = RowOutside::of(ui.ctx(), &layout);
        if let Some(gain) = sheet.gain {
            let content = world.resource::<Content>().clone();
            gain_float(
                ui,
                &messages::of(&content).player,
                gain,
                layout.experience.center_top().x,
                layout.paper.min.y,
                now,
            );
        }
        let points = points_button(ui.ctx(), world, &outside, sheet.points);
        let statuses = status_row(ui.ctx(), world, &outside);
        bar::rebind_notice(
            ui.ctx(),
            world,
            Pos2::new(layout.paper.center().x, outside.notice_foot),
        );
        match (points, statuses) {
            (Some(points), Some(statuses)) => Some(points.union(statuses)),
            (points, statuses) => points.or(statuses),
        }
    }
}

const BOTTOM: f32 = 8.0;
const GLOBE_OVER_PAPER: f32 = 14.0;
const BESIDE_GLOBE: f32 = 6.0;
const PADDING: Vec2 = Vec2::new(12.0, 6.0);
const EXPERIENCE_ROW: f32 = 12.0;
const EXPERIENCE_BAR: f32 = 7.0;
const ROW_GAP: f32 = 4.0;
const LABEL_GAP: f32 = 8.0;
const TENTHS: u32 = 10;
const LOW_HEALTH: f32 = 0.25;
const OUTSIDE_GAP: f32 = 7.0;
const OUTSIDE_ROW: f32 = 28.0;
// Kept clear for the points button whether or not it is there, so the status effects never
// reach where it comes to stand.
const POINTS_ROOM: f32 = 170.0;
const GAIN_SECONDS: f64 = 1.8;
const GAIN_RISE: f32 = 14.0;

/// Where the bar's parts lie: the globe of health to the left of the paper, the globe of mana,
/// for a character that has any, to its right, and what the paper holds between them. The globes
/// stand beside the paper's lowest row of slots.
struct Layout {
    whole: Rect,
    health: Pos2,
    mana: Option<Pos2>,
    paper: Rect,
    /// The paper as far as what it holds reaches: under a globe to its right it runs on.
    held: Rect,
    experience: Rect,
    slots: Rect,
}

impl Layout {
    fn of(origin: Pos2, slots: Vec2, mana: bool) -> Layout {
        let one_row = 2.0 * PADDING.y + EXPERIENCE_ROW + ROW_GAP + controls::SLOT;
        let below = ((globe::SIZE - one_row) / 2.0).max(0.0);
        let rise = (below - (slots.y - controls::SLOT)).max(0.0);
        let paper_at = origin + Vec2::new(globe::SIZE - GLOBE_OVER_PAPER, rise);
        let held_at = paper_at + Vec2::new(GLOBE_OVER_PAPER + BESIDE_GLOBE, PADDING.y);
        let experience = Rect::from_min_size(held_at, Vec2::new(slots.x, EXPERIENCE_ROW));
        let slots = Rect::from_min_size(held_at + Vec2::new(0.0, EXPERIENCE_ROW + ROW_GAP), slots);
        let held = Rect::from_min_max(paper_at, slots.max + PADDING);
        let middle = held.max.y - one_row / 2.0;
        let health = Pos2::new(origin.x + globe::SIZE / 2.0, middle);
        let mana = mana.then(|| Pos2::new(slots.max.x + BESIDE_GLOBE + globe::SIZE / 2.0, middle));
        let paper = match mana {
            Some(mana) => held.with_max_x(mana.x - globe::SIZE / 2.0 + GLOBE_OVER_PAPER),
            None => held,
        };
        let seat = |at: Pos2| Rect::from_center_size(at, Vec2::splat(globe::SIZE));
        Layout {
            whole: mana
                .map_or(paper, |mana| paper.union(seat(mana)))
                .union(seat(health)),
            health,
            mana,
            paper,
            held,
            experience,
            slots,
        }
    }
}

struct RowOutside {
    left: f32,
    right: f32,
    stands_on: f32,
    side: Align,
    notice_foot: f32,
}

impl RowOutside {
    fn of(gui: &egui::Context, layout: &Layout) -> RowOutside {
        let (left, right) = (layout.slots.min.x, layout.held.max.x);
        let over = layout.paper.min.y - OUTSIDE_GAP;
        if over - OUTSIDE_ROW >= gui.content_rect().top() {
            RowOutside {
                left,
                right,
                stands_on: over,
                side: Align::Max,
                notice_foot: over - OUTSIDE_ROW,
            }
        } else {
            RowOutside {
                left,
                right,
                stands_on: layout.paper.max.y + OUTSIDE_GAP,
                side: Align::Min,
                notice_foot: layout.whole.min.y,
            }
        }
    }

    fn area(&self, gui: &egui::Context, id: &str, end: Align) -> egui::Area {
        let (x, across) = match end {
            Align::Max => (self.right, Align::Max),
            Align::Min | Align::Center => (self.left, Align::Min),
        };
        let area = egui::Area::new(Id::new(id))
            .order(Order::Background)
            .pivot(Align2([across, self.side]))
            .fixed_pos(Pos2::new(x, self.stands_on))
            .constrain(false)
            .fade_in(false);
        gui::stack(gui, Tier::Mid, area.layer());
        area
    }
}

// Between areas there is no character for a moment; the bar keeps what it last showed.
#[derive(Resource, Default)]
struct LastSheet(Option<CharacterSheet>);

#[derive(Clone)]
struct CharacterSheet {
    who: Entity,
    health: Held,
    /// None for a character that has no mana to its name.
    mana: Option<Held>,
    xp: u32,
    experience: Option<Experience>,
    points: u32,
    gain: Option<Gain>,
}

/// How much of a vital a character holds, and how much it holds at most.
#[derive(Clone, Copy)]
struct Held {
    now: f32,
    most: f32,
}

impl Held {
    fn of(world: &World, who: Entity, vital: Vital) -> Held {
        Held {
            now: stat::current(world, who, vital),
            most: stat::most(world, who, vital),
        }
    }
}

#[derive(Clone)]
struct Experience {
    level: u32,
    /// None at the last level.
    next: Option<NextLevel>,
}

#[derive(Clone, Copy)]
struct NextLevel {
    had: u32,
    needed: u32,
}

#[derive(Clone, Copy)]
struct Gain {
    amount: u32,
    at: f64,
}

fn current_sheet(world: &mut World, now: f64) -> Option<CharacterSheet> {
    let fresh = character_sheet(world);
    let mut last = world.get_resource_or_init::<LastSheet>();
    if let Some(mut fresh) = fresh {
        fresh.gain = match &last.0 {
            Some(was) if was.who == fresh.who && fresh.xp > was.xp => Some(Gain {
                amount: fresh.xp - was.xp,
                at: now,
            }),
            Some(was) if was.who == fresh.who => was.gain,
            _ => None,
        };
        last.0 = Some(fresh);
    }
    last.0.clone()
}

fn character_sheet(world: &World) -> Option<CharacterSheet> {
    let me = session::me(world)?;
    let content = world.resource::<Content>();
    let xp = class::xp_of(world, me);
    let experience = class::of(world, me).map(|class| {
        let def = class.get(content);
        let from = def.reached(xp).map(|tier| tier.xp).max().unwrap_or(0);
        let to = def.levels.iter().map(|tier| tier.xp).filter(|at| *at > xp);
        Experience {
            level: def.level_at(xp),
            next: to.min().map(|to| NextLevel {
                had: xp - from,
                needed: to - from,
            }),
        }
    });
    Some(CharacterSheet {
        who: me,
        health: Held::of(world, me, Vital::Health),
        mana: Some(Held::of(world, me, Vital::Mana)).filter(|mana| mana.most > 0.0),
        xp,
        experience,
        points: class::points_to_spend(world, me).0,
        gain: None,
    })
}

/// The globe of one of the character's vitals, as full as the character is of it. It says how
/// much it holds in numbers under the pointer, or always when the player's settings ask for it.
fn vital_globe(ui: &mut Ui, world: &mut World, center: Pos2, vital: Vital, held: Held, now: f64) {
    let full = if held.most > 0.0 {
        (held.now / held.most).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).player;
    let (name, liquid, warns) = match vital {
        Vital::Health => (words.health(), ink::WAX, full > 0.0 && full < LOW_HEALTH),
        Vital::Mana => (words.mana(), ink::AZURE, false),
    };
    let (now_held, most_held) = (held.now.max(0.0).ceil(), held.most.round());
    let amount = words.vital_figures(now_held, most_held);
    let rect = Rect::from_center_size(center, Vec2::splat(globe::SIZE));
    let response = ui.interact(
        rect,
        ui.id().with(("vital", vital.is_health())),
        Sense::hover(),
    );
    let said = words.vital_amount(name, now_held, most_held);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
    let painter = ui.painter();
    globe::socket(painter, center);
    let lit = lit_globe::show(
        world,
        painter,
        &Globe {
            key: response.id,
            center,
            radius: globe::GLASS,
            full,
            liquid,
        },
    );
    if !lit {
        globe::flat(painter, center, full, liquid);
    }
    globe::rim(painter, center, warns, now);
    if response.hovered() || world.resource::<UserSettings>().globe_amounts_shown() {
        globe::amount(painter, center, &amount);
    }
    hud::tooltip(&response, world, name, &[], None);
}

fn experience_row(ui: &mut Ui, world: &World, row: Rect, experience: &Experience) {
    let content = world.resource::<Content>();
    let words = &messages::of(content).player;
    let level = words.level_of(experience.level);
    let (share, toward) = match experience.next {
        Some(next) => (
            next.had as f32 / next.needed.max(1) as f32,
            words.toward(next.had, next.needed, experience.level + 1),
        ),
        None => (1.0, words.last_level().to_owned()),
    };
    let response = ui.interact(row, ui.id().with("experience"), Sense::hover());
    let said = format!("{level} · {toward}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
    let label = ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        &level,
        TextRole::Section.font_id(),
        ink::INK2,
    );
    let bar = Rect::from_min_max(
        Pos2::new(
            label.max.x + LABEL_GAP,
            row.center().y - EXPERIENCE_BAR / 2.0,
        ),
        Pos2::new(row.max.x, row.center().y + EXPERIENCE_BAR / 2.0),
    );
    paint::ticked_bar(ui.painter(), bar, share, ink::GOLD, TENTHS);
    hud::tooltip(&response, world, words.experience(), &[toward.into()], None);
}

fn gain_float(ui: &Ui, words: &PlayerMessages, gain: Gain, x: f32, paper_top: f32, now: f64) {
    let age = ((now - gain.at) / GAIN_SECONDS) as f32;
    if !(0.0..1.0).contains(&age) {
        return;
    }
    let fade = 1.0 - age * age;
    glyph::haloed(
        ui.painter(),
        Pos2::new(x, paper_top - 2.0 - GAIN_RISE * age),
        Align2::CENTER_BOTTOM,
        &words.experience_gained(gain.amount),
        Face::BookBold.id(TextRole::Small.size()),
        ink::GOLD2.gamma_multiply(fade),
        Halo {
            color: ink::FRAME.gamma_multiply(fade),
            width: 1.0,
        },
    );
    ui.ctx().request_repaint();
}

fn points_button(
    gui: &egui::Context,
    world: &mut World,
    outside: &RowOutside,
    points: u32,
) -> Option<Rect> {
    if points == 0 {
        return None;
    }
    let content = world.resource::<Content>().clone();
    let said = messages::of(&content).player.points_to_spend(points);
    let shown = outside
        .area(gui, "player bar points", Align::Min)
        .show(gui, |ui| controls::notice(ui, &said));
    if shown.inner.clicked() {
        menu::open_on(world, MenuTabName::Skills);
    }
    Some(shown.response.rect)
}

fn status_row(gui: &egui::Context, world: &mut World, outside: &RowOutside) -> Option<Rect> {
    let width = (outside.right - outside.left - POINTS_ROOM).max(0.0);
    let shown = outside
        .area(gui, "player bar statuses", Align::Max)
        .show(gui, |ui| {
            status::widget::row(ui, world, width, outside.side)
        });
    let rect = shown.response.rect;
    rect.is_positive().then_some(rect)
}
