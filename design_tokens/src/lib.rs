pub mod widgets;

use egui::epaint::Shadow;
use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak,
    Stroke, TextStyle,
};

pub mod color {
    use egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(24, 24, 27);
    pub const PANEL: Color32 = Color32::from_rgb(29, 29, 33);
    pub const RAISED: Color32 = Color32::from_rgb(38, 38, 43);
    pub const INPUT: Color32 = Color32::from_rgb(20, 20, 23);
    pub const BORDER: Color32 = Color32::from_rgb(50, 50, 57);
    pub const HOVER: Color32 = Color32::from_rgb(48, 48, 54);
    pub const HOVER_LINE: Color32 = Color32::from_rgb(80, 80, 90);
    pub const OVERLAY: Color32 = Color32::from_rgba_premultiplied(13, 13, 17, 235);
    pub const TEXT: Color32 = Color32::from_rgb(228, 228, 232);
    pub const MUTED: Color32 = Color32::from_rgb(142, 144, 156);
    pub const FAINT: Color32 = Color32::from_rgb(96, 98, 110);
    pub const ACCENT: Color32 = Color32::from_rgb(46, 110, 235);
    pub const ACCENT_SOFT: Color32 = Color32::from_rgb(30, 50, 96);
    pub const SELECTION: Color32 = Color32::from_rgb(120, 170, 250);
    pub const CODE: Color32 = Color32::from_rgb(132, 172, 240);
    pub const OK: Color32 = Color32::from_rgb(86, 196, 124);
    pub const WARN: Color32 = Color32::from_rgb(242, 176, 62);
    pub const DANGER: Color32 = Color32::from_rgb(226, 90, 90);
}

/// The colors prose marks words with: places, names, items, magic and danger.
pub mod ink {
    use egui::Color32;

    pub const PLACE: Color32 = Color32::from_rgb(104, 168, 235);
    pub const NAME: Color32 = Color32::from_rgb(114, 180, 111);
    pub const ITEM: Color32 = Color32::from_rgb(246, 201, 96);
    pub const MAGIC: Color32 = Color32::from_rgb(190, 155, 240);
    pub const DANGER: Color32 = Color32::from_rgb(233, 117, 120);
}

pub mod family {
    use egui::FontFamily;

    pub fn medium() -> FontFamily {
        FontFamily::Name("medium".into())
    }

    pub fn bold() -> FontFamily {
        FontFamily::Name("bold".into())
    }

    pub fn prose() -> FontFamily {
        FontFamily::Name("prose".into())
    }

    pub fn prose_italic() -> FontFamily {
        FontFamily::Name("prose-italic".into())
    }

    pub fn prose_bold() -> FontFamily {
        FontFamily::Name("prose-bold".into())
    }

    pub fn prose_bold_italic() -> FontFamily {
        FontFamily::Name("prose-bold-italic".into())
    }
}

/// Every control (button, field, combo, slider row) stands this tall: a body text row of 17 points
/// (14 points at Circular's line height of 1.2) inside the button padding, so an icon-only button
/// or one set in a smaller face lines up with the text buttons beside it.
pub const CONTROL_HEIGHT: f32 = 25.0;
pub const BUTTON_PADDING: egui::Vec2 = egui::Vec2::new(8.0, 4.0);

pub const WIDGET_RADIUS: CornerRadius = CornerRadius::same(4);
pub const WINDOW_RADIUS: CornerRadius = CornerRadius::same(6);
pub const CHIP_RADIUS: CornerRadius = CornerRadius::same(10);
pub const HAIRLINE: Stroke = Stroke {
    width: 1.0,
    color: color::BORDER,
};
pub const SHADOW: Shadow = Shadow {
    offset: [0, 6],
    blur: 18,
    spread: 0,
    color: Color32::from_black_alpha(140),
};

/// What a piece of text is for, which picks its face and size. The interface speaks in Circular;
/// what characters say and write is set in Lato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Heading,
    Body,
    Small,
    Monospace,
    Line,
    Prose,
}

impl TextRole {
    pub fn font_id(self) -> FontId {
        match self {
            TextRole::Heading => FontId::new(17.0, family::medium()),
            TextRole::Body => FontId::proportional(14.0),
            TextRole::Small => FontId::proportional(11.5),
            TextRole::Monospace => FontId::monospace(13.0),
            TextRole::Line => FontId::new(17.0, family::prose()),
            TextRole::Prose => FontId::new(14.0, family::prose()),
        }
    }

    pub fn size(self) -> f32 {
        self.font_id().size
    }
}

pub struct ThemeFonts {
    pub circular_400: FontData,
    pub circular_500: FontData,
    pub circular_700: FontData,
    pub lato_400: FontData,
    pub lato_400_italic: FontData,
    pub lato_700: FontData,
    pub lato_700_italic: FontData,
}

