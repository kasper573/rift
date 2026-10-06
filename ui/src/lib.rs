mod components;
mod utils;

pub mod themes;
pub mod tokens;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate, Startup};
use bevy_asset::{AssetServer, Handle};
use bevy_camera::visibility::VisibilitySystems;
use bevy_ecs::prelude::*;
use bevy_ecs::template::{FnTemplate, TemplateContext};
use bevy_picking::PickingSystems;
use bevy_scene::{Scene, ScenePlugin};
use bevy_text::{EditableText, Font};
use bevy_ui::UiSystems;
use bevy_ui_widgets::{
    Button, ButtonPlugin, Checkbox, CheckboxPlugin, EditableTextInputPlugin, ScrollAreaPlugin,
};

use utils::motion::MotionPlugin;
use utils::opacity::OpacityPlugin;

pub use utils::theme;
pub(crate) use utils::{
    carry, collapse, cursor, drag, motion, opacity, overlay, place, presence, state, style, surface,
};

pub use bevy_ui_widgets::{Activate, ValueChange, observe};
pub use components::*;
pub use utils::carry::{Carriable, Carried, CarryTarget};
pub use utils::cursor::{ClickThrough, CursorStyle, InterfaceCursor, clicks_through};
pub use utils::drag::{
    DragHandle, DragRoot, Geom, OnSettle, OnTap, Raised, ResizeHandle, SnapGrid, topmost,
};
pub use utils::motion::{Easing, Timing, Transform2d, transition};
pub use utils::overlay::{Dismissable, Open, OverlayAction, set_overlay_open};
pub use utils::place::{node_rect, place};
pub use utils::presence::{Leaving, Presence, PresenceMove};
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
            .init_resource::<TypewriterSpeed>()
            .add_message::<TypewriterReveal>()
            .init_resource::<components::confirm::ModalCounter>()
            .init_resource::<MotionPreference>()
            .init_resource::<InputCatalog>()
            .init_resource::<InterfaceCursor>()
            .register_required_components_with::<Button, CursorStyle>(|| CursorStyle::Pointer)
            .register_required_components_with::<Checkbox, CursorStyle>(|| CursorStyle::Pointer)
            .register_required_components_with::<EditableText, CursorStyle>(|| CursorStyle::Text)
            .add_systems(Startup, (load_fonts, overlay::spawn_overlay_host))
            .add_systems(
                PreUpdate,
                cursor::track_interface_cursor.after(PickingSystems::Hover),
            )
            .configure_sets(
                PostUpdate,
                UiReactive
                    .before(UiSystems::Prepare)
                    .before(VisibilitySystems::VisibilityPropagate)
                    .before(opacity::OpacitySet::Calculate),
            )
            .add_systems(
                PostUpdate,
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
                    components::text_input::blur_field,
                    components::scroll_area::pin_to_bottom,
                    components::scroll_area::animate_scroll,
                    (
                        components::rich_text::lay_out_rich_text,
                        components::rich_text::type_rich_text,
                        components::input::name_inputs,
                        components::rich_text::move_words,
                        components::dialogue::reveal_choices,
                        components::dialogue::mark_selected_choice,
                        components::dialogue::shake_refused,
                        components::cast::sync_cast,
                        components::bubble::sync_bubbles,
                        components::captions::sync_captions,
                        components::alert_bar::sync_alert_bars,
                        components::milestones::sync_milestones,
                        components::intro::move_intro_titles,
                        components::confirm::stamp_modals,
                        components::confirm::keep_default,
                        components::confirm::despawn_closed,
                    )
                        .chain(),
                    (presence::advance_leaving, presence::advance_presence).chain(),
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
            .add_observer(components::window::announce_tab)
            .add_observer(components::sonner::on_close)
            .add_observer(components::alert_bar::close_alert)
            .add_observer(components::sonner::toaster_hover)
            .add_observer(components::sonner::toaster_leave)
            .add_observer(overlay::on_overlay_action)
            .add_observer(overlay::dismiss_on_press)
            .add_observer(overlay::tooltip_over)
            .add_observer(overlay::tooltip_out)
            .add_observer(carry::lift)
            .add_observer(carry::follow)
            .add_observer(carry::set_down)
            .add_observer(carry::deliver);
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
        "fonts/lato-400-italic.ttf",
        "fonts/lato-700-normal.ttf",
        "fonts/lato-700-italic.ttf",
    ]
    .iter()
    .map(|path| assets.load(*path))
    .collect();
    commands.insert_resource(DesignFonts(handles));
}
