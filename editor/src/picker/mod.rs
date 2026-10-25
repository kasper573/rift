//! The picker field: how every content relation is assigned. A field holds one [`Pick`] and
//! draws it as a chip; clicking the chip opens a popover that lists what a [`PickSource`] offers,
//! filtered by free text and by the source's category chips, and dropping something dragged
//! from a browser onto the chip assigns it when the source accepts it.

pub mod assets;
pub mod rows;

pub use self::assets::AssetSource;
pub use self::rows::RowSource;

use std::sync::Arc;
use ui::chip;
use ui::drag;
use ui::egui;
use ui::menu;
use ui::theme;

use engine::core::assets::AssetKind;
use engine::core::content::{ContentFile, ContentId, table_of};
use engine::systems::area::prop::PropDef;
use ui::egui::{Align, Color32, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2};
use ui::glyph::{self, Mark};

use crate::forms::{FormAction, FormCx};
use crate::icons::{self, IconTone, KindIcon};
use crate::keymap::EditorKey;
use crate::tabs::shown_path;
use ui::input;
use ui::input::{clear_draft, draft_of, set_draft};

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
            Dragged::Row { key, .. } => shown_path(key).to_owned(),
            Dragged::Asset(path) => path.to_owned(),
        }
    }
}

/// A chip in the popover that narrows the list to one category.
#[derive(Clone)]
pub struct Category {
    pub id: String,
    pub label: String,
}

/// One thing a source offers.
#[derive(Clone)]
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
    /// How far the candidate sits from the file being edited: how many folders up from it their
    /// paths meet. Nearer candidates are listed first.
    pub nearness: usize,
}

/// What a picker field lists, how it shows a pick, and what it takes by drop.
pub trait PickSource {
    fn placeholder(&self) -> String;
    /// The chips offered for quick filtering; none hides the chip row.
    fn categories(&self, cx: &FormCx) -> Vec<Category>;
    fn candidates(&self, cx: &FormCx) -> Vec<Candidate>;
    /// The pick a dragged thing becomes on this field, or why the field refuses it.
    fn accepts(&self, cx: &FormCx, dragged: &Dragged) -> Result<Pick, String>;
    /// The muted folder and the file a pick is shown as.
    fn shown(&self, pick: &Pick) -> (String, String);
    /// The pick's name alone, as a grid labels it.
    fn short(&self, pick: &Pick) -> String;
    /// What stands for a pick that has no picture.
    fn icon(&self, pick: &Pick) -> KindIcon;
    /// What is said about a pick under the list beyond its name: its picture, its uses.
    fn describe(&self, cx: &FormCx, pick: &Pick) -> String;
    /// How the candidates are laid out until the user picks another layout.
    fn layout(&self) -> PickLayout {
        PickLayout::List
    }
    /// The action behind the arrow button beside a held pick.
    fn opens(&self, _pick: &Pick) -> Option<FormAction> {
        None
    }
    /// "New …" lines at the popover's foot that create and pick in one step.
    fn creates(&self, _cx: &FormCx) -> Vec<(String, FormAction)> {
        Vec::new()
    }
}

/// How a popover lays its candidates out: pictures with their names, or a line each with its
/// whole path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickLayout {
    Grid,
    List,
}

pub fn row(ui: &mut Ui, cx: &mut FormCx, table: &str, key: &mut String, path: &str) -> bool {
    let mut pick = Pick::new(table, key.as_str());
    let changed = field(ui, cx, &RowSource::table(table, path), &mut pick, path);
    if changed {
        *key = pick.key;
    }
    changed
}

/// A field picking the file of `Row` that `id` names, among the files as they stand now.
pub fn content<Row: ContentFile>(
    ui: &mut Ui,
    cx: &mut FormCx,
    id: &mut ContentId<Row>,
    path: &str,
) -> bool {
    let mut held = Some(*id);
    let picked = maybe_content(ui, cx, &mut held, path);
    match held {
        Some(new) if picked => {
            *id = new;
            true
        }
        _ => false,
    }
}

