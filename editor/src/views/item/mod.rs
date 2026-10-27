//! The item view: the item as the player sees it, in a slot and on its card, its look lying on
//! the stage as a drop, how a held item is wielded, and whether its pictures are the sizes the game
//! draws.

pub mod wield;

use std::collections::HashMap;
use std::sync::Arc;

use engine::core::content::ContentFile;
use engine::core::content::media::GltfInfos;
use engine::core::gui::ImageSource;
use engine::systems::area::{Picture, Pictured};
use engine::systems::item::card::{self as item_card, CardLook};
use engine::systems::item::{self, CARD_SIZE, ICON_SIZE, ItemDef, Wear, widget};
use ui::egui::{self, Sense, Ui, Vec2};
use ui::{card, chip, form, panel, sheet, theme};

use super::area::camera;
use super::stage::{self, StagePicture};
use crate::app::Editor;
use crate::shell::row_form::usages;
use crate::shell::{row_form, sidebar};
use crate::tabs::shown_path;
use crate::thumbs::ProjectImages;

#[derive(Default)]
pub struct ItemViews {
    views: HashMap<String, ItemView>,
}

pub struct ItemView {
    pub page: ItemPage,
    pub camera: camera::OrbitCamera,
    pub spinning: bool,
    picture: StagePicture,
    pub wield: wield::WieldView,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemPage {
    #[default]
    Card,
    Look,
    Wield,
}

impl ItemPage {
    fn label(self) -> &'static str {
        match self {
            ItemPage::Card => "card",
            ItemPage::Look => "look",
            ItemPage::Wield => "wield",
        }
    }
}

/// A picture the item names, with the size the game draws it at and the size it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureCheck {
    pub field: &'static str,
    pub path: String,
    pub wanted: (u32, u32),
    pub size: Option<(u32, u32)>,
}

impl PictureCheck {
    pub fn fits(&self) -> bool {
        self.size == Some(self.wanted)
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, key: &str) {
    let Some(row) = super::read_shown::<ItemDef>(editor, ui, key) else {
        return;
    };
    ensure_view(editor, key);
    sidebar::row(ui, "item", |ui| {
        row_form::show_row(editor, ui, ItemDef::TABLE, key)
    });
    sidebar::inspector(editor, ui, "item", |editor, ui| {
        inspect(editor, ui, key, &row);
    });
    let held = matches!(row.wear(), Some(Wear::Held { .. }));
    let pages: &[ItemPage] = if held {
        &[ItemPage::Card, ItemPage::Look, ItemPage::Wield]
    } else {
        &[ItemPage::Card, ItemPage::Look]
    };
    let page = Some(editor.views.item.view_mut(key).page)
        .filter(|page| pages.contains(page))
        .unwrap_or_default();
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(theme::mono(shown_path(key)).color(theme::CODE));
                ui.add_space(8.0);
                for other in pages {
                    if chip::mode(ui, other.label(), page == *other).clicked() {
                        editor.views.item.view_mut(key).page = *other;
                    }
                }
                ui.add_space(8.0);
                ui.label(theme::faint(item_card::kind(&row).word()));
                if page != ItemPage::Card {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let view = editor.views.item.view_mut(key);
                        let spinning = match page {
                            ItemPage::Wield => &mut view.wield.stage.spinning,
                            _ => &mut view.spinning,
                        };
                        if chip::toggle(ui, "turning", *spinning).clicked() {
                            *spinning = !*spinning;
                        }
                    });
                }
            });
            ui.add_space(4.0);
            match page {
                ItemPage::Card => {
                    egui::ScrollArea::vertical()
                        .id_salt(("item", key))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            sheet::section(ui, "as the player sees it");
                            shown(editor, ui, key, &row);
                        });
                }
                ItemPage::Look => {
                    sheet::section(ui, "dropped on the ground");
                    let size = Vec2::new(ui.available_width(), ui.available_height().max(200.0));
                    drop_strip(editor, ui, key, &row, size);
                }
                ItemPage::Wield => wield::show(editor, ui, key, &row),
            }
        });
    editor.status.left.push(format!(
        "{} · {} · looks like {}",
        shown_path(key),
        item_card::kind(&row).word(),
        shown_path(&look_path(editor, &row).unwrap_or_default())
    ));
}

/// What the game draws of the item: its icon in a slot and its card.
fn shown(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ItemDef) {
    let ctx = ui.ctx().clone();
    let Some(content) = editor.loaded() else {
        form::hint(ui, "The card shows once the content loads.");
        return;
    };
    let Some(item) = content.by_path::<ItemDef>(key) else {
        return;
    };
    let Editor { project, form, .. } = &mut *editor;
    let mut images = ProjectImages {
        ctx: &ctx,
        files: &project.as_ref().expect("a project").files,
        thumbnails: &mut form.thumbnails,
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            widget::shown_slot(ui, &mut images, &row.icon);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let art = images.image(&row.icon).map(|image| image.texture);
            let look = CardLook {
                width: ui.available_width().min(card::WIDTH),
                closable: false,
                ..CardLook::PINNED
            };
            item::card::face_of(row, art, look).show(ui, |ui, plate| {
                item::card::summary(ui, &content, item, plate);
                item::card::details(ui, &content, item, |_| None);
                item::card::grant_lines(ui, &mut images, &content, item);
            });
        });
    });
}

