//! The site a shader of a stage drawn on a site reads through the prelude: the patch of ground a
//! system's own material lays its shader over, or a line of text the client letters, whose
//! shader is drawn over the presented frame.

use bevy::camera::visibility::RenderLayers;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::{Shader, ShaderRef};
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline};

use super::super::uniform::{MAX_PATH_POINTS, ParamsUniform, SiteUniform, VfxShaderKey};
use super::super::{BodyShape, SiteKind};
use super::repaint;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Pos;
use crate::core::render::present::PRESENT_LAYER;
use crate::core::render::world_point;
use crate::core::units::{Meters, Radians, Seconds};

/// How many of a site's own pixels make a metre of the world.
pub const TEXELS_PER_METER: f32 = 16.0;

/// A site measured for drawing: where it is in the world or on the screen and how big, in its own
/// pixels; its facing; the body a shader reads through the prelude; and the image a shader works
/// on, when the site has one.
#[derive(Clone)]
pub struct SiteFrame {
    pub kind: SiteKind,
    pub place: SitePlace,
    pub size: Vec2,
    pub facing: Radians,
    pub body: SiteBody,
    pub texture: Option<SiteTexture>,
}

/// Where a site stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SitePlace {
    /// On the ground at a point of the world, `footprint` metres across.
    World { at: Vec3, footprint: Vec2 },
    /// On the presented frame, `center` window pixels from its middle with y up.
    Screen { center: Vec2 },
}

/// The image a shader samples through the prelude's `sample`: the texel rect of the site within
/// it and the size of one of its texels.
#[derive(Clone)]
pub struct SiteTexture {
    pub image: Handle<Image>,
    pub rect: Rect,
    pub pixel: Vec2,
}

/// How a site's body lies on the quad drawn over it: its shape and measures in metres, and where
/// the site's point sits in site uv.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteBody {
    pub shape: BodyShape,
    pub origin: Vec2,
    pub extent: Vec2,
}

/// Where a shader is in its run: how far through it, and the seconds since it started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub progress: f32,
    pub time: Seconds,
}

/// A shader of the text stage drawn on a quad over the presented frame.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(VfxShaderKey)]
pub struct TextMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub image: Handle<Image>,
    #[uniform(2)]
    pub site: SiteUniform,
    #[uniform(3)]
    pub params: ParamsUniform,
    pub shader: Handle<Shader>,
}

/// A piece that draws a line of text over the presented frame, hidden until something is
/// presented on it.
pub fn screen_piece(world: &mut World) -> Entity {
    let quad = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(1.0, 1.0));
    let material = world
        .resource_mut::<Assets<TextMaterial>>()
        .add(TextMaterial {
            image: Handle::default(),
            site: SiteUniform::default(),
            params: ParamsUniform::default(),
            shader: Handle::default(),
        });
    world
        .spawn((
            Mesh2d(quad),
            MeshMaterial2d(material),
            RenderLayers::layer(PRESENT_LAYER),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id()
}

/// Draws a compiled text shader over the line of text `frame` is through its piece, with the site
/// as the prelude reads it and the parameters as the shader's `Params` struct holds them.
pub fn present_shader(
    world: &mut World,
    piece: Entity,
    frame: &SiteFrame,
    shader: Handle<Shader>,
    params: ParamsUniform,
    run: Run,
) {
    let (SitePlace::Screen { center }, Some(texture)) = (frame.place, &frame.texture) else {
        hide(world, piece);
        return;
    };
    let Some(handle) = world
        .get::<MeshMaterial2d<TextMaterial>>(piece)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let painted = TextMaterial {
        image: texture.image.clone(),
        site: frame.uniform(run),
        params,
        shader,
    };
    repaint(world, &handle, painted, |was, now| {
        (&was.image, &was.site, &was.params, &was.shader)
            == (&now.image, &now.site, &now.params, &now.shader)
    });
    let mut entity = world.entity_mut(piece);
    if let Some(mut shown) = entity.get_mut::<Transform>() {
        shown.set_if_neq(Transform {
            translation: center.extend(TEXT_Z),
            scale: frame.size.extend(1.0),
            ..default()
        });
    }
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        visibility.set_if_neq(Visibility::Inherited);
    }
}

