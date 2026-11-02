use egui::{
    Align2, Color32, ColorImage, FontFamily, FontId, Id, Painter, Pos2, Rect, Response, RichText,
    Sense, Shape, Stroke, TextureHandle, TextureId, TextureOptions, Ui, Vec2, WidgetInfo,
    WidgetType,
};
use strum::VariantArray;

pub use egui_phosphor::regular as phosphor;

use crate::face::TextRole;
use crate::ink;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray)]
pub enum CardKind {
    Equipment,
    Usable,
    Resource,
    Quest,
    Active,
    Passive,
}

impl CardKind {
    pub fn word(self) -> &'static str {
        match self {
            CardKind::Equipment => "equipment",
            CardKind::Usable => "usable",
            CardKind::Resource => "resource",
            CardKind::Quest => "quest",
            CardKind::Active => "active skill",
            CardKind::Passive => "passive skill",
        }
    }

    pub fn ground(self) -> [Color32; 2] {
        match self {
            CardKind::Equipment => [Color32::from_rgb(0x3A, 0x5E, 0xA3), ink::AZURE],
            CardKind::Usable | CardKind::Active => [Color32::from_rgb(0xC8, 0x30, 0x2A), ink::WAX],
            CardKind::Resource => [
                Color32::from_rgb(0xE3, 0xB3, 0x35),
                Color32::from_rgb(0xC4, 0x92, 0x1E),
            ],
            CardKind::Quest => [ink::TYRIAN, Color32::from_rgb(0x4F, 0x2A, 0x5C)],
            CardKind::Passive => [
                Color32::from_rgb(0x4A, 0x94, 0x60),
                Color32::from_rgb(0x3F, 0x7A, 0x58),
            ],
        }
    }

    pub fn texture(self, ctx: &egui::Context) -> TextureId {
        let id = Id::new(("kind glyph", self));
        if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
            return texture.id();
        }
        let texture = load(ctx, self.word(), self.png());
        ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
        texture.id()
    }

    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32, upside_down: bool) {
        let uv = if upside_down {
            Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::ZERO)
        } else {
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
        };
        painter.add(Shape::image(self.texture(painter.ctx()), rect, uv, color));
    }

    fn png(self) -> &'static [u8] {
        match self {
            CardKind::Equipment => include_bytes!("../glyphs/equipment.png"),
            CardKind::Usable => include_bytes!("../glyphs/usable.png"),
            CardKind::Resource => include_bytes!("../glyphs/resource.png"),
            CardKind::Quest => include_bytes!("../glyphs/quest.png"),
            CardKind::Active => include_bytes!("../glyphs/active.png"),
            CardKind::Passive => include_bytes!("../glyphs/passive.png"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray)]
pub enum Mark {
    Check,
    Cross,
    Flag,
    Lock,
    Unlock,
    CaretUp,
    CaretDown,
    CaretLeft,
    CaretRight,
    Warning,
    Play,
    Pause,
    Stop,
    Skip,
    Dot,
    Plus,
    Minus,
    Search,
    Music,
    Reload,
    Undo,
    Pin,
    Eye,
    EyeSlash,
    Folder,
    FolderOpen,
    Settings,
    Bell,
    Speaker,
    Mute,
    Duplicate,
    Paint,
    Edit,
    Move,
    File,
    Copy,
    Crosshair,
    Grid,
    List,
    Tag,
    More,
    Shift,
    Arrow,
    Raise,
}

impl Mark {
    pub fn word(self) -> &'static str {
        match self {
            Mark::Check => "yes",
            Mark::Cross => "no",
            Mark::Flag => "flag",
            Mark::Lock => "locked",
            Mark::Unlock => "unlocked",
            Mark::CaretUp => "up",
            Mark::CaretDown => "down",
            Mark::CaretLeft => "left",
            Mark::CaretRight => "right",
            Mark::Warning => "warning",
            Mark::Play => "play",
            Mark::Pause => "pause",
            Mark::Stop => "stop",
            Mark::Skip => "skip",
            Mark::Dot => "dot",
            Mark::Plus => "add",
            Mark::Minus => "remove",
            Mark::Search => "search",
            Mark::Music => "sound",
            Mark::Reload => "reload",
            Mark::Undo => "undo",
            Mark::Pin => "pin",
            Mark::Eye => "shown",
            Mark::EyeSlash => "hidden",
            Mark::Folder => "folder",
            Mark::FolderOpen => "open folder",
            Mark::Settings => "settings",
            Mark::Bell => "bell",
            Mark::Speaker => "sound on",
            Mark::Mute => "sound off",
            Mark::Duplicate => "duplicate",
            Mark::Paint => "paint",
            Mark::Edit => "edit",
            Mark::Move => "move",
            Mark::File => "page",
            Mark::Copy => "copy",
            Mark::Crosshair => "pick",
            Mark::Grid => "tiles",
            Mark::List => "list",
            Mark::Tag => "rename",
            Mark::More => "more",
            Mark::Shift => "Shift",
            Mark::Arrow => "arrow",
            Mark::Raise => "above",
        }
    }
}

