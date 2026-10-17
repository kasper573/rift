//! The picker field: how every content relation is assigned. A field holds one [`Pick`] and
//! draws it as a chip; clicking the chip opens a popover that lists what a [`PickSource`] offers,
//! filtered by free text and by the source's category chips, and dropping something dragged
//! from a browser onto the chip assigns it when the source accepts it.

pub mod assets;
pub mod rows;

use egui::{Align, Color32, Layout, RichText, Sense, Stroke, Ui, Vec2};

pub use self::assets::AssetSource;
pub use self::rows::{PictureFrame, RowSource, picture_of};
use crate::drafts::{clear_draft, draft_of, set_draft};
use crate::forms::{FormAction, FormCx};
use crate::ui::assets::AssetKind;
use crate::ui::{theme, widgets};

/// What a picker field holds: a key within a category (a row's key within its table, a file's
/// path within its kind).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    pub category: String,
    pub key: String,
}

impl Pick {
    pub fn new(category: impl Into<String>, key: impl Into<String>) -> Pick {
        Pick {
            category: category.into(),
            key: key.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }
}

/// Something carried out of a browser by the pointer; a picker field takes it when its source
/// accepts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dragged {
    Row { table: String, key: String },
    Asset(String),
}

impl Dragged {
    pub fn label(&self) -> String {
        match self {
            Dragged::Row { table, key } => format!("{table}/{key}"),
            Dragged::Asset(path) => format!("assets/{path}"),
        }
    }
}

/// Makes a widget the source of a drag: the payload is set when the drag starts and a chip
/// naming it follows the pointer until it's dropped.
pub fn carry(ui: &Ui, response: &egui::Response, dragged: &Dragged) {
    response.dnd_set_drag_payload(dragged.clone());
    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Tooltip, response.id));
        widgets::floating_chip(
            &painter,
            pointer + egui::vec2(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            &dragged.label(),
            theme::RAISED,
            theme::TEXT,
            12.5,
        );
    }
}

/// A chip in the popover that narrows the list to one category.
pub struct Category {
    pub id: String,
    pub label: String,
}

/// One thing a source offers.
pub struct Candidate {
    pub pick: Pick,
    /// The category chip that keeps this candidate listed.
    pub category: String,
    /// A secondary name shown beside the key, when the key alone says little.
    pub name: String,
    /// The line under the key.
    pub hint: String,
    /// Why this candidate can't be picked here, when it can't.
    pub blocked: Option<String>,
}

/// What a picker field lists, how it shows a pick, and what it takes by drop.
pub trait PickSource {
    fn placeholder(&self) -> String;
    /// The chips offered for quick filtering; none hides the chip row.
    fn categories(&self, cx: &FormCx) -> Vec<Category>;
    fn candidates(&self, cx: &FormCx) -> Vec<Candidate>;
    /// The pick a dragged thing becomes on this field, or why the field refuses it.
    fn accepts(&self, cx: &FormCx, dragged: &Dragged) -> Result<Pick, String>;
    /// The muted prefix and the text a pick is shown as.
    fn shown(&self, pick: &Pick) -> (String, String);
    fn thumbnail(&self, ui: &mut Ui, cx: &mut FormCx, pick: &Pick, size: f32);
    fn footer(&self, cx: &FormCx, pick: &Pick) -> String;
    /// Lays the candidates out as a grid of pictures instead of a list.
    fn grid(&self) -> bool {
        false
    }
    /// The action behind the arrow button beside a held pick.
    fn opens(&self, _pick: &Pick) -> Option<FormAction> {
        None
    }
    /// A "New …" line at the popover's foot that creates and picks in one step.
    fn creates(&self, _cx: &FormCx) -> Option<(String, FormAction)> {
        None
    }
}

pub fn row(ui: &mut Ui, cx: &mut FormCx, table: &str, key: &mut String, path: &str) -> bool {
    let mut pick = Pick::new(table, key.as_str());
    let changed = field(ui, cx, &RowSource::table(table, path), &mut pick, path);
    if changed {
        *key = pick.key;
    }
    changed
}

