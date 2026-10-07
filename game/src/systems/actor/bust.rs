use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::core::assets::AssetRef;

pub use crate::data::expression::Id as IndividualExpression;

pub struct ModelDef {
    pub sheet: AssetRef,
    pub busts: Option<Busts>,
}

pub struct Busts {
    pub generic: GenericBusts,
    pub individual: &'static [IndividualExpression],
}

pub struct GenericBusts {
    pub neutral: AssetRef,
    pub happy: AssetRef,
    pub sad: AssetRef,
    pub angry: AssetRef,
    pub surprised: AssetRef,
    pub thinking: AssetRef,
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

pub struct IndividualExpressionDef {
    pub bust: AssetRef,
}

impl Busts {
    pub fn all(&self) -> impl Iterator<Item = AssetRef> + '_ {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(
                self.individual
                    .iter()
                    .map(|expression| expression.get().bust),
            )
    }

    pub fn face(&self, face: Face) -> Option<AssetRef> {
        match face {
            Face::Generic(expression) => Some(self.generic.of(expression)),
            Face::Individual(expression) => self
                .individual
                .contains(&expression)
                .then(|| expression.get().bust),
        }
    }
}

impl GenericBusts {
    pub fn of(&self, expression: GenericExpression) -> AssetRef {
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