pub fn hide(world: &mut World, piece: Entity) {
    if let Some(mut visibility) = world.get_mut::<Visibility>(piece) {
        visibility.set_if_neq(Visibility::Hidden);
    }
}

impl SiteFrame {
    /// The patch an area site is drawn on, around its footprint's outline, with the footprint on
    /// it, laid on ground `ground` high.
    pub fn of_area(footprint: &PlacedShape, ground: impl Fn(Pos<Meters>) -> Meters) -> SiteFrame {
        let (center, size) = bounds(&footprint.outline());
        let size = size.max(Vec2::ONE);
        let min = center - size / 2.0;
        let origin = Vec2::new(
            (footprint.at.x - min.x) / size.x,
            (footprint.at.y - min.y) / size.y,
        );
        let (shape, extent) = match footprint.shape {
            Shape::Circle { radius } => (BodyShape::Circle, Vec2::new(radius.0, 0.0)),
            Shape::Rect { width, height } => (BodyShape::Rect, Vec2::new(width.0, height.0)),
            Shape::Cone { angle, length } => {
                (BodyShape::Cone, Vec2::new(angle.radians().0, length.0))
            }
            Shape::Line { length, width } => (BodyShape::Line, Vec2::new(length.0, width.0)),
        };
        let at = Pos::new(center.x, center.y);
        SiteFrame {
            kind: SiteKind::Area,
            place: SitePlace::World {
                at: world_point(at, ground(at)),
                footprint: size,
            },
            size: size * TEXELS_PER_METER,
            facing: Radians(-footprint.facing.0),
            body: SiteBody {
                shape,
                origin,
                extent,
            },
            texture: None,
        }
    }

    /// A line of text drawn on the screen at `center`, in window pixels from the window's middle
    /// with y up, over `texture`, the text's coverage as an image's alpha.
    pub fn text(center: Vec2, size: Vec2, texture: SiteTexture) -> SiteFrame {
        SiteFrame {
            kind: SiteKind::Text,
            place: SitePlace::Screen { center },
            size,
            facing: Radians::default(),
            body: SiteBody {
                shape: BodyShape::None,
                origin: Vec2::splat(0.5),
                extent: Vec2::ZERO,
            },
            texture: Some(texture),
        }
    }

    /// How many metres the site spans: a patch of ground its footprint, a line of text its pixels
    /// over the pixels to a metre.
    pub fn metres(&self) -> Vec2 {
        match self.place {
            SitePlace::World { footprint, .. } => footprint,
            SitePlace::Screen { .. } => self.size / TEXELS_PER_METER,
        }
    }

    /// The site as the prelude reads it, `run` into its shader's run.
    pub fn uniform(&self, run: Run) -> SiteUniform {
        let (rect, pixel) = self.texture.as_ref().map_or(
            (Rect::from_corners(Vec2::ZERO, Vec2::ONE), Vec2::ONE),
            |texture| (texture.rect, texture.pixel),
        );
        let metres = self.metres();
        SiteUniform {
            rect: Vec4::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y),
            size: metres * TEXELS_PER_METER,
            pixel,
            kind: self.kind as u32,
            facing: self.facing.0,
            heading: self.facing.0,
            progress: run.progress,
            time: run.time.0,
            overscan: Vec2::ONE,
            shape: self.body.shape as u32,
            point_count: 0,
            image: u32::from(self.texture.is_some()),
            faded: 0.0,
            origin: self.body.origin,
            extent: self.body.extent,
            metres,
            points: [Vec4::ZERO; MAX_PATH_POINTS / 2],
        }
    }
}

impl From<&TextMaterial> for VfxShaderKey {
    fn from(material: &TextMaterial) -> VfxShaderKey {
        VfxShaderKey::of(&material.shader)
    }
}

impl Material2d for TextMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = key.bind_group_data.handle();
        }
        Ok(())
    }
}

/// How far above the presented frame a line of text is drawn.
const TEXT_Z: f32 = 3.0;

fn bounds(points: &[Pos<Meters>]) -> (Vec2, Vec2) {
    if points.is_empty() {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    let min = points
        .iter()
        .fold(Vec2::MAX, |min, at| min.min(Vec2::new(at.x, at.y)));
    let max = points
        .iter()
        .fold(Vec2::MIN, |max, at| max.max(Vec2::new(at.x, at.y)));
    ((min + max) / 2.0, max - min)
}