/// A field picking the file of `Row` that `id` names, or none.
pub fn maybe_content<Row: ContentFile>(
    ui: &mut Ui,
    cx: &mut FormCx,
    id: &mut Option<ContentId<Row>>,
    path: &str,
) -> bool {
    let reader = cx.project.reader();
    let mut key = id
        .and_then(|id| reader.path_of(id))
        .unwrap_or_default()
        .to_owned();
    if !row(ui, cx, Row::TABLE, &mut key, path) {
        return false;
    }
    let picked = (!key.is_empty())
        .then(|| reader.id_of::<Row>(&key))
        .flatten();
    if picked.is_none() && !key.is_empty() {
        return false;
    }
    *id = picked;
    true
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
    kind: AssetKind,
    file: &mut String,
    path: &str,
) -> bool {
    let mut pick = Pick::new(<&str>::from(kind), file.as_str());
    let changed = field(ui, cx, &AssetSource::of(kind), &mut pick, path);
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
            && glyph::button(ui, Mark::Arrow, 22.0, theme::MUTED, "Open")
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
                chip::floating(
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
        if let Some(dragged) = drag::dropped::<Dragged>(&response) {
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
        if std::mem::take(&mut cx.autofocus) {
            egui::Popup::open_id(ui.ctx(), id);
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

/// The narrowest a pick's folder is shown, cut short; any narrower and only its name shows.
fn chip(ui: &mut Ui, cx: &mut FormCx, source: &dyn PickSource, pick: &Pick) -> egui::Response {
    let empty = pick.is_empty();
    input::frame()
        .show(ui, |ui| {
            let row = egui::vec2(ui.available_width(), input::row_height(ui));
            ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                ui.label(Mark::CaretDown.text(11.0).color(theme::MUTED));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if empty {
                        ui.label(theme::muted(format!("pick {}…", source.placeholder())));
                        return;
                    }
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    let mut unlimited = u32::MAX;
                    face(ui, cx, source, pick, rect, &mut unlimited);
                    let (prefix, text) = source.shown(pick);
                    let name = egui::WidgetText::from(
                        theme::mono(text.clone()).size(13.0).color(theme::CODE),
                    )
                    .into_galley(
                        ui,
                        Some(egui::TextWrapMode::Truncate),
                        ui.available_width(),
                        egui::TextStyle::Monospace,
                    );
                    let room = ui.available_width() - name.size().x - ui.spacing().item_spacing.x;
                    if !prefix.is_empty() {
                        let folder = egui::WidgetText::from(
                            theme::muted(prefix.clone()).monospace().size(13.0),
                        )
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Truncate),
                            room.max(0.0),
                            egui::TextStyle::Monospace,
                        );
                        if !folder.elided {
                            ui.add(egui::Label::new(folder));
                        }
                    }
                    ui.add(egui::Label::new(name))
                        .on_hover_text(format!("{prefix}{text}"));
                });
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
    if menu::opened_afresh(ui, id) {
        forget(ui, id);
    }
    let field = path.rsplit('/').next().unwrap_or(path);
    cx.state.status = vec![
        field.to_owned(),
        if pick.is_empty() {
            format!(
                "{} move · {} picks · {} closes",
                EditorKey::Move.name(),
                EditorKey::Confirm.name(),
                EditorKey::Cancel.name()
            )
        } else {
            format!(
                "{} move · {} picks · {} keeps {}",
                EditorKey::Move.name(),
                EditorKey::Confirm.name(),
                EditorKey::Cancel.name(),
                source.short(pick)
            )
        },
    ];
    let search_id = id.with("search");
    let category_id = id.with("category");
    let layout_id = egui::Id::new(("picker layout", source.placeholder()));
    let mut search = draft_of(ui, search_id).unwrap_or_default();
    let mut category = draft_of(ui, category_id);
    let mut layout = ui
        .ctx()
        .data(|d| d.get_temp::<PickLayout>(layout_id))
        .unwrap_or_else(|| source.layout());
    let needle = search.to_lowercase();
    let found = found(ui, cx, source, id);
    let categories = &found.categories;
    let all = &found.candidates;
    let total = all.len();
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
        a.nearness
            .cmp(&b.nearness)
            .then(a.blocked.is_some().cmp(&b.blocked.is_some()))
            .then((a.pick != *pick).cmp(&(b.pick != *pick)))
            .then(a.pick.category.cmp(&b.pick.category))
            .then(a.pick.key.cmp(&b.pick.key))
    });
    let counted = if all.iter().any(|candidate| candidate.blocked.is_some()) {
        let pickable = candidates
            .iter()
            .filter(|candidate| candidate.blocked.is_none())
            .count();
        format!("{pickable} pickable")
    } else {
        format!("{} of {total}", candidates.len())
    };
    let highlight_id = id.with("highlight");
    let mut highlight: usize = ui.ctx().data(|d| d.get_temp(highlight_id)).unwrap_or(0);
    ui.horizontal(|ui| {
        let layouts = [
            (PickLayout::List, Mark::List, "List"),
            (PickLayout::Grid, Mark::Grid, "Grid"),
        ];
        let buttons = layouts.len() as f32 * (LAYOUT_BUTTON + ui.spacing().item_spacing.x);
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width() - buttons);
            input::boxed(ui, None, |ui| {
                glyph::show(ui, Mark::Search, 13.0, theme::MUTED);
                let count = ui.painter().layout_no_wrap(
                    counted.clone(),
                    egui::FontId::monospace(11.5),
                    theme::MUTED,
                );
                let response = ui.add(
                    input::unframed(egui::TextEdit::singleline(&mut search))
                        .hint_text(theme::muted(format!("filter · {}", source.placeholder())))
                        .desired_width(ui.available_width() - count.size().x - 8.0),
                );
                input::default_focus(ui, &response);
                if response.changed() {
                    set_draft(ui, search_id, &search);
                    highlight = 0;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::muted(&counted).monospace().size(11.5));
                });
                response
            });
        });
        for (choice, mark, label) in layouts {
            let color = if layout == choice {
                theme::ACCENT
            } else {
                theme::FAINT
            };
            if glyph::button(ui, mark, LAYOUT_BUTTON, color, label)
                .on_hover_text(label)
                .clicked()
            {
                layout = choice;
                ui.ctx().data_mut(|d| d.insert_temp(layout_id, choice));
            }
        }
    });
    if !categories.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for listed in categories.iter() {
                let on = category.as_deref() == Some(listed.id.as_str());
                let count = all
                    .iter()
                    .filter(|candidate| candidate.category == listed.id)
                    .count();
                if chip::toggle(ui, &format!("{} · {count}", listed.label), on).clicked() {
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
    let columns = match layout {
        PickLayout::Grid => GRID_COLUMNS,
        PickLayout::List => 1,
    };
    let (up, down, left, right, enter, escape) = ui.input(|i| {
        let arrow = EditorKey::Move.arrow(i);
        (
            arrow == Some([0, -1]),
            arrow == Some([0, 1]),
            arrow == Some([-1, 0]),
            arrow == Some([1, 0]),
            EditorKey::Confirm.pressed(i),
            EditorKey::Cancel.pressed(i),
        )
    });
    let last = candidates.len().saturating_sub(1);
    if down {
        highlight = (highlight + columns).min(last);
    }
    if up {
        highlight = highlight.saturating_sub(columns);
    }
    if layout == PickLayout::Grid {
        if right {
            highlight = (highlight + 1).min(last);
        }
        if left {
            highlight = highlight.saturating_sub(1);
        }
    }
    let mut picked: Option<Pick> = None;
    let mut hovered: Option<usize> = None;
    let mut budget = icons::DECODES_PER_FRAME;
    let moved = up || down || left || right;
    let (row_height, gap) = match layout {
        PickLayout::Grid => (TILE, GRID_GAP),
        PickLayout::List => (LINE, 0.0),
    };
    let rows = candidates.len().div_ceil(columns);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(gap);
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .id_salt(id.with("list"))
            .show_rows(ui, row_height, rows, |ui, shown| {
                let top = ui.cursor().top();
                for row in shown.clone() {
                    ui.horizontal(|ui| {
                        let in_row = candidates.iter().enumerate().skip(row * columns);
                        for (index, &candidate) in in_row.take(columns) {
                            let cell = Cell {
                                lit: index == highlight,
                                held: candidate.pick == *pick,
                            };
                            let response = match layout {
                                PickLayout::Grid => {
                                    ground(ui, cx, source, candidate, cell, &mut budget)
                                }
                                PickLayout::List => {
                                    line(ui, cx, source, candidate, cell, &mut budget)
                                }
                            };
                            if response.hovered() {
                                hovered = Some(index);
                            }
                            if response.clicked() && candidate.blocked.is_none() {
                                picked = Some(candidate.pick.clone());
                            }
                        }
                    });
                }
                if moved {
                    let row = highlight / columns;
                    let y = top + (row as f32 - shown.start as f32) * (row_height + gap);
                    let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), y..=y + row_height);
                    ui.scroll_to_rect(rect, None);
                }
                if candidates.is_empty() {
                    ui.label(theme::muted("nothing matches"));
                }
            });
    });
    if enter
        && let Some(candidate) = candidates.get(highlight)
        && candidate.blocked.is_none()
    {
        picked = Some(candidate.pick.clone());
    }
    ui.separator();
    let described = hovered
        .or((up || down || left || right).then_some(highlight))
        .and_then(|index| candidates.get(index).copied())
        .or_else(|| candidates.iter().copied().find(|c| c.pick == *pick));
    describe(ui, cx, source, described, pick);
    for (label, action) in source.creates(cx) {
        let response = ui
            .horizontal(|ui| {
                glyph::show(ui, Mark::Plus, 14.0, theme::CODE);
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

const LAYOUT_BUTTON: f32 = 22.0;
const GRID_COLUMNS: usize = 5;
const GRID_GAP: f32 = 4.0;
const TILE: f32 = 78.0;
const TILE_PICTURE: f32 = 48.0;
const LINE: f32 = 24.0;
const LINE_ICON: f32 = 16.0;

/// How a candidate stands among the rest: under the keyboard, and whether the field holds it.
#[derive(Clone, Copy)]
struct Cell {
    lit: bool,
    held: bool,
}

impl Cell {
    fn fill(self, hovered: bool) -> Color32 {
        if self.lit {
            theme::ACCENT_SOFT
        } else if hovered {
            ui::ink::gold_wash(0.2)
        } else if self.held {
            theme::RAISED
        } else {
            Color32::TRANSPARENT
        }
    }
}

/// The line under the candidates, about the one the pointer is on, else the one the keys moved
/// to, else the one the field holds.
fn describe(
    ui: &mut Ui,
    cx: &FormCx,
    source: &dyn PickSource,
    described: Option<&Candidate>,
    held: &Pick,
) {
    let Some(candidate) = described else {
        ui.label(theme::faint("point at one to read about it").size(11.5));
        return;
    };
    ui.horizontal(|ui| {
        ui.label(theme::mono(source.shown(&candidate.pick).1).size(12.5));
        if !candidate.name.is_empty() {
            ui.label(theme::muted(&candidate.name).size(12.0));
        }
        if candidate.pick == *held {
            ui.label(theme::faint("· held").size(11.5));
        }
    });
    let said = match &candidate.blocked {
        Some(reason) => RichText::new(reason).color(theme::DANGER),
        None => theme::muted(&candidate.hint),
    };
    ui.add(egui::Label::new(said.size(11.5)).wrap());
    ui.add(
        egui::Label::new(
            theme::faint(source.describe(cx, &candidate.pick))
                .monospace()
                .size(11.5),
        )
        .wrap(),
    );
}

fn line(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    candidate: &Candidate,
    cell: Cell,
    budget: &mut u32,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, LINE), Sense::click());
    ui.painter()
        .rect_filled(rect, 3.0, cell.fill(response.hovered()));
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + 6.0 + LINE_ICON / 2.0, rect.center().y),
        Vec2::splat(LINE_ICON),
    );
    face(ui, cx, source, &candidate.pick, icon, budget);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                egui::pos2(icon.right() + 6.0, rect.top()),
                egui::pos2(rect.right() - 6.0, rect.bottom()),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    let (prefix, text) = source.shown(&candidate.pick);
    let ink = if candidate.blocked.is_some() {
        theme::FAINT
    } else {
        theme::TEXT
    };
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::monospace(12.5);
    job.append(
        &prefix,
        0.0,
        egui::TextFormat::simple(font.clone(), theme::MUTED),
    );
    job.append(&text, 0.0, egui::TextFormat::simple(font, ink));
    child.add(egui::Label::new(job).truncate().selectable(false));
    if !candidate.name.is_empty() && child.available_width() > 40.0 {
        child.add(
            egui::Label::new(theme::muted(&candidate.name).size(12.0))
                .truncate()
                .selectable(false),
        );
    }
    match &candidate.blocked {
        Some(reason) => response.on_hover_text(reason),
        None => response,
    }
}

