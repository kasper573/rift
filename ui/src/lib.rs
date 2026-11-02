pub use egui;
pub use egui_code_editor;
pub use egui_dock;
pub use egui_ltreeview;
pub use egui_snarl;

pub mod card;
pub mod chip;
pub mod controls;
pub mod dialog;
pub mod draft;
pub mod drag;
pub mod dusk;
pub mod face;
pub mod float;
mod fonts;
pub mod form;
pub mod glyph;
pub mod ink;
pub mod input;
pub mod keys;
pub mod menu;
pub mod meter;
pub mod motion;
pub mod paint;
pub mod panel;
pub mod raster;
pub mod sheet;
pub mod slip;
pub mod theme;
pub mod words;

use egui::{Context, Margin, Stroke, Style, TextStyle};

use crate::face::TextRole;

const TOOLTIP_DELAY_SECS: f32 = 0.4;
const TOOLTIP_GRACE_SECS: f32 = 0.3;

pub fn apply(ctx: &Context) {
    ctx.set_fonts(fonts::definitions());
    ctx.set_theme(egui::Theme::Light);
    ctx.all_styles_mut(style);
}

fn style(style: &mut Style) {
    for (text, role) in [
        (TextStyle::Body, TextRole::Body),
        (TextStyle::Button, TextRole::Button),
        (TextStyle::Small, TextRole::Small),
        (TextStyle::Heading, TextRole::Title),
        (TextStyle::Monospace, TextRole::Mono),
    ] {
        style.text_styles.insert(text, role.font_id());
    }
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(6.0, 5.0);
    spacing.button_padding = controls::BUTTON_PADDING;
    spacing.interact_size.y = controls::CONTROL_HEIGHT;
    // egui's default menu height holds ten of its own rows; ten of these taller ones keep fitting.
    spacing.combo_height = 10.0 * (controls::CONTROL_HEIGHT + spacing.item_spacing.y);
    spacing.icon_width = 14.0;
    spacing.icon_width_inner = 8.0;
    spacing.icon_spacing = 6.0;
    spacing.slider_width = 150.0;
    spacing.menu_margin = slip::PADDING;
    spacing.window_margin = Margin::ZERO;
    spacing.tooltip_width = 260.0;
    spacing.scroll.floating = false;
    spacing.scroll.bar_width = 8.0;
    spacing.scroll.bar_inner_margin = 2.0;
    spacing.scroll.bar_outer_margin = 0.0;
    spacing.scroll.handle_min_length = 24.0;
    spacing.scroll.foreground_color = false;
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = TOOLTIP_DELAY_SECS;
    style.interaction.tooltip_grace_time = TOOLTIP_GRACE_SECS;
    // A window's corner reaches this far past its edge for a resize, which would take the clicks
    // meant for whatever lies beside it.
    style.interaction.resize_grab_radius_corner = style.interaction.resize_grab_radius_side;
    let visuals = &mut style.visuals;
    visuals.dark_mode = false;
    visuals.override_text_color = Some(ink::INK);
    visuals.weak_text_color = Some(ink::INK3);
    visuals.panel_fill = ink::CREAM;
    visuals.window_fill = ink::CREAM;
    visuals.window_stroke = Stroke::NONE;
    visuals.window_shadow = sheet::shadow(sheet::Depth::Front);
    visuals.popup_shadow = slip::SHADOW;
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.menu_corner_radius = egui::CornerRadius::ZERO;
    visuals.window_highlight_topmost = false;
    visuals.extreme_bg_color = ink::on(ink::CREAM, ink::INK, 0.04);
    visuals.faint_bg_color = ink::on(ink::CREAM, ink::INK, 0.05);
    visuals.code_bg_color = ink::on(ink::CREAM, ink::INK, 0.05);
    visuals.hyperlink_color = ink::AZURE;
    visuals.warn_fg_color = ink::OCHRE;
    visuals.error_fg_color = ink::RUBRIC;
    visuals.selection.bg_fill = ink::on(ink::CREAM, ink::GOLD, 0.35);
    visuals.selection.stroke = Stroke::new(1.0, ink::INK);
    visuals.text_cursor.stroke = Stroke::new(1.5, ink::INK);
    visuals.slider_trailing_fill = true;
    visuals.striped = false;
    visuals.resize_corner_size = sheet::CURL;
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
    widgets.noninteractive.weak_bg_fill = egui::Color32::TRANSPARENT;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, ink::ink_wash(0.18));
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, ink::INK);
    widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    widgets.inactive.bg_stroke = Stroke::new(1.0, ink::INK);
    widgets.inactive.fg_stroke = Stroke::new(1.0, ink::INK);
    widgets.hovered.bg_fill = ink::gold_wash(0.2);
    widgets.hovered.weak_bg_fill = ink::gold_wash(0.2);
    widgets.hovered.bg_stroke = Stroke::new(1.0, ink::INK);
    widgets.hovered.fg_stroke = Stroke::new(1.0, ink::INK);
    widgets.active.bg_fill = ink::gold_wash(0.28);
    widgets.active.weak_bg_fill = ink::gold_wash(0.28);
    widgets.active.bg_stroke = Stroke::new(1.0, ink::INK);
    widgets.active.fg_stroke = Stroke::new(1.0, ink::INK);
    widgets.open.bg_fill = ink::LIGHT;
    widgets.open.weak_bg_fill = ink::LIGHT;
    widgets.open.bg_stroke = Stroke::new(1.0, ink::INK);
    widgets.open.fg_stroke = Stroke::new(1.0, ink::INK);
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(controls::RADIUS as u8);
        widget.expansion = 0.0;
    }
}

pub(crate) fn popup_style(style: &mut Style) {
    let widgets = &mut style.visuals.widgets;
    for widget in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
    ] {
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
    }
}
