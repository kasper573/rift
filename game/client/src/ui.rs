//! The HUD's widget/window system, built on egui. A widget is a draggable on-screen control; a
//! widget may pair with a window it toggles (the widget hides while its window is open, and vice
//! versa). Both snap to a grid and persist their geometry through [`UserSettings`]. The player
//! healthbar rides along as a non-interactive egui overlay.
//!
//! Adding a panel is a method that draws its widget (and optionally calls [`Hud::window`]); the
//! shared helpers below carry the dragging, snapping, persistence, and chrome.

use std::collections::HashMap;

use egui_macroquad::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TextureHandle, Vec2, pos2, vec2,
};
use macroquad::input::{KeyCode, is_key_pressed};
use macroquad::prelude::ImageFormat;
use macroquad::texture::Image;
use world::math::{Offset, Pixels, Pos};
use world::{ItemId, MmoClient};

use crate::render::{self, Screen};
use crate::user_settings::{Placement, UserSettings};

const WIDGET: f32 = 48.0;
const SLOT: f32 = 36.0;
const MIN_SIZE: Vec2 = vec2(100.0, 100.0);
const TITLE_H: f32 = 22.0;

const BAR: Size = Size { w: 20.0, h: 4.0 };
const BAR_BORDER: Color32 = Color32::from_rgb(0x14, 0x0A, 0x28);
const BAR_BG: Color32 = Color32::from_rgb(0x2A, 0x1C, 0x5C);
const BAR_FILL: Color32 = Color32::from_rgb(0x00, 0xFF, 0x00);

pub struct Hud {
    pub settings: UserSettings,
    pub pointer_captured: bool,
    opened: HashMap<&'static str, bool>,
    geom: HashMap<&'static str, Geom>,
    textures: Option<Textures>,
}

impl Hud {
    pub fn new() -> Hud {
        Hud {
            settings: UserSettings::load(),
            pointer_captured: false,
            opened: HashMap::new(),
            geom: HashMap::new(),
            textures: None,
        }
    }

    pub fn frame(&mut self, ctx: &egui::Context, client: &mut MmoClient, screen: Screen) {
        self.ensure(ctx);
        if client.my_position().is_none() {
            self.pointer_captured = false;
            return;
        }
        self.toggles();
        self.healthbar(ctx, client, screen);
        self.character(ctx, client);
        self.inventory(ctx, client);
        self.settings_panel(ctx);
        self.pointer_captured = ctx.wants_pointer_input() || ctx.is_pointer_over_area();
    }

    fn toggles(&mut self) {
        if is_key_pressed(KeyCode::I) {
            self.flip("inventory");
        }
        if is_key_pressed(KeyCode::O) {
            self.flip("settings");
        }
    }

