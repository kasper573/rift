use bevy::prelude::*;
use ui::egui::{self, CursorIcon, Sense, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::float;

use super::{BUTTON, MapReachStep, MapSight, MapView, MapWindow, minimap, pictures};
use crate::core::content::Content;
use crate::core::gui;
use crate::core::math::{Pos, Rect};
use crate::core::units::{MeterSize, Meters};
use crate::systems::GameSettings;
use crate::systems::area::{self, AreaId};
use crate::systems::hud::menu::{self, MenuTab, MenuTabName};
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceIcon;

pub struct MapMenuTab;

impl MenuTab for MapMenuTab {
    fn name(&self) -> MenuTabName {
        MenuTabName::Map
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Map
    }
    fn opens(&self) -> InputAction {
        InputAction::OpenMap
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        let room = ui.available_rect_before_wrap();
        let sheet = ui.allocate_rect(room, Sense::hover()).rect;
        ui.scope_builder(UiBuilder::new().max_rect(sheet), |ui| {
            ui.shrink_clip_rect(sheet);
            map(ui, world, sheet);
        });
    }
}

#[derive(Resource, Default)]
pub(super) struct MapTabLook {
    area: Option<AreaId>,
    step: Option<MapReachStep>,
    /// The reach drawn, which eases toward the reach the step asks for.
    drawn: Option<Meters>,
    middle: Option<Pos<Meters>>,
    /// The sheet the map was drawn on last, which its zoom steps are preloaded for.
    sheet: Option<egui::Rect>,
    pub(super) sight: Option<MapSight>,
}

/// Asks for what the map shows at each of its zoom steps when it opens on the player at `at` in
/// `area`, so neither opening the map nor zooming it waits on a picture.
pub(super) fn preload(world: &mut World, area: AreaId, at: Pos<Meters>) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(bounds) =
        area::loaded(world.resource::<Content>(), area).map(|area| area.size.bounds())
    else {
        return;
    };
    let sheet = world
        .resource::<MapTabLook>()
        .sheet
        .unwrap_or_else(|| gui.content_rect());
    let reaches = reaches_of(world.resource::<GameSettings>(), Some(bounds), sheet);
    for reach in reaches {
        let per_meter = minimap::GROUND / reach.0;
        let middle = covering(at, bounds, sheet, per_meter);
        let (across, down) = (
            sheet.width() / per_meter / 2.0,
            sheet.height() / per_meter / 2.0,
        );
        let shown = Rect::from_points([
            Pos::new(middle.x - across, middle.y - down),
            Pos::new(middle.x + across, middle.y + down),
        ]);
        let pixels = per_meter * gui.pixels_per_point();
        pictures::want(world, area, shown, pixels, pictures::Wanted::Preload);
    }
}

pub(super) fn rest(world: &mut World) {
    if menu::showing(world) != Some(MenuTabName::Map) {
        let mut look = world.resource_mut::<MapTabLook>();
        look.middle = None;
        look.drawn = None;
        look.sight = None;
    }
}

const INSET: f32 = 18.0;
const BUTTON_GAP: f32 = 6.0;
const NAME_ROOM: f32 = 30.0;