pub fn apply(ctx: &Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.global_style()).clone();
    for (text, role) in [
        (TextStyle::Body, TextRole::Body),
        (TextStyle::Button, TextRole::Body),
        (TextStyle::Small, TextRole::Small),
        (TextStyle::Heading, TextRole::Heading),
        (TextStyle::Monospace, TextRole::Monospace),
    ] {
        style.text_styles.insert(text, role.font_id());
    }
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = BUTTON_PADDING;
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.interaction.selectable_labels = false;
    let visuals = &mut style.visuals;
    visuals.dark_mode = true;
    visuals.override_text_color = Some(color::TEXT);
    visuals.panel_fill = color::PANEL;
    visuals.window_fill = color::RAISED;
    visuals.window_stroke = HAIRLINE;
    visuals.extreme_bg_color = color::INPUT;
    visuals.faint_bg_color = color::RAISED;
    visuals.code_bg_color = color::INPUT;
    visuals.hyperlink_color = color::CODE;
    visuals.warn_fg_color = color::WARN;
    visuals.error_fg_color = color::DANGER;
    visuals.selection.bg_fill = color::ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0, color::SELECTION);
    visuals.widgets.noninteractive.bg_fill = color::PANEL;
    visuals.widgets.noninteractive.bg_stroke = HAIRLINE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, color::TEXT);
    visuals.widgets.inactive.bg_fill = color::RAISED;
    visuals.widgets.inactive.weak_bg_fill = color::RAISED;
    visuals.widgets.inactive.bg_stroke = HAIRLINE;
    // egui sizes a button that is frameless at rest by its padding, grown by this and shrunk by
    // its border, and sizes it once hovered by its padding alone.
    visuals.widgets.inactive.expansion = HAIRLINE.width;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, color::TEXT);
    visuals.widgets.hovered.bg_fill = color::HOVER;
    visuals.widgets.hovered.weak_bg_fill = color::HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, color::HOVER_LINE);
    visuals.widgets.active.bg_fill = color::ACCENT_SOFT;
    visuals.widgets.active.weak_bg_fill = color::ACCENT_SOFT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, color::ACCENT);
    visuals.widgets.open.bg_fill = color::RAISED;
    visuals.widgets.open.weak_bg_fill = color::RAISED;
    visuals.window_corner_radius = WINDOW_RADIUS;
    visuals.menu_corner_radius = WINDOW_RADIUS;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = WIDGET_RADIUS;
    }
    ctx.all_styles_mut(|target| *target = style.clone());
}

/// Circular for the interface families, Lato for the prose ones. egui's built-in faces stay behind
/// both for the glyphs these lack.
pub fn fonts(faces: ThemeFonts) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    // Circular asks for its typographic metrics, whose 0.2 em line gap egui lays entirely under
    // the descender, so its capitals land well above the middle of their row and every button's
    // text rides high beside the icons centred on that row. Shifting the glyphs down by the gap
    // puts the capitals on the row's middle as drawn.
    let circular = |data: FontData| {
        data.tweak(FontTweak {
            y_offset_factor: 0.2,
            ..FontTweak::default()
        })
    };
    let faces = [
        ("circular-400", circular(faces.circular_400)),
        ("circular-500", circular(faces.circular_500)),
        ("circular-700", circular(faces.circular_700)),
        ("lato-400", faces.lato_400),
        ("lato-400-italic", faces.lato_400_italic),
        ("lato-700", faces.lato_700),
        ("lato-700-italic", faces.lato_700_italic),
    ];
    for (name, data) in faces {
        fonts.font_data.insert(name.to_owned(), data.into());
    }
    name_families(
        &mut fonts,
        &[
            (FontFamily::Proportional, &["circular-400"]),
            (family::medium(), &["circular-500", "circular-400"]),
            (family::bold(), &["circular-700", "circular-400"]),
            (family::prose(), &["lato-400"]),
            (family::prose_italic(), &["lato-400-italic", "lato-400"]),
            (family::prose_bold(), &["lato-700", "lato-400"]),
            (
                family::prose_bold_italic(),
                &["lato-700-italic", "lato-700", "lato-400"],
            ),
        ],
    );
    fonts
}

/// egui's built-in faces under every family the theme names, for text drawn before the real
/// faces are installed.
pub fn builtin_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    name_families(
        &mut fonts,
        &[
            (family::medium(), &[]),
            (family::bold(), &[]),
            (family::prose(), &[]),
            (family::prose_italic(), &[]),
            (family::prose_bold(), &[]),
            (family::prose_bold_italic(), &[]),
        ],
    );
    fonts
}

fn name_families(fonts: &mut FontDefinitions, families: &[(FontFamily, &[&str])]) {
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, faces) in families {
        let chain = faces
            .iter()
            .map(|face| (*face).to_owned())
            .chain(fallback.iter().cloned())
            .collect();
        fonts.families.insert(family.clone(), chain);
    }
}
