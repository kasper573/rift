use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, RichText, WidgetText};
use strum::{IntoStaticStr, VariantArray};

use crate::ink;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum Face {
    Book,
    BookMedium,
    BookBold,
    BookItalic,
    SmallCaps,
    SmallCapsBold,
    Quill,
    Numerals,
    Mono,
    MonoBold,
}

impl Face {
    pub fn family(self) -> FontFamily {
        match self {
            Face::Book => FontFamily::Proportional,
            Face::Mono => FontFamily::Monospace,
            other => FontFamily::Name(other.key().into()),
        }
    }

    pub fn id(self, size: f32) -> FontId {
        FontId::new(size, self.family())
    }

    pub(crate) fn key(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextRole {
    Title,
    NoteTitle,
    Section,
    Tab,
    MainTab,
    Band,
    Name,
    Body,
    CardBody,
    Line,
    Button,
    Tooltip,
    Small,
    Flavour,
    Numeral,
    KeyHint,
    Mono,
    Count,
    SlotKey,
}

impl TextRole {
    pub fn face(self) -> Face {
        match self {
            TextRole::Title
            | TextRole::Section
            | TextRole::Tab
            | TextRole::MainTab
            | TextRole::Band
            | TextRole::Name => Face::SmallCapsBold,
            TextRole::NoteTitle => Face::BookBold,
            TextRole::Body
            | TextRole::CardBody
            | TextRole::Line
            | TextRole::Button
            | TextRole::Tooltip
            | TextRole::Small => Face::Book,
            TextRole::Flavour => Face::Quill,
            TextRole::Numeral => Face::Numerals,
            TextRole::KeyHint | TextRole::Mono => Face::Mono,
            TextRole::Count | TextRole::SlotKey => Face::MonoBold,
        }
    }

    pub fn size(self) -> f32 {
        match self {
            TextRole::Title | TextRole::Line => 17.0,
            TextRole::NoteTitle | TextRole::Name => 16.0,
            TextRole::Section | TextRole::Tab | TextRole::KeyHint => 12.0,
            TextRole::Band => 12.5,
            TextRole::Body => 15.0,
            TextRole::CardBody | TextRole::Flavour => 14.5,
            TextRole::Button | TextRole::MainTab => 14.0,
            TextRole::Tooltip => 13.5,
            TextRole::Small | TextRole::Numeral | TextRole::Mono => 13.0,
            TextRole::Count => 11.0,
            TextRole::SlotKey => 10.0,
        }
    }

    /// Extra space between letters, in em.
    fn tracking(self) -> f32 {
        match self {
            TextRole::Title => 0.06,
            TextRole::Section => 0.1,
            TextRole::Tab | TextRole::MainTab => 0.08,
            TextRole::Name => 0.04,
            _ => 0.0,
        }
    }

    pub fn line_height(self) -> Option<f32> {
        match self {
            TextRole::Body => Some(self.size() * 1.4),
            TextRole::Line => Some(self.size() * 1.45),
            _ => None,
        }
    }

    pub fn font_id(self) -> FontId {
        self.face().id(self.size())
    }

    pub fn rich(self, text: impl Into<String>) -> RichText {
        let rich = RichText::new(text).font(self.font_id());
        match self.line_height() {
            Some(height) => rich.line_height(Some(height)),
            None => rich,
        }
    }

    pub fn job(self, text: &str, color: Color32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.append(text, 0.0, self.format(color));
        job
    }

    pub fn format(self, color: Color32) -> TextFormat {
        TextFormat {
            font_id: self.font_id(),
            extra_letter_spacing: self.tracking() * self.size(),
            line_height: self.line_height(),
            color,
            ..TextFormat::default()
        }
    }

    pub(crate) fn widget_text(self, text: &str, color: Color32) -> WidgetText {
        if self.tracking() == 0.0 {
            self.rich(text).color(color).into()
        } else {
            self.job(text, color).into()
        }
    }
}

pub fn body(text: impl Into<String>) -> RichText {
    TextRole::Body.rich(text)
}

pub fn secondary(text: impl Into<String>) -> RichText {
    TextRole::Body.rich(text).color(ink::INK2)
}

pub fn small(text: impl Into<String>) -> RichText {
    TextRole::Small.rich(text)
}

pub fn small_faint(text: impl Into<String>) -> RichText {
    TextRole::Small.rich(text).color(ink::INK3)
}

pub fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(Face::BookBold.family())
}

pub fn italic(text: impl Into<String>) -> RichText {
    RichText::new(text).family(Face::BookItalic.family())
}

pub fn quill(text: impl Into<String>) -> RichText {
    TextRole::Flavour.rich(text)
}

pub fn mono(text: impl Into<String>) -> RichText {
    TextRole::Mono.rich(text)
}

/// A Roman numeral for a card band; zero stays a zero and a count past the numerals' reach stays
/// in digits.
pub fn roman(n: u32) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    const STEPS: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = n;
    let mut out = String::new();
    for (value, numeral) in STEPS {
        while left >= value {
            out.push_str(numeral);
            left -= value;
        }
    }
    out
}
