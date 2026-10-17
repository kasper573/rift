use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiOutput, EguiPlugin, EguiPostUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext,
};
use egui::load::SizedTexture;
use egui::{FontDefinitions, LayerId, Order};

use crate::core::assets::AssetRef;
use crate::core::content::asset_of;
use crate::core::text::MotionPreference;

/// Each band draws in its own egui order, so it stacks the same however the frame's systems run.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Band {
    Stage,
    Hud,
    /// Drawn after the stage and the HUD, whose rects what floats over the world steers around.
    World,
    Notices,
    System,
    Modal,
    Pointer,
    /// The cursor itself, when the frame draws it, over whatever follows the pointer.
    Cursor,
}

impl Band {
    pub fn order(self) -> Order {
        match self {
            Band::World | Band::Stage => Order::Background,
            Band::Hud => Order::Middle,
            Band::Notices | Band::System | Band::Modal => Order::Foreground,
            Band::Pointer => Order::Tooltip,
            Band::Cursor => Order::Debug,
        }
    }
}

#[derive(Resource, Clone)]
pub struct GuiContext(pub egui::Context);

#[derive(Resource, Default)]
pub struct WorldLayers(HashSet<LayerId>);

impl WorldLayers {
    pub fn contains(&self, layer: LayerId) -> bool {
        self.0.contains(&layer)
    }
}

#[derive(Resource, Clone)]
pub struct GuiFonts(pub FontDefinitions);

/// Where each named interface widget was drawn this pass, for what plays over a widget.
#[derive(Resource, Default)]
pub struct WidgetRects(pub HashMap<String, egui::Rect>);

#[derive(Clone, Debug)]
pub struct GuiImage {
    pub texture: egui::TextureId,
    pub size: egui::Vec2,
    pub path: String,
}

impl GuiImage {
    pub fn widget(&self, size: egui::Vec2) -> egui::Image<'static> {
        egui::Image::new(SizedTexture::new(self.texture, size)).alt_text(self.path.clone())
    }
}

pub struct GuiPlugin;

impl Plugin for GuiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .insert_resource(EguiGlobalSettings {
                auto_create_primary_context: false,
                enable_cursor_icon_updates: false,
                ..default()
            })
            .init_resource::<WorldLayers>()
            .init_resource::<WidgetRects>()
            .init_resource::<GuiImages>()
            .init_resource::<GuiCursor>()
            .configure_sets(
                EguiPrimaryContextPass,
                (
                    Band::Stage,
                    Band::Hud,
                    Band::World,
                    Band::Notices,
                    Band::System,
                    Band::Modal,
                    Band::Pointer,
                    Band::Cursor,
                )
                    .chain(),
            )
            .add_systems(PreUpdate, adopt_context)
            .add_systems(Update, apply_look.run_if(look_changed))
            .add_systems(
                EguiPrimaryContextPass,
                forget_world_layers.before(Band::Stage),
            )
            .add_systems(
                PostUpdate,
                read_cursor.after(EguiPostUpdateSet::ProcessOutput),
            );
    }
}

pub fn context(world: &World) -> Option<egui::Context> {
    world.get_resource::<GuiContext>().map(|gui| gui.0.clone())
}

pub fn world_area(world: &mut World, id: impl Hash + std::fmt::Debug) -> egui::Area {
    let id = egui::Id::new(id);
    world
        .resource_mut::<WorldLayers>()
        .0
        .insert(LayerId::new(Band::World.order(), id));
    egui::Area::new(id)
        .order(Band::World.order())
        .interactable(false)
        .constrain(false)
}

/// egui places an area by the size it had the pass before, so one held by any point but its top
/// left shows out of place on the pass its size changes, and that pass is drawn again.
pub fn show_anchored<R>(
    gui: &egui::Context,
    area: egui::Area,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let before = gui.memory(|memory| memory.area_rect(area.layer().id));
    let shown = area.show(gui, add);
    if before.is_some_and(|before| before.size() != shown.response.rect.size()) {
        gui.request_discard("an anchored area changed size");
    }
    shown
}

/// A widget's tooltip opens once the pointer has been on it a moment, however the pointer moves or
/// clicks there, and at once while another has only just closed. egui's own waits for the pointer to
/// rest, and after a click to move again, so it never opened on a widget the player clicks.
pub fn tooltip(response: &egui::Response, add: impl FnOnce(&mut egui::Ui)) {
    let gui = &response.ctx;
    let key = response.id.with("tooltip");
    if !response.contains_pointer() || gui.dragged_id().is_some() {
        gui.data_mut(|data| data.remove::<Pointed>(key));
        return;
    }
    let (delay, grace) = {
        let style = gui.global_style();
        (
            style.interaction.tooltip_delay,
            style.interaction.tooltip_grace_time,
        )
    };
    let (now, pass) = (gui.input(|input| input.time), gui.cumulative_pass_nr());
    let since = gui.data_mut(|data| {
        let since = data
            .get_temp::<Pointed>(key)
            .filter(|pointed| pointed.pass + 1 >= pass)
            .map_or(now, |pointed| pointed.since);
        data.insert_temp(key, Pointed { since, pass });
        since
    });
    let follows_another = egui::Tooltip::seconds_since_last_tooltip(gui) < grace;
    if follows_another || now - since >= f64::from(delay) {
        egui::Tooltip::for_widget(response).show(add);
    }
}

/// An image is kept from the first ask on, so one shown again never waits on a fresh load.
pub fn image(world: &mut World, asset: &AssetRef) -> Option<GuiImage> {
    image_file(world, &asset.path())
}

