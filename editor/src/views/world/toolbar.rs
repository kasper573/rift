use egui::{Align, Layout, RichText, Ui, Vec2};
use engine::core::content::ContentRow;
use engine::systems::item::ItemDef;
use engine::systems::npc::NpcDef;
use engine::systems::quest::QuestDef;
use engine::systems::shop::ShopDef;

use super::filters::{ColorBy, ColorField, MapLens};
use super::{MapPart, OpenWorldMap, select};
use crate::app::Editor;
use crate::ui::{theme, widgets};

const SEARCH_WIDTH: f32 = 300.0;
const RESULTS_PER_KIND: usize = 6;
const RESULTS_SHOWN: usize = 24;

pub fn show(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap) {
    let mut pick: Option<Pick> = None;
    let mut remove: Option<Chip> = None;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let search = search_box(editor, ui, &mut remove);
                if !editor.views.world.search.trim().is_empty() {
                    pick = results(editor, ui, map, &search);
                }
                ui.menu_button(RichText::new("Layers ▾").size(12.5), |ui| {
                    ui.set_width(220.0);
                    layer_rows(editor, ui, map);
                });
                ui.menu_button(RichText::new("Color by ▾").size(12.5), |ui| {
                    ui.set_width(260.0);
                    if let Some(color) = color_menu(editor, ui) {
                        pick = Some(Pick::Color(color));
                        ui.close();
                    }
                });
                if ui.button(RichText::new("Arrange…").size(12.5)).clicked() {
                    super::arrange::open(editor);
                }
                if ui.button(RichText::new("Fit").size(12.5)).clicked() {
                    editor.views.world.request_fit();
                }
                ui.label(
                    theme::faint(format!("zoom {:.0}%", editor.views.world.zoom * 100.0))
                        .monospace()
                        .size(12.0),
                );
            });
        });
    let view = &mut editor.views.world;
    match remove {
        Some(Chip::Lens(index)) => {
            view.lenses.remove(index);
        }
        Some(Chip::Color) => view.color = None,
        Some(Chip::Line) => view.quest_line = None,
        Some(Chip::Route) => view.route = None,
        None => {}
    }
    if let Some(pick) = pick {
        apply(editor, map, pick);
    }
}

pub fn layer_rows(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap) {
    let atlas = &map.atlas;
    let count = |of: fn(&super::atlas::AtlasArea) -> usize| atlas.areas.values().map(of).sum();
    let givers: usize = atlas
        .areas
        .keys()
        .map(|area| atlas.givers(*area).count())
        .sum();
    let layers = &mut editor.views.world.layers;
    for (on, label, count) in [
        (&mut layers.warps, "warps", atlas.warps.len()),
        (
            &mut layers.residents,
            "residents",
            count(|info| info.residents.len()),
        ),
        (
            &mut layers.populations,
            "populations",
            count(|info| info.populations.len()),
        ),
        (&mut layers.givers, "quest givers", givers),
        (
            &mut layers.regions,
            "regions and notes",
            map.layout.regions.len() + map.layout.notes.len(),
        ),
    ] {
        ui.horizontal(|ui| {
            widgets::toggle(ui, on);
            ui.label(RichText::new(label).size(13.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::faint(count.to_string()).monospace().size(12.0));
            });
        });
    }
}

enum Chip {
    Lens(usize),
    Color,
    Line,
    Route,
}

#[derive(Clone)]
enum Pick {
    Area(String),
    Lens(MapLens),
    Line(String),
    Color(ColorBy),
}

struct SearchResult {
    kind: &'static str,
    label: String,
    pick: Pick,
}

fn search_box(editor: &mut Editor, ui: &mut Ui, remove: &mut Option<Chip>) -> egui::Response {
    egui::Frame::new()
        .fill(theme::INPUT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_min_width(SEARCH_WIDTH);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(theme::muted("🔍"));
                let view = &mut editor.views.world;
                let mut chip = |ui: &mut Ui, text: &str, which: Chip| {
                    if widgets::chip_closable(ui, text).1 {
                        *remove = Some(which);
                    }
                };
                for (index, lens) in view.lenses.iter().enumerate() {
                    chip(ui, &lens.chip(), Chip::Lens(index));
                }
                if let Some(color) = &view.color {
                    chip(ui, &color.chip(), Chip::Color);
                }
                if let Some(line) = &view.quest_line {
                    chip(ui, &format!("quest line: {line}"), Chip::Line);
                }
                if let Some(route) = &view.route {
                    chip(
                        ui,
                        &format!("route: {} → {}", route.from, route.to),
                        Chip::Route,
                    );
                }
                let empty = view.lenses.is_empty()
                    && view.color.is_none()
                    && view.quest_line.is_none()
                    && view.route.is_none();
                let hint = if empty {
                    "Search areas, npcs, items, quests…"
                } else {
                    ""
                };
                ui.add(
                    egui::TextEdit::singleline(&mut view.search)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .desired_width(if empty { SEARCH_WIDTH - 30.0 } else { 120.0 })
                        .font(egui::TextStyle::Body),
                )
            })
            .inner
        })
        .inner
}

