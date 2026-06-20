mod context;
mod cursor;
mod draggable;
mod reconcile;
mod view;

use bevy_app::{App, Plugin, Update};

pub use bevy_ui::prelude::Node;
pub use bevy_view_macro::view;
pub use context::{context, provide};
pub use cursor::{CursorIcon, CursorLock, HoverCursor, hovered_cursor};
pub use draggable::{Draggable, Geom, Resizable, draggable, resizable};
pub use reconcile::{
    Instance, ViewNode, ViewRoot, activate_click, activate_drag, activate_drag_end, activate_out,
    activate_over, instance_of, render,
};
pub use view::{
    Bind, Element, InstanceId, PortalKind, View, boundary, button, dyn_text, each, gate, hide,
    image, node, outlet, portal, show, text,
};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorLock>()
            .add_observer(reconcile::on_view_added)
            .add_observer(reconcile::on_view_removed)
            .add_observer(reconcile::on_view_click)
            .add_observer(reconcile::on_view_drag)
            .add_observer(reconcile::on_view_drag_end)
            .add_observer(reconcile::on_view_over)
            .add_observer(reconcile::on_view_out)
            .add_systems(Update, reconcile::render_roots);
    }
}