pub fn image_file(world: &mut World, file: &str) -> Option<GuiImage> {
    if !world.resource::<GuiImages>().0.contains_key(file) {
        let handle = world.resource::<AssetServer>().load(file.to_owned());
        world
            .resource_mut::<GuiImages>()
            .0
            .insert(file.to_owned(), GuiTexture::Loading(handle));
    }
    let texture = match &world.resource::<GuiImages>().0[file] {
        GuiTexture::Ready(texture) => texture.clone(),
        GuiTexture::Loading(handle) => {
            let texture = upload(world, file, handle)?;
            world
                .resource_mut::<GuiImages>()
                .0
                .insert(file.to_owned(), GuiTexture::Ready(texture.clone()));
            texture
        }
    };
    Some(GuiImage {
        texture: texture.id(),
        size: texture.size_vec2(),
        path: asset_of(file).to_owned(),
    })
}

pub fn pointer_claimed(world: &World) -> bool {
    let Some(gui) = context(world) else {
        return false;
    };
    gui.egui_is_using_pointer()
        || gui.dragged_id().is_some()
        || gui.viewport(|viewport| viewport.hits.click.is_some() || viewport.hits.drag.is_some())
}

/// egui keeps a root layer under the whole screen for panels to catch clicks on, which this
/// interface never draws on.
pub fn layer_at(gui: &egui::Context, at: egui::Pos2) -> Option<LayerId> {
    gui.layer_id_at(at)
        .filter(|&layer| layer != LayerId::background())
}

/// Whether the interface is dragging something, which follows the pointer while it lasts.
pub fn dragging(world: &World) -> bool {
    context(world).is_some_and(|gui| gui.dragged_id().is_some())
}

/// Draws what follows the pointer, its top left `from` the pointer, in the frame the pointer is read:
/// never kept on screen or faded in, as egui does to its areas, which would part it from the pointer.
pub fn at_pointer<R>(
    gui: &egui::Context,
    id: egui::Id,
    band: Band,
    from: egui::Vec2,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let at = gui.pointer_latest_pos()?;
    let shown = egui::Area::new(id)
        .order(band.order())
        .fixed_pos(at + from)
        .interactable(false)
        .constrain(false)
        .fade_in(false)
        .show(gui, add);
    Some(shown.inner)
}

pub fn cursor(world: &World) -> egui::CursorIcon {
    world.resource::<GuiCursor>().0
}

const TOOLTIP_DELAY_SECS: f32 = 0.4;
const TOOLTIP_GRACE_SECS: f32 = 0.3;

#[derive(Clone, Copy)]
struct Pointed {
    since: f64,
    pass: u64,
}

#[derive(Resource, Default)]
struct GuiImages(HashMap<String, GuiTexture>);

enum GuiTexture {
    Loading(Handle<Image>),
    Ready(egui::TextureHandle),
}

#[derive(Resource, Default)]
struct GuiCursor(egui::CursorIcon);

fn adopt_context(
    mut contexts: Query<&mut EguiContext, Added<PrimaryEguiContext>>,
    mut commands: Commands,
) {
    let Ok(mut context) = contexts.single_mut() else {
        return;
    };
    let gui = context.get_mut().clone();
    design_tokens::apply(&gui);
    // A window's corner reaches this far past its edge for a resize, which would take the clicks
    // meant for a slot in the window beside it.
    gui.all_styles_mut(|style| {
        style.interaction.resize_grab_radius_corner = style.interaction.resize_grab_radius_side;
        style.interaction.tooltip_delay = TOOLTIP_DELAY_SECS;
        style.interaction.tooltip_grace_time = TOOLTIP_GRACE_SECS;
    });
    gui.options_mut(|options| options.zoom_with_keyboard = false);
    commands.insert_resource(GuiContext(gui));
}

fn look_changed(
    gui: Option<Res<GuiContext>>,
    fonts: Option<Res<GuiFonts>>,
    motion: Option<Res<MotionPreference>>,
) -> bool {
    gui.is_some_and(|gui| {
        gui.is_changed()
            || fonts.is_some_and(|fonts| fonts.is_changed())
            || motion.is_some_and(|motion| motion.is_changed())
    })
}

fn apply_look(gui: Res<GuiContext>, fonts: Option<Res<GuiFonts>>, motion: Res<MotionPreference>) {
    gui.0
        .set_fonts(fonts.map_or_else(design_tokens::builtin_fonts, |fonts| fonts.0.clone()));
    set_motion(&gui.0, *motion);
}

fn set_motion(gui: &egui::Context, motion: MotionPreference) {
    let seconds = if motion.reduced {
        0.0
    } else {
        egui::Style::default().animation_time
    };
    gui.all_styles_mut(|style| style.animation_time = seconds);
}

fn forget_world_layers(mut layers: ResMut<WorldLayers>, mut widgets: ResMut<WidgetRects>) {
    layers.0.clear();
    widgets.0.clear();
}

/// bevy_egui blends every texture as premultiplied but samples a Bevy image as stored, which would
/// show the color of each transparent pixel; egui premultiplies the pixels it is handed itself.
fn upload(world: &World, file: &str, handle: &Handle<Image>) -> Option<egui::TextureHandle> {
    let image = world.resource::<Assets<Image>>().get(handle)?;
    let rgba = image.clone().try_into_dynamic().ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(context(world)?.load_texture(file, pixels, egui::TextureOptions::NEAREST))
}

fn read_cursor(
    outputs: Query<&EguiOutput, With<PrimaryEguiContext>>,
    mut cursor: ResMut<GuiCursor>,
) {
    if let Ok(output) = outputs.single() {
        cursor.0 = output.platform_output.cursor_icon;
    }
}
