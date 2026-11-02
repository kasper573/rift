use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use ui::egui;

use bevy::prelude::*;
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiOutput, EguiPlugin, EguiPostUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext,
};
use strum::VariantArray;
use ui::egui::load::SizedTexture;
use ui::egui::{LayerId, Order};

use crate::core::content::ImageRef;
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::WindowPx;

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

/// The background stacks in tiers over what floats over the world: whatever a tier holds lies over
/// everything in the tiers before it, however recently either was drawn or clicked. egui would
/// otherwise stack the background by whichever layer it saw first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray)]
pub enum Tier {
    Back,
    Mid,
    Front,
    Top,
    Peak,
}

impl Tier {
    fn anchor(self) -> LayerId {
        LayerId::new(Order::Background, egui::Id::new(("tier", self)))
    }
}

/// Puts a background layer in its tier for this pass.
pub fn stack(gui: &egui::Context, tier: Tier, layer: LayerId) {
    gui.set_sublayer(tier.anchor(), layer);
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

/// Where a gui's pictures come from: the game's own asset loading, or a tool's.
pub trait ImageSource {
    fn image(&mut self, asset: &ImageRef) -> Option<GuiImage>;
}

impl ImageSource for World {
    fn image(&mut self, asset: &ImageRef) -> Option<GuiImage> {
        image(self, asset)
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
            .add_systems(
                EguiPrimaryContextPass,
                (forget_world_layers, raise_tiers).before(Band::Stage),
            )
            .add_systems(
                PostUpdate,
                read_cursor.after(EguiPostUpdateSet::ProcessOutput),
            );
    }
}

/// `rect` of the interface in window pixels, as reported.
pub fn window_rect(rect: egui::Rect) -> Rect<WindowPx> {
    Rect::new(
        Pos::new(rect.min.x, rect.min.y),
        Size::new(rect.width(), rect.height()),
    )
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

/// An image is kept from the first ask on, so one shown again never waits on a fresh load.
pub fn image(world: &mut World, asset: &ImageRef) -> Option<GuiImage> {
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
        path: file.to_owned(),
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

/// Whether the interface is dragging something, which follows the pointer while it lasts.
pub fn dragging(world: &World) -> bool {
    context(world).is_some_and(|gui| gui.dragged_id().is_some())
}

pub fn cursor(world: &World) -> egui::CursorIcon {
    world.resource::<GuiCursor>().0
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
    ui::apply(&gui);
    gui.options_mut(|options| options.zoom_with_keyboard = false);
    commands.insert_resource(GuiContext(gui));
}

fn forget_world_layers(mut layers: ResMut<WorldLayers>, mut widgets: ResMut<WidgetRects>) {
    layers.0.clear();
    widgets.0.clear();
}

fn raise_tiers(gui: Option<Res<GuiContext>>) {
    let Some(gui) = gui else {
        return;
    };
    for &tier in Tier::VARIANTS {
        gui.0.move_to_top(tier.anchor());
    }
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