impl Mark {
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        self.icon().paint(painter, rect, color);
    }

    pub fn text(self, size: f32) -> RichText {
        self.icon().text(size)
    }

    pub fn icon(self) -> Icon {
        Icon::regular(match self {
            Mark::Check => phosphor::CHECK,
            Mark::Cross => phosphor::X,
            Mark::Flag => phosphor::FLAG,
            Mark::Lock => phosphor::LOCK,
            Mark::Unlock => phosphor::LOCK_OPEN,
            Mark::CaretUp => phosphor::CARET_UP,
            Mark::CaretDown => phosphor::CARET_DOWN,
            Mark::CaretLeft => phosphor::CARET_LEFT,
            Mark::CaretRight => phosphor::CARET_RIGHT,
            Mark::Warning => phosphor::WARNING,
            Mark::Play => phosphor::PLAY,
            Mark::Pause => phosphor::PAUSE,
            Mark::Stop => phosphor::STOP,
            Mark::Skip => phosphor::SKIP_FORWARD,
            Mark::Dot => phosphor::DOT_OUTLINE,
            Mark::Plus => phosphor::PLUS,
            Mark::Minus => phosphor::MINUS,
            Mark::Search => phosphor::MAGNIFYING_GLASS,
            Mark::Music => phosphor::MUSIC_NOTE,
            Mark::Reload => phosphor::ARROW_CLOCKWISE,
            Mark::Undo => phosphor::ARROW_COUNTER_CLOCKWISE,
            Mark::Pin => phosphor::PUSH_PIN,
            Mark::Eye => phosphor::EYE,
            Mark::EyeSlash => phosphor::EYE_SLASH,
            Mark::Folder => phosphor::FOLDER,
            Mark::FolderOpen => phosphor::FOLDER_OPEN,
            Mark::Settings => phosphor::GEAR_SIX,
            Mark::Bell => phosphor::BELL,
            Mark::Speaker => phosphor::SPEAKER_HIGH,
            Mark::Mute => phosphor::SPEAKER_X,
            Mark::Duplicate => phosphor::COPY_SIMPLE,
            Mark::Paint => phosphor::PAINT_BRUSH,
            Mark::Edit => phosphor::PENCIL_SIMPLE,
            Mark::Move => phosphor::ARROWS_OUT_CARDINAL,
            Mark::File => phosphor::FILE,
            Mark::Copy => phosphor::COPY,
            Mark::Crosshair => phosphor::CROSSHAIR,
            Mark::Grid => phosphor::GRID_FOUR,
            Mark::List => phosphor::LIST,
            Mark::Tag => phosphor::TAG,
            Mark::More => phosphor::DOTS_THREE,
            Mark::Shift => phosphor::ARROW_FAT_UP,
            Mark::Arrow => phosphor::ARROW_RIGHT,
            Mark::Raise => phosphor::ARROW_LINE_UP,
        })
    }
}

/// The weights the icon font comes in; every weight draws the same icons at the same codepoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, VariantArray)]
pub enum Weight {
    Regular,
    Fill,
}

impl Weight {
    pub fn family(self) -> FontFamily {
        FontFamily::Name(self.key().into())
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Weight::Regular => "phosphor",
            Weight::Fill => "phosphor fill",
        }
    }

    pub(crate) fn font(self) -> &'static [u8] {
        match self {
            Weight::Regular => egui_phosphor::bytes::regular::FONT,
            Weight::Fill => egui_phosphor::bytes::fill::FONT,
        }
    }
}

/// One icon of the icon font, by its codepoint in [`phosphor`], in one of its weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon {
    pub glyph: &'static str,
    pub weight: Weight,
}

impl Icon {
    pub const fn regular(glyph: &'static str) -> Icon {
        Icon {
            glyph,
            weight: Weight::Regular,
        }
    }

    pub const fn fill(glyph: &'static str) -> Icon {
        Icon {
            glyph,
            weight: Weight::Fill,
        }
    }

    /// The icon as text `size` tall, for a widget that shows text.
    pub fn text(self, size: f32) -> RichText {
        RichText::new(self.glyph)
            .family(self.weight.family())
            .size(size)
    }