/// The item's look on the stage, as it lies when dropped.
fn drop_strip(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ItemDef, size: Vec2) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    let Some(picture) = picture(editor, key, row) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "pick a prop for the look to see it dropped",
            egui::FontId::proportional(13.0),
            theme::MUTED,
        );
        return;
    };
    let view_key = format!("item:{key}");
    stage::show_picture(editor, ui, &painter, rect, &view_key);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let view = editor.views.item.view_mut(key);
    let busy = camera::navigate(ui, &response, &mut view.camera, rect.height());
    if busy && response.dragged_by(egui::PointerButton::Secondary) {
        view.spinning = false;
    }
    if view.spinning {
        stage::spin(&mut view.camera, dt);
        ui.ctx().request_repaint();
    }
    let camera = view.camera;
    stage::request(editor, ui, &view_key, rect, camera, picture, Vec::new());
}

fn inspect(editor: &mut Editor, ui: &mut Ui, key: &str, row: &ItemDef) {
    let ctx = ui.ctx().clone();
    if editor.views.item.view_mut(key).page == ItemPage::Wield
        && matches!(row.wear(), Some(Wear::Held { .. }))
    {
        wield::inspect(editor, ui, key);
        ui.add_space(8.0);
    }
    let checks = pictures(editor, key);
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Pictures", "", |_| {});
        ui.add_space(4.0);
        for check in &checks {
            let thumb = editor.thumb(&ctx, &check.path);
            form::kv(ui, check.field, |ui| {
                ui.vertical(|ui| {
                    if let Some(thumb) = thumb {
                        let (width, height) = check.size.unwrap_or(check.wanted);
                        let scale = (ui.available_width() / width.max(1) as f32).min(2.0);
                        ui.image(egui::load::SizedTexture::new(
                            thumb.texture.id(),
                            Vec2::new(width as f32 * scale, height as f32 * scale),
                        ));
                    }
                    let (want_w, want_h) = check.wanted;
                    match check.size {
                        Some((width, height)) if check.fits() => {
                            ui.colored_label(theme::OK, format!("{width} × {height}"));
                        }
                        Some((width, height)) => {
                            ui.colored_label(
                                theme::DANGER,
                                format!("{width} × {height} · the game draws {want_w} × {want_h}"),
                            );
                        }
                        None => {
                            ui.colored_label(theme::DANGER, "does not read as a picture");
                        }
                    }
                    ui.label(theme::muted(&check.path).size(12.0));
                });
            });
        }
    });
    ui.add_space(8.0);
    panel::card(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        panel::header(ui, "Look", "", |_| {});
        ui.add_space(4.0);
        match look_path(editor, row) {
            Some(look) => {
                ui.horizontal(|ui| {
                    let side = Vec2::splat(64.0);
                    match editor.form.prop_thumbnails.thumbnail(&look) {
                        Some(texture) => {
                            ui.image(egui::load::SizedTexture::new(texture, side));
                        }
                        None => {
                            let (rect, _) = ui.allocate_exact_size(side, Sense::hover());
                            ui.painter().rect_filled(rect, 3.0, theme::INPUT);
                            ui.ctx().request_repaint();
                        }
                    }
                    ui.label(theme::mono(shown_path(&look)).size(13.0));
                });
            }
            None => {
                ui.colored_label(theme::DANGER, "the look names no prop that loads");
            }
        }
    });
    ui.add_space(8.0);
    usages::show_usages_grouped(editor, ui, key, true);
}

/// The item's icon and card against the sizes the game draws them at.
pub fn pictures(editor: &mut Editor, key: &str) -> Vec<PictureCheck> {
    let Some(Ok(row)) = editor.read::<ItemDef>(key) else {
        return Vec::new();
    };
    let Editor { project, form, .. } = &mut *editor;
    let Some(project) = project.as_ref() else {
        return Vec::new();
    };
    let files = &project.files;
    [
        ("icon", row.icon.as_str(), ICON_SIZE),
        ("card", row.card.as_str(), CARD_SIZE),
    ]
    .into_iter()
    .map(|(field, path, wanted)| PictureCheck {
        field,
        path: path.to_owned(),
        wanted,
        size: form
            .thumbnails
            .dimensions(files, path)
            .map(|[width, height]| (width, height)),
    })
    .collect()
}

pub fn ensure_view(editor: &mut Editor, key: &str) {
    if editor.views.item.views.contains_key(key) {
        return;
    }
    let radius = editor
        .read::<ItemDef>(key)
        .and_then(Result::ok)
        .and_then(|row| look_path(editor, &row))
        .and_then(|look| {
            let project = editor.project.as_ref()?;
            let content = project.content.as_ref()?;
            let prop = content.by_path::<engine::systems::area::prop::PropDef>(&look)?;
            let model = prop.get(content).model.path();
            let info = content.built::<GltfInfos>().0.get(&model)?;
            Some((info.bounds.max - info.bounds.min).length() / 2.0)
        })
        .unwrap_or(0.5);
    editor.views.item.views.insert(
        key.to_owned(),
        ItemView {
            page: ItemPage::default(),
            camera: stage::framing(radius.max(0.4)),
            spinning: true,
            picture: StagePicture::default(),
            wield: wield::WieldView::default(),
        },
    );
}

impl ItemViews {
    pub fn view_mut(&mut self, key: &str) -> &mut ItemView {
        self.views
            .get_mut(key)
            .expect("ensure_view ran before the item view was changed")
    }
}

/// The path of the prop the item looks like, when it loads.
fn look_path(editor: &Editor, row: &ItemDef) -> Option<String> {
    let reader = editor.reader()?;
    Some(reader.path_of(row.look)?.to_owned())
}

fn picture(editor: &mut Editor, key: &str, row: &ItemDef) -> Option<(u64, Arc<Picture>)> {
    let project = editor.project.as_ref()?;
    let content = project.content.clone()?;
    let generation = project.generation;
    let look = look_path(editor, row)?;
    let prop = content.by_path::<engine::systems::area::prop::PropDef>(&look)?;
    ensure_view(editor, key);
    editor
        .views
        .item
        .view_mut(key)
        .picture
        .of(&content, generation, Pictured::Prop(prop))
}
