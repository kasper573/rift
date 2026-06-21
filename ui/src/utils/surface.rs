use bevy_color::Color;
use bevy_ui::{BoxShadow, ShadowStyle, Val};

/// Elevation shadow for surface cards. A card's depth is a shadow, never a 1px border — a border on
/// a rounded box leaks white at the corners. `level` scales offset and blur (1 = resting, 2 = raised).
pub(crate) fn elevation(level: u8) -> BoxShadow {
    let scale = level as f32;
    BoxShadow(vec![
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(1.0 * scale),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(2.0 * scale),
        },
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(4.0 * scale),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(12.0 * scale),
        },
    ])
}
