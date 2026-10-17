use design_tokens::ThemeFonts;
use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, RichText};

pub use design_tokens::SHADOW;
pub use design_tokens::color::{
    ACCENT, ACCENT_SOFT, BG, BORDER, CODE, DANGER, FAINT, INPUT, MUTED, OK, OVERLAY, PANEL, RAISED,
    SELECTION, TEXT, WARN,
};
pub use design_tokens::family::medium;

pub const VARIABLE: Color32 = Color32::from_rgb(176, 140, 230);
pub const VARIABLE_FILL: Color32 = Color32::from_rgba_premultiplied(21, 16, 27, 30);
pub const CANVAS: Color32 = Color32::from_rgb(20, 20, 24);
pub const BLOCKED: Color32 = Color32::from_rgb(220, 60, 60);
pub const SORT_LINE: Color32 = Color32::from_rgb(120, 170, 255);

pub fn apply(ctx: &Context) {
    ctx.set_fonts(fonts());
    design_tokens::apply(ctx);
}

/// The theme's faces, bundled from the new-game template, and a symbols face for the editor's
/// glyphs (arrows, music notes, box marks). The named families reach it before egui's built-ins.
fn fonts() -> FontDefinitions {
    let face = |bytes: &'static [u8]| FontData::from_static(bytes);
    let mut fonts = design_tokens::fonts(ThemeFonts {
        circular_400: face(include_bytes!(
            "../../templates/new_game/assets/fonts/circular-400-normal.ttf"
        )),
        circular_500: face(include_bytes!(
            "../../templates/new_game/assets/fonts/circular-500-normal.ttf"
        )),
        circular_700: face(include_bytes!(
            "../../templates/new_game/assets/fonts/circular-700-normal.ttf"
        )),
        lato_400: face(include_bytes!(
            "../../templates/new_game/assets/fonts/lato-400-normal.ttf"
        )),
        lato_400_italic: face(include_bytes!(
            "../../templates/new_game/assets/fonts/lato-400-italic.ttf"
        )),
        lato_700: face(include_bytes!(
            "../../templates/new_game/assets/fonts/lato-700-normal.ttf"
        )),
        lato_700_italic: face(include_bytes!(
            "../../templates/new_game/assets/fonts/lato-700-italic.ttf"
        )),
    });
    fonts.font_data.insert(
        "symbols".to_owned(),
        FontData::from_static(include_bytes!("../../assets/DejaVuSans.ttf")).into(),
    );
    let builtins: Vec<String> = FontDefinitions::default().font_data.into_keys().collect();
    for (family, faces) in fonts.families.iter_mut() {
        let at = match family {
            FontFamily::Name(_) => faces
                .iter()
                .position(|face| builtins.contains(face))
                .unwrap_or(faces.len()),
            _ => faces.len(),
        };
        faces.insert(at, "symbols".to_owned());
    }
    fonts
}

pub fn ok(text: impl Into<String>) -> RichText {
    RichText::new(text).color(OK)
}

pub fn warn(text: impl Into<String>) -> RichText {
    RichText::new(text).color(WARN)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn faint(text: impl Into<String>) -> RichText {
    RichText::new(text).color(FAINT)
}

pub fn code(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().color(CODE)
}

pub fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace()
}

pub fn alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(medium()).color(TEXT)
}

pub fn badge_color(kind: &str) -> Color32 {
    match kind {
        "population" | "resident" | "npc" | "player" | "nameplate" | "tint" | "voice" => {
            Color32::from_rgb(214, 168, 42)
        }
        "object" | "safe_zone" | "area" | "walk" => Color32::from_rgb(76, 170, 96),
        "shared_script" | "inline_script" | "script" | "shown" | "requires" => {
            Color32::from_rgb(204, 96, 164)
        }
        "soundscape" | "sfx" | "babble" | "step" => Color32::from_rgb(134, 110, 220),
        "loot" | "item" | "stack" | "reward" | "hold" | "deliver" | "offer" | "shop" => {
            Color32::from_rgb(52, 160, 184)
        }
        "blueprint" | "scenery" | "tileset" | "model" | "bust" => Color32::from_rgb(96, 128, 196),
        "respawn" | "quest" | "defeat" | "hand_in" | "task" | "decide" | "pick_one" => {
            Color32::from_rgb(222, 112, 96)
        }
        _ => Color32::from_rgb(120, 124, 136),
    }
}

pub fn badge_letter(kind: &str) -> String {
    match kind {
        "shared_script" | "inline_script" | "script" => "S".to_owned(),
        "population" | "resident" | "npc" => "N".to_owned(),
        "object" => "P".to_owned(),
        "safe_zone" => "Z".to_owned(),
        "soundscape" => "♪".to_owned(),
        "blueprint" | "scenery" => "L".to_owned(),
        "loot" | "item" => "I".to_owned(),
        "respawn" => "F".to_owned(),
        "quest" => "R".to_owned(),
        "tint" | "voice" | "nameplate" => "L".to_owned(),
        other => other
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_default(),
    }
}

pub const TERRAIN: [Color32; 8] = [
    Color32::from_rgb(230, 160, 70),
    Color32::from_rgb(150, 110, 90),
    Color32::from_rgb(240, 240, 220),
    Color32::from_rgb(190, 200, 210),
    Color32::from_rgb(110, 180, 110),
    Color32::from_rgb(90, 150, 230),
    Color32::from_rgb(200, 110, 200),
    Color32::from_rgb(230, 90, 90),
];