    /// Draws the icon as large as fits in `rect`, centred on it.
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let size = rect.width().min(rect.height());
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            self.glyph,
            FontId::new(size, self.weight.family()),
            color,
        );
    }
}

pub fn show(ui: &mut Ui, mark: Mark, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, mark.word()));
    if ui.is_rect_visible(rect) {
        mark.paint(ui.painter(), rect, color);
    }
    response
}

pub fn button(ui: &mut Ui, mark: Mark, size: f32, color: Color32, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 3.0, ink::ink_wash(0.1));
        }
        let color = if ui.is_enabled() { color } else { ink::INK3 };
        mark.paint(ui.painter(), rect.shrink(size * 0.15), color);
        if response.has_focus() {
            crate::controls::focus_ring(ui.painter(), rect, 3.0);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A bright disc with an exclamation mark on it, `radius` across from `at`, for what needs a look.
pub fn alert(painter: &Painter, at: Pos2, radius: f32) {
    painter.circle(at, radius, ink::GOLD2, Stroke::new(1.0, ink::INK));
    Icon::fill(phosphor::EXCLAMATION_MARK).paint(
        painter,
        Rect::from_center_size(at, Vec2::splat(radius * 1.5)),
        ink::INK,
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Halo {
    pub color: Color32,
    pub width: f32,
}

pub fn haloed(
    painter: &Painter,
    at: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    halo: Halo,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let rect = anchor.anchor_size(at, galley.size());
    for offset in [
        Vec2::new(-halo.width, 0.0),
        Vec2::new(halo.width, 0.0),
        Vec2::new(0.0, -halo.width),
        Vec2::new(0.0, halo.width),
    ] {
        painter.galley_with_override_text_color(rect.min + offset, galley.clone(), halo.color);
    }
    painter.galley(rect.min, galley, color);
    rect
}

/// Faint diagonal hatching over a rect, for something held out of use.
pub fn hatch(painter: &Painter, rect: Rect, color: Color32) {
    hatch_every(painter, rect, color, 6.0);
}

/// Hatching across `rect` with `step` between its lines.
pub(crate) fn hatch_every(painter: &Painter, rect: Rect, color: Color32, step: f32) {
    let stroke = Stroke::new(1.0, color);
    let span = rect.width() + rect.height();
    let mut offset = 0.0;
    while offset < span {
        let start = Pos2::new(
            (rect.min.x + offset).min(rect.max.x),
            rect.min.y + (offset - rect.width()).max(0.0),
        );
        let end = Pos2::new(
            rect.min.x + (offset - rect.height()).max(0.0),
            (rect.min.y + offset).min(rect.max.y),
        );
        painter.line_segment([start, end], stroke);
        offset += step;
    }
}

/// A line from `from` ending in an arrowhead at `to`, dashed when asked.
pub fn arrow(painter: &Painter, from: Pos2, to: Pos2, stroke: Stroke, dashed: bool) {
    let delta = to - from;
    let length = delta.length();
    if length <= f32::EPSILON {
        return;
    }
    let dir = delta / length;
    if dashed {
        painter.add(Shape::dashed_line(&[from, to], stroke, 6.0, 4.0));
    } else {
        painter.line_segment([from, to], stroke);
    }
    arrow_head(painter, to, dir, 7.0, stroke.color);
}

pub fn dashed_circle(painter: &Painter, center: Pos2, radius: f32, stroke: Stroke) {
    let points: Vec<Pos2> = (0..=48)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 48.0;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(Shape::dashed_line(&points, stroke, 4.0, 3.0));
}

pub fn badge(painter: &Painter, at: Pos2, anchor: Align2, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), TextRole::Count.font_id(), color);
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let rect = anchor.anchor_size(at, size);
    painter.rect(
        rect,
        3.0,
        ink::wash(ink::CREAM, 0.9),
        Stroke::new(1.0, ink::ink_wash(0.45)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

pub fn arrow_head(painter: &Painter, tip: Pos2, dir: Vec2, size: f32, color: Color32) {
    let dir = dir.normalized();
    let side = Vec2::new(-dir.y, dir.x);
    painter.add(Shape::convex_polygon(
        vec![
            tip,
            tip - dir * size + side * size * 0.55,
            tip - dir * size - side * size * 0.55,
        ],
        color,
        Stroke::NONE,
    ));
}

fn load(ctx: &egui::Context, name: &str, png: &[u8]) -> TextureHandle {
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .unwrap_or_else(|error| panic!("the {name} glyph is not a png: {error}"))
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let pixels = ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    ctx.load_texture(format!("kind glyph {name}"), pixels, TextureOptions::LINEAR)
}
