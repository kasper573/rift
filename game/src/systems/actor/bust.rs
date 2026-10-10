use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::core::assets::AssetRef;
use crate::core::content::Content;

pub use crate::data::expression::Id as IndividualExpression;

#[derive(Clone)]
pub struct ModelDef {
    pub sheet: AssetRef,
    pub busts: Option<Busts>,
}

impl crate::core::content::ContentRow for ModelDef {
    const TABLE: &'static str = "model";
}

#[derive(Clone)]
pub struct Busts {
    pub generic: GenericBusts,
    pub individual: &'static [IndividualExpression],
}

#[derive(Clone)]
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

#[derive(Clone)]
pub struct IndividualExpressionDef {
    pub bust: AssetRef,
}

impl crate::core::content::ContentRow for IndividualExpressionDef {
    const TABLE: &'static str = "expression";
}

impl Busts {
    pub fn all<'a>(&'a self, content: &'a Content) -> impl Iterator<Item = AssetRef> + 'a {
        GenericExpression::VARIANTS
            .iter()
            .map(|&expression| self.generic.of(expression))
            .chain(
                self.individual
                    .iter()
                    .map(|expression| expression.get(content).bust),
            )
    }

    pub fn face(&self, content: &Content, face: Face) -> Option<AssetRef> {
        match face {
            Face::Generic(expression) => Some(self.generic.of(expression)),
            Face::Individual(expression) => self
                .individual
                .contains(&expression)
                .then(|| expression.get(content).bust),
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
