use bevy::prelude::*;
use ui::egui::{self, Align, Align2, Id, Order, Pos2, Rect, Sense, Ui, UiBuilder, Vec2};
use ui::face::{Face, TextRole};
use ui::glyph::{self, Halo};
use ui::vial::{self, Vial};
use ui::{controls, ink, paint, slip, words};

use crate::core::content::Content;
use crate::core::gui::{self, Tier};
use crate::systems::dialogue::stage;
use crate::systems::hud::menu::{self, MenuTabName};
use crate::systems::hud::{self, HudWidget};
use crate::systems::player::session;
use crate::systems::skill::bar::{self, BarSlots};
use crate::systems::{class, stat, status};

pub const PLAYER_BAR: &str = "player_bar";

pub struct PlayerBarWidget;

impl HudWidget for PlayerBarWidget {
    fn fallback(&self, screen: Rect) -> Pos2 {
        let size = Layout::of(Pos2::ZERO, BarSlots::resting_size())
            .vial_and_paper
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
        let layout = Layout::of(ui.cursor().min, slots.size());
        ui.allocate_rect(layout.vial_and_paper, Sense::hover());
        slip::paint(ui.painter(), layout.paper);
        if let Some(experience) = &sheet.experience {
            experience_row(ui, world, layout.experience, experience);
        }
        ui.scope_builder(UiBuilder::new().max_rect(layout.slots), |ui| {
            slots.show(ui, world);
        });
        bar::reshape_grip(ui, world, layout.paper);
        health_vial(ui, world, layout.vial, &sheet, now);
        let outside = RowOutside::of(ui.ctx(), &layout);
        if let Some(gain) = sheet.gain {
            gain_float(
                ui,
                gain,
                layout.experience.center_top().x,
                layout.paper.min.y,
                now,
            );
        }
        let points = points_button(ui.ctx(), world, &outside, sheet.points);
        let statuses = status_row(ui.ctx(), world, &outside);
        bar::rebind_notice(
            ui,
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
const VIAL_OVER_PAPER: f32 = 14.0;
const BESIDE_VIAL: f32 = 6.0;
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

struct Layout {
    vial_and_paper: Rect,
    vial: Pos2,
    paper: Rect,
    experience: Rect,
    slots: Rect,
}

impl Layout {
    fn of(origin: Pos2, slots: Vec2) -> Layout {
        let one_row = 2.0 * PADDING.y + EXPERIENCE_ROW + ROW_GAP + controls::SLOT;
        let rise = ((vial::SIZE - one_row) / 2.0).max(0.0);
        let paper_at = origin + Vec2::new(vial::SIZE - VIAL_OVER_PAPER, rise);
        let held_at = paper_at + Vec2::new(VIAL_OVER_PAPER + BESIDE_VIAL, PADDING.y);
        let experience = Rect::from_min_size(held_at, Vec2::new(slots.x, EXPERIENCE_ROW));
        let slots = Rect::from_min_size(held_at + Vec2::new(0.0, EXPERIENCE_ROW + ROW_GAP), slots);
        let paper = Rect::from_min_max(paper_at, slots.max + PADDING);
        let vial = Pos2::new(origin.x + vial::SIZE / 2.0, paper_at.y + one_row / 2.0);
        Layout {
            vial_and_paper: paper.union(Rect::from_center_size(vial, Vec2::splat(vial::SIZE))),
            vial,
            paper,
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
        let (left, right) = (layout.slots.min.x, layout.paper.max.x);
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
                notice_foot: layout.vial_and_paper.min.y,
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
    health: f32,
    max_health: f32,
    xp: u32,
    experience: Option<Experience>,
    points: u32,
    gain: Option<Gain>,
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
        health: stat::current_health(world, me),
        max_health: stat::max_health(world, me),
        xp,
        experience,
        points: class::points_to_spend(world, me).0,
        gain: None,
    })
}

fn health_vial(ui: &mut Ui, world: &World, center: Pos2, sheet: &CharacterSheet, now: f64) {
    let share = if sheet.max_health > 0.0 {
        (sheet.health / sheet.max_health).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let amount = format!("{:.0}", sheet.health.max(0.0).ceil());
    let of = format!("of {:.0}", sheet.max_health);
    let rect = Rect::from_center_size(center, Vec2::splat(vial::SIZE));
    let response = ui.interact(rect, ui.id().with("health"), Sense::hover());
    let said = format!("Health {amount} {of}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
    vial::paint(
        ui.painter(),
        center,
        &Vial {
            full: share,
            amount: &amount,
            of: &of,
            low: share > 0.0 && share < LOW_HEALTH,
        },
        now,
    );
    hud::tooltip(
        &response,
        world,
        "Health",
        &[format!("{amount} {of}")],
        None,
    );
}

fn experience_row(ui: &mut Ui, world: &World, row: Rect, experience: &Experience) {
    let level = format!("Level {}", experience.level);
    let (share, toward) = match experience.next {
        Some(next) => (
            next.had as f32 / next.needed.max(1) as f32,
            format!(
                "{} of {} to level {}",
                next.had,
                next.needed,
                experience.level + 1
            ),
        ),
        None => (1.0, "The last level".to_owned()),
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
    hud::tooltip(&response, world, "Experience", &[toward], None);
}

fn gain_float(ui: &Ui, gain: Gain, x: f32, paper_top: f32, now: f64) {
    let age = ((now - gain.at) / GAIN_SECONDS) as f32;
    if !(0.0..1.0).contains(&age) {
        return;
    }
    let fade = 1.0 - age * age;
    glyph::haloed(
        ui.painter(),
        Pos2::new(x, paper_top - 2.0 - GAIN_RISE * age),
        Align2::CENTER_BOTTOM,
        &format!("+{} xp", gain.amount),
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
    let shown = outside
        .area(gui, "player bar points", Align::Min)
        .show(gui, |ui| {
            controls::notice(ui, &format!("{} to spend", words::counted(points, "point")))
        });
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
