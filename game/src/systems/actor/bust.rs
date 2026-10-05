use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::core::assets::AssetRef;
use crate::core::sfx::SfxId;

pub struct ModelDef {
    pub sheet: AssetRef,
    pub busts: Option<Busts>,
}

pub struct Busts {
    pub generic: GenericBusts,
    pub individual: &'static [(IndividualExpression, Bust)],
}

pub struct GenericBusts {
    pub neutral: Bust,
    pub happy: Bust,
    pub sad: Bust,
    pub angry: Bust,
    pub surprised: Bust,
    pub thinking: Bust,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bust {
    pub art: AssetRef,
    pub cue: SfxId,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Generic(GenericExpression),
    Individual(IndividualExpression),
}

#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum GenericExpression {
    #[default]
    Neutral,
    Happy,
    Sad,
    Angry,
    Surprised,
    Thinking,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndividualExpression {
    Smug,
    Counting,
    Laughing,
    Smirk,
    Sleepy,
}

impl Busts {
    pub fn all(&self) -> impl Iterator<Item = Bust> + '_ {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(self.individual.iter().map(|(_, bust)| *bust))
    }

    pub fn face(&self, face: Face) -> Option<Bust> {
        match face {
            Face::Generic(expression) => Some(self.generic.of(expression)),
            Face::Individual(expression) => self
                .individual
                .iter()
                .find(|(own, _)| *own == expression)
                .map(|(_, bust)| *bust),
        }
    }
}

impl GenericBusts {
    pub fn of(&self, expression: GenericExpression) -> Bust {
        match expression {
            GenericExpression::Neutral => self.neutral,
            GenericExpression::Happy => self.happy,
            GenericExpression::Sad => self.sad,
            GenericExpression::Angry => self.angry,
            GenericExpression::Surprised => self.surprised,
            GenericExpression::Thinking => self.thinking,
        }
    }
}
