use bevy::prelude::*;
use ui::egui::{self, CursorIcon, Sense, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::float;

use super::{BUTTON, MapReachStep, MapSight, MapView, MapWindow, minimap};
use crate::core::content::Content;
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
    shows: Option<Rect<Meters>>,
    pub(super) sight: Option<MapSight>,
}

impl MapTabLook {
    /// The ground the tab shows, with the area it is of.
    pub fn shows(&self) -> Option<(AreaId, Rect<Meters>)> {
        self.area.zip(self.shows)
    }
}

pub(super) fn rest(world: &mut World) {
    if menu::showing(world) != Some(MenuTabName::Map) {
        let mut look = world.resource_mut::<MapTabLook>();
        look.middle = None;
        look.drawn = None;
        look.shows = None;
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
    let reaches = reaches_of(world, &view, sheet);
    let step = step
        .unwrap_or(MapReachStep(u32::MAX))
        .min(MapReachStep(reaches.len() as u32 - 1));
    view.step = step;
    view.last_step = MapReachStep(reaches.len() as u32 - 1);
    let wanted = reaches[step.0 as usize];
    let drawn = ease(
        ui,
        drawn.filter(|_| world.resource::<MapTabLook>().area == Some(view.area)),
        wanted,
    );
    view.reach = drawn;
    let whole = ui.interact(sheet, ui.id().with("sheet"), Sense::drag());
    let reach = view.reach.0;
    whole.widget_info(|| {
        let said = view.messages().map_of(&area_name, reach);
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said)
    });
    let mut middle = held.filter(|_| world.resource::<MapTabLook>().area == Some(view.area));
    let dragged = whole.drag_delta();
    if dragged != Vec2::ZERO {
        let pulled = view.ground(view.center() - dragged);
        let bounds = area::loaded(&content, view.area).map(|area| area.size.bounds());
        middle = Some(bounds.map_or(pulled, |bounds| {
            Pos::new(
                pulled.x.clamp(bounds.min_x(), bounds.max_x()),
                pulled.y.clamp(bounds.min_y(), bounds.max_y()),
            )
        }));
    }
    view.window = MapWindow::Sheet {
        rect: sheet,
        middle: middle.unwrap_or(rests_on),
    };
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
    let off_me = middle.unwrap_or(rests_on) != view.at;
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
    let from = view.ground(sheet.min);
    let to = view.ground(sheet.max);
    let mut look = world.resource_mut::<MapTabLook>();
    look.area = Some(view.area);
    look.middle = middle;
    look.drawn = Some(drawn);
    look.shows = Some(Rect::from_points([from, to]));
    look.step = stepped.or(Some(step));
}

/// The reaches the map can be drawn at: the game's, up to the one that fits the whole area in the
/// sheet, which is the furthest it zooms out.
fn reaches_of(world: &World, view: &MapView, sheet: egui::Rect) -> Vec<Meters> {
    let settings = world.resource::<GameSettings>();
    let mut reaches: Vec<Meters> = settings.map_reaches.0.clone();
    let Some(size) = area::loaded(&view.content, view.area).map(|area| area.size.bounds()) else {
        return reaches;
    };
    let across = (size.max_x() - size.min_x()).max(1.0);
    let down = (size.max_y() - size.min_y()).max(1.0);
    let per_meter = (sheet.width() / across).min(sheet.height() / down);
    let fit = Meters(minimap::GROUND / per_meter);
    reaches.retain(|reach| reach.0 < fit.0);
    reaches.push(fit);
    reaches
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