fn ground(
    ui: &mut Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    candidate: &Candidate,
    cell: Cell,
    budget: &mut u32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(TILE, TILE), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, cell.fill(response.hovered()));
    let picture = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 6.0 + TILE_PICTURE / 2.0),
        Vec2::splat(TILE_PICTURE),
    );
    face(ui, cx, source, &candidate.pick, picture, budget);
    let ink = if candidate.blocked.is_some() {
        theme::FAINT
    } else {
        theme::TEXT
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        source.short(&candidate.pick),
        egui::FontId::monospace(11.0),
        ink,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 8.0);
    let name = ui.painter().layout_job(job);
    painter.galley(
        egui::pos2(
            rect.center().x - name.size().x / 2.0,
            picture.bottom() + 4.0,
        ),
        name,
        ink,
    );
    if candidate.blocked.is_some() {
        painter.rect_filled(rect, 4.0, theme::PANEL.gamma_multiply(0.4));
    }
    match &candidate.blocked {
        Some(reason) => response.on_hover_text(reason),
        None => response,
    }
}

/// What stands for a pick: its picture, or its kind's icon.
fn face(
    ui: &Ui,
    cx: &mut FormCx,
    source: &dyn PickSource,
    pick: &Pick,
    rect: Rect,
    budget: &mut u32,
) {
    if table_of(&pick.key) == Some(PropDef::TABLE)
        && let Some(texture) = cx.state.prop_thumbnails.thumbnail(&pick.key)
    {
        let side = rect.width().min(rect.height());
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        ui.painter()
            .image(texture, square, crate::thumbs::FULL_UV, Color32::WHITE);
        return;
    }
    let picture = icons::picture(
        ui.ctx(),
        &mut cx.state.thumbnails,
        cx.project,
        &pick.key,
        budget,
    );
    let icon = source.icon(pick);
    icons::paint_file(ui.painter(), rect, picture, icon, IconTone::Normal);
}

