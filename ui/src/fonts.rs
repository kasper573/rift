use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontTweak};
use strum::VariantArray;

use crate::face::Face;
use crate::glyph::Weight;

/// The families, every face and icon weight its own and none falling back on another: a glyph a
/// face lacks is not drawn as text at all.
pub(crate) fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    for &face in Face::VARIANTS {
        let data = FontData::from_static(bytes(face)).tweak(tweak(face));
        fonts
            .font_data
            .insert(face.key().to_owned(), Arc::new(data));
        fonts
            .families
            .insert(face.family(), vec![face.key().to_owned()]);
    }
    for &weight in Weight::VARIANTS {
        fonts.font_data.insert(
            weight.key().to_owned(),
            Arc::new(FontData::from_static(weight.font())),
        );
        fonts
            .families
            .insert(weight.family(), vec![weight.key().to_owned()]);
    }
    fonts
}

pub(crate) fn bytes(face: Face) -> &'static [u8] {
    match face {
        Face::Book => include_bytes!("../fonts/Alegreya-Regular.ttf"),
        Face::BookMedium => include_bytes!("../fonts/Alegreya-Medium.ttf"),
        Face::BookBold => include_bytes!("../fonts/Alegreya-Bold.ttf"),
        Face::BookItalic => include_bytes!("../fonts/Alegreya-Italic.ttf"),
        Face::BookMediumItalic => include_bytes!("../fonts/Alegreya-MediumItalic.ttf"),
        Face::SmallCaps => include_bytes!("../fonts/AlegreyaSC-Medium.ttf"),
        Face::SmallCapsBold => include_bytes!("../fonts/AlegreyaSC-Bold.ttf"),
        Face::Quill => include_bytes!("../fonts/IMFellEnglish-Italic.ttf"),
        Face::Mono => include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
        Face::MonoBold => include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    }
}

// In a row centred on Alegreya's metrics its capitals sit on the row's middle, but its x-height,
// and the old-style figures that share it, sit a tenth of the size below it, so a line of mostly
// lowercase text looks dropped in any box drawn around it. Lifting the glyphs halfway between
// the two centres puts both within a hair of the middle.
fn tweak(face: Face) -> FontTweak {
    match face {
        Face::Book
        | Face::BookMedium
        | Face::BookBold
        | Face::BookItalic
        | Face::BookMediumItalic => FontTweak {
            y_offset_factor: -0.06,
            ..FontTweak::default()
        },
        Face::SmallCaps | Face::SmallCapsBold => FontTweak {
            y_offset_factor: -0.05,
            ..FontTweak::default()
        },
        _ => FontTweak::default(),
    }
}