    fn healthbar(&self, ctx: &egui::Context, client: &mut MmoClient, screen: Screen) {
        let Some(camera) = render::camera(client) else {
            return;
        };
        let Some(pos) = client.my_position() else {
            return;
        };
        let Some((health, max)) = client.my_vitals() else {
            return;
        };
        if health <= 0.0 || max <= 0.0 {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let scale = screen.scale / ppp;
        let point = |frame: Pos<Pixels>| {
            pos2(
                (frame.x * screen.scale + screen.offset.x) / ppp,
                (frame.y * screen.scale + screen.offset.y) / ppp,
            )
        };
        let top_left = render::to_frame_f(camera, pos).round() + Offset::new(-BAR.w / 2.0, 3.0);
        let inner = top_left + Offset::splat(1.0);
        let fill_w = ((BAR.w - 2.0) * (health / max).clamp(0.0, 1.0)).floor() * scale;

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("rift.healthbar"),
        ));
        painter.rect_filled(
            Rect::from_min_size(point(top_left), vec2(BAR.w * scale, BAR.h * scale)),
            CornerRadius::ZERO,
            BAR_BORDER,
        );
        painter.rect_filled(
            Rect::from_min_size(
                point(inner),
                vec2((BAR.w - 2.0) * scale, (BAR.h - 2.0) * scale),
            ),
            CornerRadius::ZERO,
            BAR_BG,
        );
        painter.rect_filled(
            Rect::from_min_size(point(inner), vec2(fill_w, (BAR.h - 2.0) * scale)),
            CornerRadius::ZERO,
            BAR_FILL,
        );
    }

    fn character(&mut self, ctx: &egui::Context, client: &mut MmoClient) {
        let name = client.my_name().unwrap_or_default();
        let (health, max) = client.my_vitals().unwrap_or((0.0, 0.0));
        let xp = client.my_xp().unwrap_or(0);
        let lines = [name, format!("{health:.0} / {max:.0}"), format!("xp {xp}")];
        let screen = ctx.screen_rect();
        let default_pos = pos2(screen.min.x + 8.0, screen.min.y + 8.0);
        let base = self.placed("character", default_pos);
        let pos = snap_pos(&self.settings, base);
        let resp = egui::Area::new(egui::Id::new("rift.widget.character"))
            .order(egui::Order::Middle)
            .current_pos(pos)
            .movable(false)
            .constrain(true)
            .show(ctx, |ui| text_box(ui, &lines, 16.0))
            .inner;
        self.dragged("character", &resp);
    }

    fn inventory(&mut self, ctx: &egui::Context, client: &mut MmoClient) {
        if self.is_open("inventory") {
            let items = client.my_inventory();
            let close = self.window(
                ctx,
                "inventory.window",
                "Inventory",
                vec2(400.0, 200.0),
                |ui, tex| {
                    inventory_grid(ui, &items, tex, client);
                },
            );
            if close {
                self.set_open("inventory", false);
            }
            return;
        }
        let screen = ctx.screen_rect();
        let default_pos = pos2(screen.max.x - 8.0 - WIDGET, screen.min.y + 8.0);
        let base = self.placed("inventory", default_pos);
        let pos = snap_pos(&self.settings, base);
        let icon = self.tex().inventory.clone();
        let resp = egui::Area::new(egui::Id::new("rift.widget.inventory"))
            .order(egui::Order::Middle)
            .current_pos(pos)
            .movable(false)
            .constrain(true)
            .show(ctx, |ui| icon_widget(ui, &icon, 8.0, "I", "Inventory"))
            .inner;
        self.dragged("inventory", &resp);
        if resp.clicked() {
            self.set_open("inventory", true);
        }
    }

    fn settings_panel(&mut self, ctx: &egui::Context) {
        if self.is_open("settings") {
            let enabled = self.settings.snapping_enabled();
            let mut toggle = false;
            let close = self.window(
                ctx,
                "settings.window",
                "Settings",
                vec2(400.0, 200.0),
                |ui, _tex| {
                    let label = if enabled {
                        "ui snapping enabled"
                    } else {
                        "ui snapping disabled"
                    };
                    if ui.button(label).clicked() {
                        toggle = true;
                    }
                },
            );
            if toggle {
                self.settings.toggle_snapping();
                self.settings.save();
            }
            if close {
                self.set_open("settings", false);
            }
            return;
        }
        let screen = ctx.screen_rect();
        let default_pos = pos2(
            screen.max.x - 8.0 - WIDGET - 8.0 - WIDGET,
            screen.min.y + 8.0,
        );
        let base = self.placed("settings", default_pos);
        let pos = snap_pos(&self.settings, base);
        let icon = self.tex().settings.clone();
        let resp = egui::Area::new(egui::Id::new("rift.widget.settings"))
            .order(egui::Order::Middle)
            .current_pos(pos)
            .movable(false)
            .constrain(true)
            .show(ctx, |ui| icon_widget(ui, &icon, 8.0, "O", "Settings"))
            .inner;
        self.dragged("settings", &resp);
        if resp.clicked() {
            self.set_open("settings", true);
        }
    }

    /// Draws a titled, draggable, resizable window with `content` as its body; returns whether its
    /// close button was clicked this frame. Position and size are controlled by us (snapped and
    /// persisted), not by egui's window state.
    fn window(
        &mut self,
        ctx: &egui::Context,
        key: &'static str,
        title: &str,
        default_size: Vec2,
        content: impl FnOnce(&mut egui::Ui, &Textures),
    ) -> bool {
        let screen = ctx.screen_rect();
        let default_pos = pos2(
            screen.center().x - default_size.x / 2.0,
            screen.center().y - default_size.y / 2.0,
        );
        let geom = self.geom(key, default_pos, default_size);
        let pos = snap_pos(&self.settings, geom.pos);
        let size = snap_vec(&self.settings, geom.size.max(MIN_SIZE));
        let id = egui::Id::new(("rift.window", key));

        let (chrome, ()) = {
            let tex = self.tex();
            let resize_icon = tex.resize.clone();
            egui::Area::new(id)
                .order(egui::Order::Foreground)
                .current_pos(pos)
                .movable(false)
                .constrain(true)
                .show(ctx, |ui| {
                    window_chrome(ui, id, size, title, &resize_icon, |ui| content(ui, tex))
                })
                .inner
        };

        let geom = self.geom.get_mut(key).expect("seeded");
        geom.pos += chrome.move_delta;
        geom.size = (geom.size + chrome.size_delta).max(MIN_SIZE);
        if chrome.drag_ended {
            let pos = snap_pos(&self.settings, self.geom[key].pos);
            let size = snap_vec(&self.settings, self.geom[key].size.max(MIN_SIZE));
            self.settings.set_placement(
                key,
                Placement {
                    pos: (pos.x, pos.y),
                    size: Some((size.x, size.y)),
                },
            );
            self.settings.save();
        }
        chrome.close
    }

    fn dragged(&mut self, key: &'static str, resp: &Response) {
        if resp.dragged() {
            self.geom.get_mut(key).expect("seeded").pos += resp.drag_delta();
        }
        if resp.drag_stopped() {
            let pos = snap_pos(&self.settings, self.geom[key].pos);
            self.settings.set_placement(
                key,
                Placement {
                    pos: (pos.x, pos.y),
                    size: None,
                },
            );
            self.settings.save();
        }
    }

    fn placed(&mut self, key: &'static str, default_pos: Pos2) -> Pos2 {
        self.geom(key, default_pos, Vec2::ZERO).pos
    }

    fn geom(&mut self, key: &'static str, default_pos: Pos2, default_size: Vec2) -> Geom {
        if !self.geom.contains_key(key) {
            let stored = self.settings.placement(key);
            let pos = stored.map_or(default_pos, |p| pos2(p.pos.0, p.pos.1));
            let size = stored
                .and_then(|p| p.size)
                .map_or(default_size, |s| vec2(s.0, s.1));
            self.geom.insert(key, Geom { pos, size });
        }
        self.geom[key]
    }

    fn ensure(&mut self, ctx: &egui::Context) {
        if self.textures.is_some() {
            return;
        }
        apply_style(ctx);
        let items = world::items::items()
            .iter()
            .map(|item| load_texture(ctx, &item.id, item.icon.0))
            .collect();
        self.textures = Some(Textures {
            items,
            inventory: load_texture(
                ctx,
                "rift.ui.inventory",
                asset("icons/potion/red_potion.png"),
            ),
            settings: load_texture(
                ctx,
                "rift.ui.settings",
                asset("icons/weapon_and_tool/iron_sword.png"),
            ),
            resize: load_texture(ctx, "rift.ui.resize", asset("icons/cursors/pointer010.png")),
        });
    }

    fn tex(&self) -> &Textures {
        self.textures.as_ref().expect("textures loaded")
    }

    fn is_open(&self, key: &'static str) -> bool {
        self.opened.get(key).copied().unwrap_or(false)
    }

    fn set_open(&mut self, key: &'static str, open: bool) {
        self.opened.insert(key, open);
    }

    fn flip(&mut self, key: &'static str) {
        let open = self.is_open(key);
        self.set_open(key, !open);
    }
}