/// A field over several tables at once; the pick names which table its key came from.
pub fn row_among(
    ui: &mut Ui,
    cx: &mut FormCx,
    tables: &[&str],
    table: &mut String,
    key: &mut String,
    path: &str,
) -> bool {
    let mut pick = Pick::new(table.as_str(), key.as_str());
    let changed = field(ui, cx, &RowSource::tables(tables, path), &mut pick, path);
    if changed {
        *table = pick.category;
        *key = pick.key;
    }
    changed
}

pub fn asset_of(
    ui: &mut Ui,
    cx: &mut FormCx,
    kinds: &[AssetKind],
    file: &mut String,
    path: &str,
) -> bool {
    let mut pick = Pick::new(
        assets::kind_of(file).map(AssetKind::id).unwrap_or_default(),
        file.as_str(),
    );
    let changed = field(ui, cx, &AssetSource::of(kinds), &mut pick, path);
    if changed {
        *file = pick.key;
    }
    changed
}

/// The field itself: the chip, the arrow button, the drop target and the popover.
pub fn field(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    pick: &mut Pick,
    path: &str,
) -> bool {
    let id = ui.id().with(path).with("picker");
    let mut changed = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if !pick.is_empty()
            && let Some(action) = source.opens(pick)
            && ui
                .add(
                    egui::Button::new(RichText::new("→").color(Color32::WHITE))
                        .fill(theme::ACCENT)
                        .min_size(Vec2::new(26.0, ui.spacing().interact_size.y)),
                )
                .on_hover_text(format!("Open {}", source.shown(pick).1))
                .clicked()
        {
            cx.actions.push(action);
        }
        let response = chip(ui, cx, source, pick);
        if let Some(dragged) = response.dnd_hover_payload::<Dragged>() {
            let verdict = source.accepts(cx, &dragged);
            let (ink, text) = match &verdict {
                Ok(dropped) => (
                    theme::ACCENT,
                    format!("drop to pick {}", source.shown(dropped).1),
                ),
                Err(reason) => (theme::DANGER, reason.clone()),
            };
            ui.painter().rect_stroke(
                response.rect,
                4.0,
                Stroke::new(2.0, ink),
                egui::StrokeKind::Outside,
            );
            if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                let painter = ui
                    .ctx()
                    .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("verdict")));
                widgets::floating_chip(
                    &painter,
                    pointer + egui::vec2(12.0, 34.0),
                    egui::Align2::LEFT_TOP,
                    &text,
                    theme::RAISED,
                    ink,
                    12.0,
                );
            }
        }
        if let Some(dragged) = response.dnd_release_payload::<Dragged>() {
            match source.accepts(cx, &dragged) {
                Ok(dropped) => {
                    if dropped != *pick {
                        *pick = dropped;
                        changed = true;
                    }
                }
                Err(reason) => cx.state.status = vec![reason],
            }
        }
        let popup = egui::Popup::from_toggle_button_response(&response)
            .id(id)
            .width(440.0);
        popup.show(|ui| {
            if popover(ui, cx, source, pick, path, id) {
                changed = true;
            }
        });
    });
    changed
}

fn chip(ui: &mut Ui, cx: &mut FormCx, source: &dyn PickSource, pick: &Pick) -> egui::Response {
    let empty = pick.is_empty();
    egui::Frame::new()
        .fill(if empty {
            theme::INPUT
        } else {
            theme::ACCENT_SOFT
        })
        .stroke(Stroke::new(
            1.0,
            if empty { theme::BORDER } else { theme::ACCENT },
        ))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if empty {
                    ui.label(theme::muted(format!("pick {}…", source.placeholder())));
                    return;
                }
                source.thumbnail(ui, cx, pick, 18.0);
                let (prefix, text) = source.shown(pick);
                ui.label(theme::muted(prefix).monospace().size(13.0));
                ui.add(
                    egui::Label::new(theme::mono(text).size(13.0).color(theme::CODE)).truncate(),
                );
            });
        })
        .response
        .interact(Sense::click())
}