fn results(
    editor: &mut Editor,
    ui: &mut Ui,
    map: &OpenWorldMap,
    search: &egui::Response,
) -> Option<Pick> {
    let text = editor.views.world.search.trim().to_lowercase();
    let atlas = &map.atlas;
    let mut rows: Vec<SearchResult> = atlas
        .areas
        .values()
        .filter(|info| info.key.contains(&text))
        .take(RESULTS_PER_KIND)
        .map(|info| SearchResult {
            kind: "area",
            label: info.key.clone(),
            pick: Pick::Area(info.key.clone()),
        })
        .collect();
    for table in [
        NpcDef::TABLE,
        ItemDef::TABLE,
        QuestDef::TABLE,
        ShopDef::TABLE,
    ] {
        let Some(keys) = map.content.table_named(table).map(|rows| rows.keys()) else {
            continue;
        };
        rows.extend(
            keys.iter()
                .filter(|key| key.contains(&text))
                .map(|key| SearchResult {
                    kind: table,
                    label: key.clone(),
                    pick: Pick::Lens(MapLens::Row {
                        table: table.to_owned(),
                        key: key.clone(),
                    }),
                }),
        );
    }
    rows.extend(
        atlas
            .lines
            .iter()
            .filter(|line| line.category.to_lowercase().contains(&text))
            .map(|line| SearchResult {
                kind: "quest line",
                label: format!("{} · {} quests", line.category, line.quests.len()),
                pick: Pick::Line(line.category.clone()),
            }),
    );
    rows.extend(
        atlas
            .module_kinds
            .iter()
            .filter(|kind| kind.tag().contains(&text))
            .take(RESULTS_PER_KIND)
            .map(|kind| SearchResult {
                kind: "module",
                label: kind.tag().to_owned(),
                pick: Pick::Lens(MapLens::Module(*kind)),
            }),
    );
    rows.push(SearchResult {
        kind: "text",
        label: format!("“{text}” anywhere in an area"),
        pick: Pick::Lens(MapLens::Text(text.clone())),
    });
    let mut picked: Option<Pick> = None;
    if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        picked = rows.first().map(|row| row.pick.clone());
    }
    egui::Area::new(ui.id().with("world-search-results"))
        .fixed_pos(search.rect.left_bottom() + Vec2::new(-8.0, 8.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::RAISED)
                .show(ui, |ui| {
                    ui.set_width(SEARCH_WIDTH + 40.0);
                    let mut last_kind = "";
                    for row in rows.iter().take(RESULTS_SHOWN) {
                        if row.kind != last_kind {
                            widgets::section(ui, row.kind);
                            last_kind = row.kind;
                        }
                        let response = ui.add(
                            egui::Button::new(theme::mono(&row.label).size(12.5))
                                .frame(false)
                                .min_size(Vec2::new(ui.available_width(), 20.0)),
                        );
                        if response.clicked() {
                            picked = Some(row.pick.clone());
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(theme::faint("Enter takes the first · Esc clears").size(11.0));
                });
        });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        editor.views.world.search.clear();
    }
    picked
}

fn apply(editor: &mut Editor, map: &OpenWorldMap, pick: Pick) {
    let view = &mut editor.views.world;
    view.search.clear();
    match pick {
        Pick::Area(key) => {
            if let Some(area) = map.atlas.id(&key) {
                let zoom = view.zoom;
                view.look_at(map, area, zoom);
            }
            select(editor, vec![MapPart::Card(key)]);
        }
        Pick::Lens(lens) => {
            if !view.lenses.contains(&lens) {
                view.lenses.push(lens);
            }
        }
        Pick::Line(category) => view.quest_line = Some(category),
        Pick::Color(color) => view.color = Some(color),
    }
}

fn color_menu(editor: &mut Editor, ui: &mut Ui) -> Option<ColorBy> {
    let fields = ColorField::all(editor);
    let mut picked = None;
    if ui
        .add(egui::Button::new(RichText::new("none").size(13.0)).frame(false))
        .clicked()
    {
        editor.views.world.color = None;
        ui.close();
    }
    let current = editor.views.world.color.as_ref().map(|color| &color.field);
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            let mut last_table = None;
            for field in fields {
                if last_table != Some(field.table) {
                    widgets::section(ui, field.table.table());
                    last_table = Some(field.table);
                }
                let button = egui::Button::new(theme::mono(field.path_text()).size(12.5))
                    .frame(false)
                    .selected(current == Some(&field));
                if ui.add(button).clicked() {
                    picked = Some(ColorBy::new(field));
                }
            }
        });
    picked
}