#[derive(Clone, Copy)]
struct Geom {
    pos: Pos2,
    size: Vec2,
}

struct Textures {
    items: Vec<TextureHandle>,
    inventory: TextureHandle,
    settings: TextureHandle,
    resize: TextureHandle,
}

struct Chrome {
    move_delta: Vec2,
    size_delta: Vec2,
    drag_ended: bool,
    close: bool,
}

struct Size {
    w: f32,
    h: f32,
}

fn window_chrome<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    size: Vec2,
    title: &str,
    resize_icon: &TextureHandle,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (Chrome, R) {
    let origin = ui.min_rect().min;
    ui.set_min_size(size);
    let full = Rect::from_min_size(origin, size);
    let painter = ui.painter().clone();
    painter.rect_filled(full, CornerRadius::ZERO, Color32::BLACK);
    painter.rect_stroke(
        full,
        CornerRadius::ZERO,
        Stroke::new(1.0, Color32::WHITE),
        StrokeKind::Inside,
    );

    let title_bar = Rect::from_min_size(origin, vec2(size.x, TITLE_H));
    let close = Rect::from_min_size(
        pos2(title_bar.right() - TITLE_H, origin.y),
        vec2(TITLE_H, TITLE_H),
    );
    let handle = Rect::from_min_max(origin, pos2(close.left(), title_bar.bottom()));
    let drag_resp = ui.interact(handle, id.with("drag"), Sense::click_and_drag());
    let close_resp = ui.interact(close, id.with("close"), Sense::click());
    painter.text(
        pos2(origin.x + 6.0, title_bar.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.0),
        Color32::WHITE,
    );
    painter.text(
        close.center(),
        Align2::CENTER_CENTER,
        "x",
        FontId::proportional(14.0),
        if close_resp.hovered() {
            Color32::WHITE
        } else {
            Color32::from_gray(150)
        },
    );
    painter.line_segment(
        [
            pos2(full.left(), title_bar.bottom()),
            pos2(full.right(), title_bar.bottom()),
        ],
        Stroke::new(1.0, Color32::from_gray(70)),
    );

    ui.allocate_exact_size(vec2(size.x, TITLE_H), Sense::hover());
    let inner = content(ui);

    let resize = Rect::from_min_size(full.max - vec2(16.0, 16.0), vec2(16.0, 16.0));
    let resize_resp = ui.interact(resize, id.with("resize"), Sense::drag());
    painter.image(resize_icon.id(), resize, uv(), Color32::WHITE);

    let chrome = Chrome {
        move_delta: drag_resp.drag_delta(),
        size_delta: resize_resp.drag_delta(),
        drag_ended: drag_resp.drag_stopped() || resize_resp.drag_stopped(),
        close: close_resp.clicked(),
    };
    (chrome, inner)
}