fn popover(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    pick: &mut Pick,
    path: &str,
    id: egui::Id,
) -> bool {
    let mut changed = false;
    let field = path.rsplit('/').next().unwrap_or(path);
    cx.state.status = vec![
        field.to_owned(),
        if pick.is_empty() {
            "↑ ↓ ← → move · Enter picks · Esc closes".to_owned()
        } else {
            format!("↑ ↓ ← → move · Enter picks · Esc keeps {}", pick.key)
        },
    ];
    let search_id = id.with("search");
    let category_id = id.with("category");
    let mut search = draft_of(ui, search_id).unwrap_or_default();
    let mut category = draft_of(ui, category_id);
    let needle = search.to_lowercase();
    let categories = source.categories(cx);
    let all = source.candidates(cx);
    let total = all.len();
    let mut blocked_rule: Option<String> = None;
    let mut candidates: Vec<&Candidate> = all
        .iter()
        .filter(|candidate| {
            category
                .as_ref()
                .is_none_or(|category| candidate.category == *category)
        })
        .filter(|candidate| {
            needle.is_empty()
                || candidate.pick.key.to_lowercase().contains(&needle)
                || candidate.name.to_lowercase().contains(&needle)
                || candidate.category.to_lowercase().contains(&needle)
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.blocked
            .is_some()
            .cmp(&b.blocked.is_some())
            .then((a.pick != *pick).cmp(&(b.pick != *pick)))
            .then(a.pick.category.cmp(&b.pick.category))
            .then(a.pick.key.cmp(&b.pick.key))
    });
    if all.iter().any(|candidate| candidate.blocked.is_some()) {
        blocked_rule = Some(format!(
            "{} pickable",
            candidates
                .iter()
                .filter(|candidate| candidate.blocked.is_none())
                .count()
        ));
    }
    let highlight_id = id.with("highlight");
    let mut highlight: usize = ui.ctx().data(|d| d.get_temp(highlight_id)).unwrap_or(0);
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::muted("🔍"));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut search)
                        .frame(egui::Frame::NONE)
                        .hint_text(format!("filter · {}", source.placeholder()))
                        .desired_width(ui.available_width() - 120.0),
                );
                widgets::default_focus(ui, &response);
                if response.changed() {
                    set_draft(ui, search_id, &search.clone());
                    highlight = 0;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let counted = match &blocked_rule {
                        Some(rule) => rule.clone(),
                        None => format!("{} of {total}", candidates.len()),
                    };
                    ui.label(theme::muted(counted).monospace().size(11.5));
                });
            });
        });
    if !categories.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for listed in &categories {
                let on = category.as_deref() == Some(listed.id.as_str());
                let count = all
                    .iter()
                    .filter(|candidate| candidate.category == listed.id)
                    .count();
                if widgets::toggle_chip(ui, &format!("{} · {count}", listed.label), on).clicked() {
                    category = (!on).then(|| listed.id.clone());
                    match &category {
                        Some(picked) => set_draft(ui, category_id, picked),
                        None => clear_draft(ui, category_id),
                    }
                    highlight = 0;
                }
            }
        });
    }
    let (up, down, enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let grid = source.grid();
    let columns = if grid { 5 } else { 1 };
    if down {
        highlight = (highlight + columns).min(candidates.len().saturating_sub(1));
    }
    if up {
        highlight = highlight.saturating_sub(columns);
    }
    if grid {
        let (left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if right {
            highlight = (highlight + 1).min(candidates.len().saturating_sub(1));
        }
        if left {
            highlight = highlight.saturating_sub(1);
        }
    }
    let mut picked: Option<Pick> = None;
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .id_salt(id.with("list"))
        .show(ui, |ui| {
            if grid {
                egui::Grid::new(id.with("grid"))
                    .num_columns(columns)
                    .spacing([4.0, 4.0])
                    .show(ui, |ui| {
                        for (index, candidate) in candidates.iter().enumerate() {
                            if tile(ui, cx, source, candidate, index == highlight, pick) {
                                picked = Some(candidate.pick.clone());
                            }
                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            } else {
                for (index, candidate) in candidates.iter().enumerate() {
                    if line(ui, cx, source, candidate, index == highlight, pick) {
                        picked = Some(candidate.pick.clone());
                    }
                }
            }
            if candidates.is_empty() {
                ui.label(theme::muted("nothing matches"));
            }
        });
    if enter
        && let Some(candidate) = candidates.get(highlight)
        && candidate.blocked.is_none()
    {
        picked = Some(candidate.pick.clone());
    }
    ui.separator();
    if !pick.is_empty() {
        ui.label(theme::muted(source.footer(cx, pick)).monospace().size(11.5));
    }
    if let Some((label, action)) = source.creates(cx) {
        let response = ui
            .horizontal(|ui| {
                ui.label(theme::code("+"));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(label);
                    ui.label(theme::muted("creates the row and picks it, as one step").size(11.5));
                });
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            cx.actions.push(action);
            ui.close();
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(highlight_id, highlight));
    if let Some(chosen) = picked {
        if chosen != *pick {
            *pick = chosen;
            changed = true;
        }
        close(ui, id);
    }
    if escape {
        close(ui, id);
    }
    changed
}

fn line(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    candidate: &Candidate,
    lit: bool,
    held: &Pick,
) -> bool {
    let full = ui.available_rect_before_wrap();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(full.width(), 36.0), Sense::click());
    if lit {
        ui.painter().rect_filled(rect, 4.0, theme::ACCENT_SOFT);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 8));
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(6.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    source.thumbnail(&mut child, cx, &candidate.pick, 24.0);
    let (prefix, text) = source.shown(&candidate.pick);
    child.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let ink = if candidate.blocked.is_some() {
            theme::FAINT
        } else {
            theme::TEXT
        };
        ui.horizontal(|ui| {
            if !prefix.is_empty() {
                ui.label(theme::muted(prefix).monospace().size(13.0));
            }
            ui.label(RichText::new(text).monospace().size(13.0).color(ink));
            if !candidate.name.is_empty() && candidate.name != candidate.pick.key {
                ui.label(theme::muted(&candidate.name).size(12.0));
            }
        });
        let line = match &candidate.blocked {
            Some(reason) => reason.clone(),
            None => {
                let mut line = candidate.hint.clone();
                if candidate.pick == *held {
                    line.push_str(if line.is_empty() {
                        "this one"
                    } else {
                        " · this one"
                    });
                }
                line
            }
        };
        ui.label(theme::muted(line).size(11.5));
    });
    response.clicked() && candidate.blocked.is_none()
}

fn tile(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    candidate: &Candidate,
    lit: bool,
    held: &Pick,
) -> bool {
    let response = egui::Frame::new()
        .fill(if lit {
            theme::ACCENT_SOFT
        } else if candidate.pick == *held {
            theme::RAISED
        } else {
            Color32::TRANSPARENT
        })
        .corner_radius(4.0)
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.set_width(72.0);
            ui.vertical_centered(|ui| {
                ui.add_enabled_ui(candidate.blocked.is_none(), |ui| {
                    source.thumbnail(ui, cx, &candidate.pick, 48.0);
                    ui.label(
                        RichText::new(&candidate.pick.key)
                            .monospace()
                            .size(11.0)
                            .color(if candidate.blocked.is_some() {
                                theme::FAINT
                            } else {
                                theme::TEXT
                            }),
                    );
                });
            });
        })
        .response
        .interact(Sense::click());
    let response = match &candidate.blocked {
        Some(reason) => response.on_hover_text(reason),
        None => response,
    };
    response.clicked() && candidate.blocked.is_none()
}

fn close(ui: &mut Ui, id: egui::Id) {
    clear_draft(ui, id.with("search"));
    clear_draft(ui, id.with("category"));
    ui.ctx()
        .data_mut(|d| d.remove_temp::<usize>(id.with("highlight")));
    ui.close();
}
