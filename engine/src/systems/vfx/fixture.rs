use bevy_ecs::prelude::*;
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::fixture::FixtureLock;
use crate::core::content::{Content, ContentFile, Fixture, Nearby};
use crate::core::vfx::{
    ParamKind, ShaderParam, SiteKind, Stage, VfxDef, VfxParams, VfxShaderDef, VfxSite,
};

/// The parameter the engine sets when it draws the ground an area skill is aimed at: whether the
/// skill hurts whom it lands on.
pub const HOSTILE: &str = "hostile";

/// The vfx every game provides, played by the engine itself where damage, healing, defeat and a
/// level reached land, so skills, items and scripts never repeat them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum VfxFixture {
    DamageTaken,
    Healed,
    Defeated,
    LevelUp,
}

impl Fixture for VfxFixture {
    type Row = VfxDef;
    const FOLDER: Option<&'static str> = Some("vfx");
}

impl VfxFixture {
    /// The kinds of site the engine plays the fixture on.
    pub fn sites(self) -> &'static [SiteKind] {
        match self {
            VfxFixture::DamageTaken
            | VfxFixture::Healed
            | VfxFixture::Defeated
            | VfxFixture::LevelUp => &[SiteKind::Actor],
        }
    }
}

/// The shaders every game provides, drawn by the engine itself: the ground an area skill is aimed
/// at, and the pops that burst out of actors, one look for each kind of moment they mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum VfxShaderFixture {
    SkillArea,
    Pop,
    DamagePop,
    HealPop,
    StatusPop,
    LevelPop,
}

impl Fixture for VfxShaderFixture {
    type Row = VfxShaderDef;
    const FOLDER: Option<&'static str> = Some("vfx");

    fn locks(self) -> Vec<FixtureLock> {
        let stage = FixtureLock::field(crate::field!(VfxShaderDef, stage), &self.stage());
        std::iter::once(stage)
            .chain(self.params().iter().map(|param| {
                FixtureLock::entry(
                    crate::field!(VfxShaderDef, params),
                    param,
                    &[
                        crate::field!(ShaderParam, name),
                        crate::field!(ShaderParam, kind),
                    ],
                )
            }))
            .collect()
    }

    fn fresh(self, near: &Nearby) -> Result<VfxShaderDef, String> {
        Ok(VfxShaderDef {
            stage: self.stage(),
            params: self.params(),
            ..VfxShaderDef::fresh(near)?
        })
    }
}

impl VfxShaderFixture {
    /// The stage the engine draws the fixture's shader on: the ground a skill is aimed at, the text
    /// of a pop.
    pub fn stage(self) -> Stage {
        match self {
            VfxShaderFixture::SkillArea => Stage::Ground,
            VfxShaderFixture::Pop
            | VfxShaderFixture::DamagePop
            | VfxShaderFixture::HealPop
            | VfxShaderFixture::StatusPop
            | VfxShaderFixture::LevelPop => Stage::Text,
        }
    }

    /// The parameters the engine hands the fixture's shader.
    fn params(self) -> Vec<ShaderParam> {
        let set: &[(&str, ParamKind)] = match self {
            VfxShaderFixture::SkillArea => &[(HOSTILE, ParamKind::Bool)],
            VfxShaderFixture::Pop
            | VfxShaderFixture::DamagePop
            | VfxShaderFixture::HealPop
            | VfxShaderFixture::StatusPop
            | VfxShaderFixture::LevelPop => &[],
        };
        set.iter()
            .map(|&(name, kind)| ShaderParam {
                name: name.to_owned(),
                kind,
                default: kind.blank(),
                min: None,
                max: None,
            })
            .collect()
    }
}

pub fn play(world: &mut World, fixture: VfxFixture, site: VfxSite) {
    let content = world.resource::<Content>().clone();
    super::show(world, fixture.id(&content), site);
}

pub fn play_for(world: &mut World, player: Entity, fixture: VfxFixture, site: VfxSite) {
    let content = world.resource::<Content>().clone();
    super::spawn_for(
        world,
        player,
        fixture.id(&content),
        site,
        VfxParams::default(),
    );
}
