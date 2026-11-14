use bevy::prelude::*;
use ui::card::{self, CardResponse, CardTilt};
use ui::egui;
use ui::egui::{Id, LayerId, Pos2, Rect, Ui, Vec2};
use ui::motion::{self, CardPose};
use ui::sheet::Depth;

use super::panel;
use crate::core::card::CardMaterial;
use crate::core::card::stage::{self, HungCardLamp, StagedCard};
use crate::core::gui::{self, Band, GuiContext};
use crate::systems::interface::InterfaceSound;
use crate::systems::settings::UserSettings;

const CARD_TOP: f32 = 96.0;
const CARD_MARGIN: f32 = 72.0;
const EDGE_ORIGIN: f32 = 18.0;

/// One thing of a kind inspected at a time, in a card pinned beside the windows until it is
/// closed; opening another of the kind replaces it.
#[derive(Resource)]
pub(crate) struct Pinned<T: Send + Sync + 'static> {
    wanted: Option<T>,
    to_front: bool,
    drawn: Option<Rect>,
    dealt: Option<Dealt>,
    leaving: Option<Leaving<T>>,
}

impl<T: Send + Sync + 'static> Default for Pinned<T> {
    fn default() -> Pinned<T> {
        Pinned {
            wanted: None,
            to_front: false,
            drawn: None,
            dealt: None,
            leaving: None,
        }
    }
}

pub(crate) fn pin<T: PartialEq + Send + Sync + 'static>(
    world: &mut World,
    card: T,
    from: Option<Rect>,
) {
    let mut pinned = world.resource_mut::<Pinned<T>>();
    pinned.to_front = true;
    if pinned.wanted.as_ref() == Some(&card) {
        return;
    }
    let was = pinned.wanted.replace(card);
    pinned.dealt = Some(Dealt { from, since: None });
    let was_shown = was.is_some();
    if let (Some(old), Some(at)) = (was, pinned.drawn) {
        pinned.leaving = Some(Leaving {
            card: old,
            at,
            since: None,
            way: Way::Replaced,
        });
    }
    if !was_shown {
        InterfaceSound::UiOpen.play(world);
    }
}

pub(crate) fn unpin<T: Send + Sync + 'static>(world: &mut World) -> bool {
    let mut pinned = world.resource_mut::<Pinned<T>>();
    let Some(card) = pinned.wanted.take() else {
        return false;
    };
    pinned.dealt = None;
    if let Some(at) = pinned.drawn.take() {
        pinned.leaving = Some(Leaving {
            card,
            at,
            since: None,
            way: Way::Flick,
        });
    }
    InterfaceSound::UiClose.play(world);
    true
}

pub(crate) fn pinned<T: Clone + Send + Sync + 'static>(world: &World) -> Option<T> {
    world.resource::<Pinned<T>>().wanted.clone()
}

/// Where the card stood when last drawn, while it is pinned.
pub(crate) fn pinned_rect<T: Send + Sync + 'static>(world: &World) -> Option<Rect> {
    world.resource::<Pinned<T>>().drawn
}

/// Draws the pinned card of its kind, and the one it replaced or that was closed on its way
/// out, each on the card stage in the material `material` says it is made of.
pub(crate) fn draw_pinned<T: Clone + Send + Sync + 'static>(
    world: &mut World,
    key: &'static str,
    material: impl Fn(&World, &T) -> CardMaterial,
    draw: impl Fn(&mut Ui, &mut World, &T, Depth) -> CardResponse,
) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let now = gui.input(|input| input.time);
    let screen = gui.content_rect();
    let leaving = leaving_pose(world, now);
    let main = panel::layer(key);
    let scratch = LayerId::new(Band::Hud.order(), Id::new(("leaving card", key)));
    let pinned = pinned::<T>(world);
    if let Some((card, at, pose, way)) = leaving {
        let since = motion::mark(&gui, scratch);
        egui::Area::new(scratch.id)
            .order(scratch.order)
            .fixed_pos(at.min)
            .constrain(false)
            .fade_in(false)
            .interactable(false)
            .show(&gui, |ui| draw(ui, world, &card, Depth::Behind));
        let staged = StagedCard {
            key: scratch.id,
            runs: motion::lifted(&gui, scratch, since),
            flat: at,
            pose: Some(pose),
            turns: false,
            material: material(world, &card),
        };
        // The card a new one replaces sinks away under it, on its layer; one closed leaves on
        // its own.
        let onto = match way {
            Way::Replaced if pinned.is_some() => main,
            Way::Replaced | Way::Flick => scratch,
        };
        stage::stage(world, &gui, staged, onto);
    }
    let Some(card) = pinned else {
        world.resource_mut::<Pinned<T>>().drawn = None;
        return;
    };
    let start = Pos2::new(
        (screen.right() - card::WIDTH - CARD_MARGIN).max(8.0),
        screen.top() + CARD_TOP,
    );
    let rest = panel::placed_spot(world, &gui, key, start);
    let deal = dealing::<T>(world, now, rest, screen);
    let front = panel::front_sheet(world, &gui) == Some(key);
    let depth = if front { Depth::Front } else { Depth::Behind };
    let since = motion::mark(&gui, main);
    let shown = egui::Area::new(panel::panel_id(key))
        .order(Band::Hud.order())
        .fixed_pos(rest.min)
        .constrain(false)
        .fade_in(false)
        .interactable(deal.is_none())
        .show(&gui, |ui| draw(ui, world, &card, depth));
    let response = shown.inner;
    let drawn = shown.response.rect;
    let mut open = !response.close.is_some_and(|close| close.clicked());
    if deal.is_none() {
        panel::grab_spot(world, key, &response.band, drawn);
    } else {
        open = true;
    }
    let staged = StagedCard {
        key: main.id,
        runs: motion::lifted(&gui, main, since),
        flat: drawn,
        pose: deal,
        turns: world.resource::<UserSettings>().cards_tilt(),
        material: material(world, &card),
    };
    if !stage::stage(world, &gui, staged, main)
        && let Some(dealt) = world.resource_mut::<Pinned<T>>().dealt.as_mut()
    {
        dealt.since = None;
    }
    panel::mark_drawn(world, key);
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), drawn);
    world.resource_mut::<Pinned<T>>().drawn = Some(drawn);
    if !open {
        unpin::<T>(world);
    }
}