fn map(ui: &mut Ui, world: &mut World, sheet: egui::Rect) {
    let (step, held, sight, drawn) = {
        let look = world.resource::<MapTabLook>();
        (look.step, look.middle, look.sight.clone(), look.drawn)
    };
    let view = MapView::of(
        world,
        |_, _| MapReachStep(0),
        |me| MapWindow::Sheet {
            rect: sheet,
            middle: held.unwrap_or(me),
        },
    );
    let Some(mut view) = view else {
        ui.painter().rect_filled(sheet, 0.0, super::BLANK);
        return;
    };
    let content = world.resource::<Content>().clone();
    let spot = sight.as_ref().and_then(|sight| sight.spot(&content));
    if let Some((area, _)) = spot {
        view.away = area != view.area;
        view.area = area;
    }
    let rests_on = spot.map_or(view.at, |(_, at)| at);
    let area_name = view.area.get(&content).name.clone();
    let bounds = area::loaded(&content, view.area).map(|area| area.size.bounds());
    let reaches = reaches_of(world.resource::<GameSettings>(), bounds, sheet);
    let step = step
        .unwrap_or(MapReachStep(u32::MAX))
        .min(MapReachStep(reaches.len() as u32 - 1));
    view.step = step;
    view.last_step = MapReachStep(reaches.len() as u32 - 1);
    let wanted = reaches[step.0 as usize];
    let widest = reaches[reaches.len() - 1];
    // A sheet grown since the last frame lowers the widest reach at once, while the zoom is
    // still easing out of the reach it had.
    let drawn = Meters(
        ease(
            ui,
            drawn.filter(|_| world.resource::<MapTabLook>().area == Some(view.area)),
            wanted,
        )
        .0
        .min(widest.0),
    );
    view.reach = drawn;
    let per_meter = view.per_meter();
    let covered =
        |at: Pos<Meters>| bounds.map_or(at, |bounds| covering(at, bounds, sheet, per_meter));
    let whole = ui.interact(sheet, ui.id().with("sheet"), Sense::drag());
    let reach = view.reach.0;
    whole.widget_info(|| {
        let said = view.messages().map_of(&area_name, reach);
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said)
    });
    let mut middle = held.filter(|_| world.resource::<MapTabLook>().area == Some(view.area));
    view.window = MapWindow::Sheet {
        rect: sheet,
        middle: covered(middle.unwrap_or(rests_on)),
    };
    let dragged = whole.drag_delta();
    if dragged != Vec2::ZERO {
        middle = Some(covered(view.ground(view.center() - dragged)));
        view.window = MapWindow::Sheet {
            rect: sheet,
            middle: covered(middle.unwrap_or(rests_on)),
        };
    }
    let whole = whole.on_hover_cursor(if dragged == Vec2::ZERO {
        CursorIcon::Grab
    } else {
        CursorIcon::Grabbing
    });
    super::chart(ui, world, &view, sight.as_ref().and_then(MapSight::pin));
    super::north_bead(ui, world, sheet.center_top() + Vec2::new(0.0, INSET));
    let corner = sheet.right_bottom() - Vec2::splat(INSET);
    let along = Vec2::new(2.0 * BUTTON + BUTTON_GAP, 0.0);
    let stepped = super::reach_buttons(ui, &view, [corner - along, corner]);
    let stepped = stepped.or_else(|| super::wheeled(ui, world, &whole, &view));
    let off_me = covered(middle.unwrap_or(rests_on)) != covered(view.at);
    if !view.away
        && centre_button(
            ui,
            corner - 2.0 * along,
            off_me,
            view.messages().centre_on_you(),
        )
    {
        middle = spot.map(|_| view.at);
    }
    super::lettered(
        ui,
        &area_name,
        sheet.center_bottom() - Vec2::new(0.0, NAME_ROOM),
    );
    let mut look = world.resource_mut::<MapTabLook>();
    look.area = Some(view.area);
    look.middle = middle;
    look.drawn = Some(drawn);
    look.sheet = Some(sheet);
    look.step = stepped.or(Some(step));
}

/// The reaches the map can be drawn at: the game's, up to the one at which the area just covers
/// the sheet, which is the furthest it zooms out, so the map never shows past the area's edge.
fn reaches_of(
    settings: &GameSettings,
    bounds: Option<Rect<Meters>>,
    sheet: egui::Rect,
) -> Vec<Meters> {
    let mut reaches: Vec<Meters> = settings.map_reaches.0.clone();
    let Some(size) = bounds else {
        return reaches;
    };
    let across = (size.max_x() - size.min_x()).max(1.0);
    let down = (size.max_y() - size.min_y()).max(1.0);
    let per_meter = (sheet.width() / across).max(sheet.height() / down);
    let cover = Meters(minimap::GROUND / per_meter);
    reaches.retain(|reach| reach.0 < cover.0);
    reaches.push(cover);
    reaches
}

/// The middle nearest `at` for a sheet drawn at `per_meter` that shows no ground past `bounds`.
fn covering(
    at: Pos<Meters>,
    bounds: Rect<Meters>,
    sheet: egui::Rect,
    per_meter: f32,
) -> Pos<Meters> {
    let along = |at: f32, low: f32, high: f32, shown: f32| {
        let half = shown / per_meter / 2.0;
        if high - low <= 2.0 * half {
            (low + high) / 2.0
        } else {
            at.clamp(low + half, high - half)
        }
    };
    Pos::new(
        along(at.x, bounds.min_x(), bounds.max_x(), sheet.width()),
        along(at.y, bounds.min_y(), bounds.max_y(), sheet.height()),
    )
}

/// Moves the drawn reach toward `wanted`, the way the camera's zoom eases.
fn ease(ui: &Ui, drawn: Option<Meters>, wanted: Meters) -> Meters {
    let Some(drawn) = drawn else {
        return wanted;
    };
    let remaining = (wanted.0 / drawn.0).ln();
    if remaining.abs() < 0.002 {
        return wanted;
    }
    let dt = ui.input(|input| input.stable_dt).min(0.1);
    ui.ctx().request_repaint();
    Meters(drawn.0 * (remaining * (1.0 - (-dt * ZOOM_EASE).exp())).exp())
}

const ZOOM_EASE: f32 = 12.0;

fn centre_button(ui: &mut Ui, at: egui::Pos2, away: bool, said: &str) -> bool {
    let button = super::round_button(ui, at, "centre", said, away);
    super::arrow(ui, at + Vec2::new(0.0, 1.0), Vec2::UP, 5.0, button.ink);
    float::tooltip(&button.response, |ui| {
        ui.label(TextRole::Tooltip.rich(said));
    });
    away && button.response.clicked()
}