fn inventory_grid(ui: &mut egui::Ui, items: &[ItemId], tex: &Textures, client: &mut MmoClient) {
    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal_wrapped(|ui| {
                for (slot, item) in items.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(SLOT, SLOT), Sense::click());
                    let inner = Rect::from_min_size(rect.min, vec2(SLOT - 2.0, SLOT - 2.0));
                    let painter = ui.painter();
                    painter.rect_filled(inner, CornerRadius::ZERO, Color32::from_black_alpha(160));
                    painter.rect_stroke(
                        inner,
                        CornerRadius::ZERO,
                        Stroke::new(
                            2.0,
                            if resp.hovered() {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                        StrokeKind::Inside,
                    );
                    let icon = &tex.items[item.0 as usize];
                    painter.image(
                        icon.id(),
                        Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(32.0, 32.0)),
                        uv(),
                        Color32::WHITE,
                    );
                    let resp = resp.on_hover_text(world::items::item(*item).display_name.as_str());
                    if resp.double_clicked() {
                        client.use_item(slot as u32);
                    }
                }
            });
        });
}

fn icon_widget(
    ui: &mut egui::Ui,
    icon: &TextureHandle,
    pad: f32,
    badge: &str,
    title: &str,
) -> Response {
    let dim = 32.0 + 2.0 * pad;
    let (rect, resp) = ui.allocate_exact_size(vec2(dim, dim), Sense::click_and_drag());
    let fill = if resp.is_pointer_button_down_on() {
        Color32::from_gray(70)
    } else if resp.hovered() {
        Color32::from_gray(40)
    } else {
        Color32::BLACK
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    painter.image(
        icon.id(),
        Rect::from_min_size(rect.min + vec2(pad, pad), vec2(32.0, 32.0)),
        uv(),
        Color32::WHITE,
    );
    let galley = painter.layout_no_wrap(badge.to_owned(), FontId::monospace(11.0), Color32::WHITE);
    let badge_rect = Rect::from_min_size(
        pos2(rect.right() - galley.size().x - 4.0, rect.top()),
        galley.size() + vec2(4.0, 2.0),
    );
    painter.rect_filled(badge_rect, CornerRadius::ZERO, Color32::BLACK);
    painter.galley(badge_rect.min + vec2(2.0, 1.0), galley, Color32::WHITE);
    resp.on_hover_text(title)
}

fn text_box(ui: &mut egui::Ui, lines: &[String], pad: f32) -> Response {
    let painter = ui.painter().clone();
    let font = FontId::proportional(14.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|line| painter.layout_no_wrap(line.clone(), font.clone(), Color32::WHITE))
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0_f32, f32::max);
    let height: f32 = galleys.iter().map(|g| g.size().y).sum();
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width + 2.0 * pad, height + 2.0 * pad),
        Sense::click_and_drag(),
    );
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::BLACK);
    let mut y = rect.min.y + pad;
    for galley in galleys {
        let advance = galley.size().y;
        painter.galley(pos2(rect.min.x + pad, y), galley, Color32::WHITE);
        y += advance;
    }
    resp
}