/// What an open popover lists, found again only once the files or what they load into change:
/// finding it reads every file of the kinds it lists.
#[derive(Clone)]
struct Found {
    from: (u64, u64),
    categories: Arc<[Category]>,
    candidates: Arc<[Candidate]>,
}

fn found(ui: &Ui, cx: &FormCx, source: &dyn PickSource, id: egui::Id) -> Found {
    let key = id.with("found");
    let from = (cx.project.files.revision(), cx.project.generation);
    if let Some(found) = ui
        .ctx()
        .data(|d| d.get_temp::<Found>(key))
        .filter(|found| found.from == from)
    {
        return found;
    }
    let found = Found {
        from,
        categories: source.categories(cx).into(),
        candidates: source.candidates(cx).into(),
    };
    ui.ctx().data_mut(|d| d.insert_temp(key, found.clone()));
    found
}

fn close(ui: &mut Ui, id: egui::Id) {
    forget(ui, id);
    ui.close();
}

fn forget(ui: &Ui, id: egui::Id) {
    ui.ctx().data_mut(|d| d.remove::<Found>(id.with("found")));
    clear_draft(ui, id.with("search"));
    clear_draft(ui, id.with("category"));
    ui.ctx()
        .data_mut(|d| d.remove_temp::<usize>(id.with("highlight")));
}
