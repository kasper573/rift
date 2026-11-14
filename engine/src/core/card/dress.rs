use ui::card::{
    ArtLook, BorderLook, CardDress, CornerLook, CrestLook, DividerLook, HaloLook, LabelLineLook,
    NameBandLook, PaperLook,
};
use ui::ink;

use super::{
    CardArt, CardBorder, CardCorners, CardCrest, CardDividers, CardHalo, CardLabelLine,
    CardNameBand, CardPaper, CardStyleDef,
};
use crate::core::content::ImageRef;
use crate::core::gui::ImageSource;
use crate::core::math::ColorGradient;

impl CardStyleDef {
    /// The style as a card is drawn in it, its colours and pictures ready. `ground` stands in
    /// place of the style's own behind the picture, for a card that brings its own.
    pub fn dress(
        &self,
        images: &mut (impl ImageSource + ?Sized),
        ground: Option<ColorGradient>,
    ) -> CardDress<'_> {
        let mut texture = |image: &ImageRef| images.image(image).map(|image| image.texture);
        CardDress {
            ground: ground.unwrap_or(self.ground).colors(),
            gradient: self.gradient.map_or([ink::FRAME; 2], ColorGradient::colors),
            icon: self.icon.as_ref().and_then(&mut texture),
            label: self.label.as_deref(),
            border: match &self.border {
                CardBorder::Ink => BorderLook::Ink,
                CardBorder::Gradient { width } => BorderLook::Gradient { width: width.0 },
                CardBorder::Double { width, gap } => BorderLook::Double {
                    width: width.0,
                    gap: gap.0,
                },
                // Until its image is loaded, the border is the plain stroke the image tints.
                CardBorder::Patterned { image, width } => match texture(image) {
                    Some(image) => BorderLook::Patterned {
                        image,
                        width: width.0,
                    },
                    None => BorderLook::Gradient { width: width.0 },
                },
            },
            corners: match self.corners {
                CardCorners::Round { radius } => CornerLook::Round { radius: radius.0 },
                CardCorners::Cut { size } => CornerLook::Cut { size: size.0 },
                CardCorners::Notched { size } => CornerLook::Notched { size: size.0 },
            },
            crest: match self.crest {
                CardCrest::None => CrestLook::None,
                CardCrest::Icon { size } => CrestLook::Icon { size: size.0 },
                CardCrest::Banner => CrestLook::Banner,
            },
            name_band: match self.name_band {
                CardNameBand::Plain { icon } => NameBandLook::Plain { icon },
                CardNameBand::Washed { strength, icon } => NameBandLook::Washed { strength, icon },
                CardNameBand::Filled { icon } => NameBandLook::Filled { icon },
                CardNameBand::Ribbon { overhang, icon } => NameBandLook::Ribbon {
                    overhang: overhang.0,
                    icon,
                },
            },
            label_line: match self.label_line {
                CardLabelLine::None => LabelLineLook::None,
                CardLabelLine::Strip => LabelLineLook::Strip,
                CardLabelLine::Seal => LabelLineLook::Seal,
            },
            art: match self.art {
                CardArt::Flush => ArtLook::Flush,
                CardArt::Inset { margin } => ArtLook::Inset { margin: margin.0 },
            },
            paper: match self.paper {
                CardPaper::Cream => PaperLook::Cream,
                CardPaper::Tinted { strength } => PaperLook::Tinted { strength },
                CardPaper::Watermarked { strength } => PaperLook::Watermarked { strength },
            },
            dividers: match self.dividers {
                CardDividers::Line => DividerLook::Line,
                CardDividers::Gradient => DividerLook::Gradient,
                CardDividers::Ornament => DividerLook::Ornament,
            },
            halo: match self.halo {
                CardHalo::Shadow => HaloLook::Shadow,
                CardHalo::Glow { spread } => HaloLook::Glow { spread: spread.0 },
            },
        }
    }
}
