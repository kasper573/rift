use bevy::prelude::*;
use ui::egui::{self, CursorIcon, Sense, Stroke, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::{float, ink};

use super::{BUTTON, MapReachStep, MapView, MapWindow};
use crate::core::content::Content;
use crate::core::math::{Pos, Rect};
use crate::core::units::{MeterSize, Meters};
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
        ui.painter()
            .rect_stroke(sheet, 0.0, EDGE, egui::StrokeKind::Inside);
    }
}

#[derive(Resource, Default)]
pub(super) struct MapTabLook {
    area: Option<AreaId>,
    step: Option<MapReachStep>,
    middle: Option<Pos<Meters>>,
    shows: Option<Rect<Meters>>,
}

impl MapTabLook {
    pub fn shows(&self) -> Option<Rect<Meters>> {
        self.shows
    }
}

pub(super) fn rest(world: &mut World) {
    if menu::showing(world) != Some(MenuTabName::Map) {
        let mut look = world.resource_mut::<MapTabLook>();
        look.middle = None;
        look.shows = None;
    }
}

const EDGE: Stroke = Stroke {
    width: 1.5,
    color: ink::FRAME,
};
const INSET: f32 = 18.0;
const BUTTON_GAP: f32 = 6.0;
const NAME_ROOM: f32 = 30.0;

fn map(ui: &mut Ui, world: &mut World, sheet: egui::Rect) {
    let (step, held) = {
        let look = world.resource::<MapTabLook>();
        (look.step, look.middle)
    };
    let view = MapView::of(
        world,
        |reaches, _| step.unwrap_or(reaches.last_step()),
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
    let area_name = view.area.get(&content).name.clone();
    let whole = ui.interact(sheet, ui.id().with("sheet"), Sense::drag());
    let reach = view.reach.0;
    whole.widget_info(|| {
        let said = format!("Map of {area_name}, reaching {reach} m");
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
        middle: middle.unwrap_or(view.at),
    };
    let whole = whole.on_hover_cursor(if dragged == Vec2::ZERO {
        CursorIcon::Grab
    } else {
        CursorIcon::Grabbing
    });
    super::chart(ui, world, &view);
    super::north_bead(ui, world, sheet.center_top() + Vec2::new(0.0, INSET));
    let corner = sheet.right_bottom() - Vec2::splat(INSET);
    let along = Vec2::new(2.0 * BUTTON + BUTTON_GAP, 0.0);
    let stepped = super::reach_buttons(ui, &view, [corner - along, corner]);
    let stepped = stepped.or_else(|| super::wheeled(ui, &whole, &view));
    if centre_button(ui, corner - 2.0 * along, middle.is_some()) {
        middle = None;
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
    look.shows = Some(Rect::from_points([from, to]));
    if stepped.is_some() {
        look.step = stepped;
    }
}

fn centre_button(ui: &mut Ui, at: egui::Pos2, away: bool) -> bool {
    let button = super::round_button(ui, at, CENTRES, away);
    super::arrow(ui, at + Vec2::new(0.0, 1.0), Vec2::UP, 5.0, button.ink);
    float::tooltip(&button.response, |ui| {
        ui.label(TextRole::Tooltip.rich(CENTRES));
    });
    away && button.response.clicked()
}

const CENTRES: &str = "Centre the map on you";
