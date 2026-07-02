mod components;
mod utils;

pub mod themes;
pub mod tokens;

use bevy_app::{App, Plugin, PostUpdate, Startup, Update};
use bevy_asset::{AssetServer, Handle};
use bevy_ecs::prelude::*;
use bevy_ecs::template::{FnTemplate, TemplateContext};
use bevy_scene::{Scene, ScenePlugin};
use bevy_text::Font;
use bevy_ui::UiSystems;
use bevy_ui_widgets::{ButtonPlugin, CheckboxPlugin, EditableTextInputPlugin, ScrollAreaPlugin};

use utils::motion::MotionPlugin;
use utils::opacity::OpacityPlugin;

pub use utils::theme;
pub(crate) use utils::{collapse, drag, motion, opacity, overlay, place, state, style, surface};

pub use bevy_ui_widgets::{Activate, ValueChange, observe};
pub use components::*;
pub use utils::drag::{DragHandle, DragRoot, Geom, OnSettle, OnTap, ResizeHandle, SnapGrid};
pub use utils::motion::{Easing, Timing, Transform2d, transition};
pub use utils::overlay::{Dismissable, Open, OverlayAction, set_overlay_open};
pub use utils::place::place;
pub use utils::state::{SelectionChanged, selected};
pub use utils::style::{StatefulPaint, Style};
pub use utils::theme::{Family, Theme};

#[derive(bevy_ecs::schedule::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiReactive;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(OpacityPlugin);
        app.add_plugins(MotionPlugin);
        if !app.is_plugin_added::<ButtonPlugin>() {
            app.add_plugins(ButtonPlugin);
        }
        if !app.is_plugin_added::<CheckboxPlugin>() {
            app.add_plugins(CheckboxPlugin);
        }
        if !app.is_plugin_added::<ScrollAreaPlugin>() {
            app.add_plugins(ScrollAreaPlugin);
        }
        if !app.is_plugin_added::<EditableTextInputPlugin>() {
            app.add_plugins(EditableTextInputPlugin);
        }
        if !app.is_plugin_added::<ScenePlugin>() {
            app.add_plugins(ScenePlugin);
        }
        app.add_plugins(drag::DragPlugin);
        app.init_resource::<overlay::TooltipClock>()
            .add_systems(Startup, (load_fonts, overlay::spawn_overlay_host))
            .add_systems(
                Update,
                (
                    overlay::reparent_portals,
                    overlay::cleanup_portals,
                    state::init_selection,
                    state::apply_start_checked,
                    state::inherit_checked,
                    overlay::open_due_tooltips,
                    overlay::advance_overlays,
                    state::apply_gating,
                    components::progress::sync_progress,
                    components::slider::sync_slider,
                    components::sonner::age_toasts,
                    components::sonner::size_toaster,
                    components::sonner::layout_toasts,
                    components::sonner::reap_toasts,
                    components::text_input::blur_on_escape,
                    components::scroll_area::pin_to_bottom,
                    components::scroll_area::animate_scroll,
                    style::apply_styles,
                )
                    .chain()
                    .in_set(UiReactive),
            )
            .add_systems(
                PostUpdate,
                (
                    collapse::advance_collapse.after(UiSystems::Layout),
                    place::position_overlays.after(UiSystems::Layout),
                    components::scroll_area::sync_scrollbars.after(UiSystems::Layout),
                    components::text_input::apply_submits.after(bevy_text::EditableTextSystems),
                ),
            )
            .add_observer(state::on_select_activate)
            .add_observer(state::on_pressable_press)
            .add_observer(state::on_pressable_release)
            .add_observer(state::on_pressable_out)
            .add_observer(components::slider::on_thumb_drag)
            .add_observer(components::scroll_area::on_scroll)
            .add_observer(components::scroll_area::on_thumb_drag)
            .add_observer(components::sonner::on_close)
            .add_observer(components::sonner::toaster_hover)
            .add_observer(components::sonner::toaster_leave)
            .add_observer(overlay::on_overlay_action)
            .add_observer(overlay::dismiss_on_press)
            .add_observer(overlay::tooltip_over)
            .add_observer(overlay::tooltip_out);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    Top,
    Right,
    #[default]
    Bottom,
    Left,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Resource)]
struct DesignFonts(#[allow(dead_code)] Vec<Handle<Font>>);

pub fn component<C: Component + Clone>(value: C) -> impl Scene {
    FnTemplate(move |_: &mut TemplateContext| Ok(value.clone()))
}

fn load_fonts(assets: Option<Res<AssetServer>>, mut commands: Commands) {
    let Some(assets) = assets else {
        return;
    };
    let handles = [
        "fonts/circular-400-normal.ttf",
        "fonts/circular-500-normal.ttf",
        "fonts/circular-700-normal.ttf",
        "fonts/lato-400-normal.ttf",
        "fonts/lato-700-normal.ttf",
    ]
    .iter()
    .map(|path| assets.load(*path))
    .collect();
    commands.insert_resource(DesignFonts(handles));
}