/// How far the pinned card of `key` is turned toward the pointer.
pub(crate) fn pinned_tilt(world: &World, key: &'static str) -> CardTilt {
    stage::tilt_of(world, panel::layer(key).id)
}

/// The lamp the pinned card of `key` is lit by, while it is on the stage.
pub(crate) fn pinned_lamp(world: &World, key: &'static str) -> Option<HungCardLamp> {
    stage::lamp_of(world, panel::layer(key).id)
}

// A press brings the area it lands on to the front as its pass ends, and areas brought to the front
// in the same pass keep their order. So the card comes to the front a pass after it was asked for,
// to end up above the window it was asked for from.
pub(crate) fn raise_pinned<T: Send + Sync + 'static>(
    key: &'static str,
) -> impl FnMut(ResMut<Pinned<T>>, Option<Res<GuiContext>>) {
    move |mut pinned, gui| {
        if !pinned.to_front {
            return;
        }
        pinned.to_front = false;
        if let Some(gui) = gui
            && pinned.wanted.is_some()
        {
            gui.0.move_to_top(panel::layer(key));
        }
    }
}

pub(crate) fn forget_pinned<T: Send + Sync + 'static>(mut pinned: ResMut<Pinned<T>>) {
    *pinned = Pinned::default();
}

struct Dealt {
    from: Option<Rect>,
    since: Option<f64>,
}

struct Leaving<T> {
    card: T,
    at: Rect,
    since: Option<f64>,
    way: Way,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Flick,
    Replaced,
}

fn leaving_pose<T: Clone + Send + Sync + 'static>(
    world: &mut World,
    now: f64,
) -> Option<(T, Rect, CardPose, Way)> {
    let mut pinned = world.resource_mut::<Pinned<T>>();
    let leaving = pinned.leaving.as_mut()?;
    let since = *leaving.since.get_or_insert(now);
    let seconds = match leaving.way {
        Way::Flick => motion::FLICK_SECS,
        Way::Replaced => motion::DEAL_SECS,
    };
    let t = ((now - since) as f32 / seconds).clamp(0.0, 1.0);
    if t >= 1.0 {
        pinned.leaving = None;
        return None;
    }
    let pose = match leaving.way {
        Way::Flick => motion::flick(t, leaving.at),
        Way::Replaced => motion::replaced(t, leaving.at),
    };
    Some((leaving.card.clone(), leaving.at, pose, leaving.way))
}

fn dealing<T: Send + Sync + 'static>(
    world: &mut World,
    now: f64,
    rest: Rect,
    screen: Rect,
) -> Option<CardPose> {
    let mut pinned = world.resource_mut::<Pinned<T>>();
    let dealt = pinned.dealt.as_mut()?;
    let since = *dealt.since.get_or_insert(now);
    let t = ((now - since) as f32 / motion::DEAL_SECS).clamp(0.0, 1.0);
    if t >= 1.0 {
        pinned.dealt = None;
        return None;
    }
    let from = dealt.from.unwrap_or_else(|| edge_origin(rest, screen));
    Some(motion::deal(t, from, rest))
}

fn edge_origin(rest: Rect, screen: Rect) -> Rect {
    let centre = rest.center();
    let to_left = centre.x - screen.left();
    let to_right = screen.right() - centre.x;
    let x = if to_right <= to_left {
        screen.right() - EDGE_ORIGIN
    } else {
        screen.left() + EDGE_ORIGIN
    };
    Rect::from_center_size(Pos2::new(x, centre.y), Vec2::splat(2.0 * EDGE_ORIGIN))
}
