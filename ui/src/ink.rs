use egui::Color32;

pub const CREAM: Color32 = Color32::from_rgb(0xEF, 0xE3, 0xC4);
pub const LIGHT: Color32 = Color32::from_rgb(0xF5, 0xEB, 0xD3);
pub(crate) const CREAM2: Color32 = Color32::from_rgb(0xE4, 0xD2, 0xA4);
pub const HONEY: Color32 = Color32::from_rgb(0xE2, 0xCB, 0x93);
pub const HONEY2: Color32 = Color32::from_rgb(0xD2, 0xB6, 0x76);
pub const INK: Color32 = Color32::from_rgb(0x2A, 0x1F, 0x15);
pub const FRAME: Color32 = Color32::from_rgb(0x2B, 0x21, 0x18);
pub const INK2: Color32 = Color32::from_rgb(0x5E, 0x4A, 0x33);
pub const INK3: Color32 = Color32::from_rgb(0x66, 0x56, 0x42);
pub const RUBRIC: Color32 = Color32::from_rgb(0x8E, 0x1F, 0x14);
pub const AZURE: Color32 = Color32::from_rgb(0x2D, 0x4E, 0x8C);
pub const VERDIGRIS: Color32 = Color32::from_rgb(0x2E, 0x62, 0x45);
pub const OCHRE: Color32 = Color32::from_rgb(0x7A, 0x52, 0x12);
pub const GOLD: Color32 = Color32::from_rgb(0xB8, 0x91, 0x2C);
pub const GOLD2: Color32 = Color32::from_rgb(0xE2, 0xC2, 0x5A);
pub const TYRIAN: Color32 = Color32::from_rgb(0x6B, 0x3A, 0x7A);
pub const WAX: Color32 = Color32::from_rgb(0xA8, 0x26, 0x1F);
pub const VERMILION: Color32 = Color32::from_rgb(0xC2, 0x1A, 0x14);
pub const SPRING: Color32 = Color32::from_rgb(0x7E, 0xE8, 0x8F);

pub const VEIL: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);

const PLACE: Color32 = AZURE;
pub const NAME: Color32 = VERDIGRIS;
pub const ITEM: Color32 = OCHRE;
pub const MAGIC: Color32 = TYRIAN;

/// What text is laid on: the paper of sheets, slips and cards, or the dusk of the notices that float
/// over the world, where the paper's inks would sink into the dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ground {
    #[default]
    Paper,
    Dusk,
}

/// What prose marks a word as, inked to read on either ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prose {
    Place,
    Name,
    Item,
    Magic,
    Danger,
}

impl Prose {
    pub const fn on(self, ground: Ground) -> Color32 {
        match (ground, self) {
            (Ground::Paper, Prose::Place) => PLACE,
            (Ground::Paper, Prose::Name) => NAME,
            (Ground::Paper, Prose::Item) => ITEM,
            (Ground::Paper, Prose::Magic) => MAGIC,
            (Ground::Paper, Prose::Danger) => RUBRIC,
            (Ground::Dusk, Prose::Place) => Color32::from_rgb(104, 168, 235),
            (Ground::Dusk, Prose::Name) => Color32::from_rgb(114, 180, 111),
            (Ground::Dusk, Prose::Item) => Color32::from_rgb(246, 201, 96),
            (Ground::Dusk, Prose::Magic) => Color32::from_rgb(190, 155, 240),
            (Ground::Dusk, Prose::Danger) => Color32::from_rgb(233, 117, 120),
        }
    }
}

pub fn wash(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

pub fn ink_wash(alpha: f32) -> Color32 {
    wash(INK, alpha)
}

pub(crate) fn white_wash(alpha: f32) -> Color32 {
    wash(Color32::WHITE, alpha)
}

pub fn gold_wash(alpha: f32) -> Color32 {
    wash(GOLD, alpha)
}

/// `color` as text over the dark `ground`: itself where it stands clear of it, else mixed toward
/// white until it does.
pub fn legible(color: Color32, ground: Color32) -> Color32 {
    (0..=LIFTS)
        .map(|lift| on(color, Color32::WHITE, f32::from(lift) / f32::from(LIFTS)))
        .find(|lifted| contrast(*lifted, ground) >= READABLE)
        .unwrap_or(Color32::WHITE)
}

/// `color` as text over the light `ground`: itself where it stands clear of it, else mixed toward
/// ink until it does.
pub fn inked(color: Color32, ground: Color32) -> Color32 {
    (0..=LIFTS)
        .map(|lift| on(color, INK, f32::from(lift) / f32::from(LIFTS)))
        .find(|darkened| contrast(*darkened, ground) >= READABLE)
        .unwrap_or(INK)
}

impl Ground {
    /// `color` as text that reads on this ground: darkened on paper, lifted on dusk.
    pub fn legible(self, color: Color32) -> Color32 {
        match self {
            Ground::Paper => inked(color, CREAM),
            Ground::Dusk => legible(color, VEIL),
        }
    }
}

/// How far two colours stand apart, from 1 for the same to 21 for black on white.
pub fn contrast(one: Color32, other: Color32) -> f32 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// The contrast text needs with what it lies on to be read.
pub const READABLE: f32 = 4.5;
const LIFTS: u8 = 16;

fn luminance(color: Color32) -> f32 {
    let linear = |channel: u8| {
        let share = f32::from(channel) / 255.0;
        if share <= 0.04045 {
            share / 12.92
        } else {
            ((share + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

pub fn on(base: Color32, over: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Color32::from_rgb(
        mix(base.r(), over.r()),
        mix(base.g(), over.g()),
        mix(base.b(), over.b()),
    )
}