fn apply_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::WHITE);
    visuals.window_fill = Color32::BLACK;
    visuals.panel_fill = Color32::BLACK;
    visuals.window_stroke = Stroke::new(1.0, Color32::WHITE);
    paint_widget(&mut visuals.widgets.noninteractive, Color32::BLACK);
    paint_widget(&mut visuals.widgets.inactive, Color32::BLACK);
    paint_widget(&mut visuals.widgets.hovered, Color32::from_gray(40));
    paint_widget(&mut visuals.widgets.active, Color32::from_gray(70));
    paint_widget(&mut visuals.widgets.open, Color32::from_gray(40));
    ctx.set_visuals(visuals);
}

fn paint_widget(widget: &mut egui::style::WidgetVisuals, fill: Color32) {
    widget.bg_fill = fill;
    widget.weak_bg_fill = fill;
    widget.bg_stroke = Stroke::new(1.0, Color32::from_gray(80));
    widget.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    widget.corner_radius = CornerRadius::ZERO;
}

fn load_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> TextureHandle {
    let image = Image::from_file_with_format(png, Some(ImageFormat::Png)).expect("decode ui png");
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width as usize, image.height as usize],
        &image.bytes,
    );
    ctx.load_texture(name, color, egui::TextureOptions::NEAREST)
}

fn snap_pos(settings: &UserSettings, pos: Pos2) -> Pos2 {
    pos2(settings.snap(pos.x), settings.snap(pos.y))
}

fn snap_vec(settings: &UserSettings, size: Vec2) -> Vec2 {
    vec2(settings.snap(size.x), settings.snap(size.y))
}

fn asset(path: &str) -> &'static [u8] {
    world::assets::bytes(path).unwrap_or_else(|| panic!("missing asset {path}"))
}

fn uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}
