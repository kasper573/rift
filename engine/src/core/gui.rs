use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use ui::egui;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiOutput, EguiPlugin, EguiPostUpdateSet,
    EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures, PrimaryEguiContext,
};
use image::imageops::{self, FilterType};
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
    finished(world, file, Finish::Plain)
}

/// The picture as an ink print of itself on the paper it is drawn over, whatever it was painted
/// in: any picture a game brings sits on a sheet as though it were printed there.
pub fn print(world: &mut World, asset: &ImageRef) -> Option<GuiImage> {
    finished(world, &asset.path(), Finish::Print)
}

fn finished(world: &mut World, file: &str, finish: Finish) -> Option<GuiImage> {
    let key = (file.to_owned(), finish);
    if !world.resource::<GuiImages>().0.contains_key(&key) {
        let handle = world.resource::<AssetServer>().load(file.to_owned());
        world
            .resource_mut::<GuiImages>()
            .0
            .insert(key.clone(), GuiTexture::Loading(handle));
    }
    let loading = match &world.resource::<GuiImages>().0[&key] {
        GuiTexture::Ready(image) => return Some(image.clone()),
        GuiTexture::Loading(handle) => handle.clone(),
    };
    let image = upload(world, file, &loading, finish)?;
    world
        .resource_mut::<GuiImages>()
        .0
        .insert(key, GuiTexture::Ready(image.clone()));
    Some(image)
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
struct GuiImages(HashMap<(String, Finish), GuiTexture>);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Finish {
    Plain,
    Print,
}

enum GuiTexture {
    Loading(Handle<Image>),
    Ready(GuiImage),
}

/// A picture smaller than this is pixel art. It is enlarged to it by whole pixels before it is
/// smoothed, so that drawn larger than it is, its pixels keep their edges; and it is kept with no
/// copy smaller than itself, which would only blur it where it is drawn a little smaller.
const SHARP_SIDE: u32 = 128;

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

/// The interface draws a picture at whatever size it likes, so it is kept with its smaller copies
/// and sampled smoothly between them: drawn smaller, every pixel of it still shows. bevy_egui blends
/// every texture as premultiplied but samples a Bevy image as stored, which would show the color of
/// each transparent pixel, so the pixels are premultiplied here as egui does its own.
fn upload(
    world: &mut World,
    file: &str,
    handle: &Handle<Image>,
    finish: Finish,
) -> Option<GuiImage> {
    let source = world.resource::<Assets<Image>>().get(handle)?;
    let mut picture = source.clone().try_into_dynamic().ok()?.to_rgba8();
    let size = egui::vec2(picture.width() as f32, picture.height() as f32);
    if finish == Finish::Print {
        ui::paint::print(&mut picture);
    }
    for pixel in picture.pixels_mut() {
        let [red, green, blue, alpha] = pixel.0;
        pixel.0 = egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha).to_array();
    }
    let longest = picture.width().max(picture.height()).max(1);
    let enlarged = SHARP_SIDE.div_ceil(longest).next_power_of_two();
    let levels = match enlarged {
        1 => longest.ilog2() + 1,
        _ => enlarged.ilog2() + 1,
    };
    let picture = imageops::resize(
        &picture,
        picture.width() * enlarged,
        picture.height() * enlarged,
        FilterType::Nearest,
    );
    let data = (0..levels)
        .scan(picture.clone(), |level, _| {
            let bytes = level.as_raw().clone();
            *level = imageops::thumbnail(
                level,
                (level.width() / 2).max(1),
                (level.height() / 2).max(1),
            );
            Some(bytes)
        })
        .flatten()
        .collect();
    let mut image = Image::new_uninit(
        Extent3d {
            width: picture.width(),
            height: picture.height(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    let texture = world
        .resource_mut::<EguiUserTextures>()
        .add_image(EguiTextureHandle::Strong(handle));
    Some(GuiImage {
        texture,
        size,
        path: file.to_owned(),
    })
}

fn read_cursor(
    outputs: Query<&EguiOutput, With<PrimaryEguiContext>>,
    mut cursor: ResMut<GuiCursor>,
) {
    if let Ok(output) = outputs.single() {
        cursor.0 = output.platform_output.cursor_icon;
    }
}
