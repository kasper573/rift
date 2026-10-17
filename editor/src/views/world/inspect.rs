use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Layout, Rect, RichText, Sense, Ui, Vec2};
use engine::core::content::{ContentRow, Fixture};
use engine::core::tiling::TilePos;
use engine::systems::area::world_map::{
    MAX_NOTE, MAX_SIZE, MIN_SIZE, RegionColor, WorldMap, WorldMapDef,
};
use engine::systems::area::{self, AreaId};
use strum::VariantArray;

use super::atlas::{OpenTarget, RowSource, RowSourceKind, WorldAtlas};
use super::canvas::{line_badge, viridis};
use super::filters::{self, AreaRange, ColorField, ColorOf, ColorRanges, ColorShow, ColorTable};
use super::{MapPart, MapTileHit, OpenWorldMap, region_color, region_color_name, select, toolbar};
use crate::app::Editor;
use crate::forms;
use crate::project::{ContentEdit, RowSelection};
use crate::tabs::EditorTab;
use crate::ui::browser::{self, BrowserAction};
use crate::ui::{theme, widgets};
use crate::views::area as area_view;
use crate::{play, value};

fn inspect_world(editor: &mut Editor, ui: &mut Ui) {
    let Some(map) = OpenWorldMap::of(editor) else {
        return;
    };
    let view = &editor.views.world;
    let hover = view.menu_target.clone().or_else(|| view.hover.clone());
    if super::shows_areas(view.zoom)
        && let Some(hit) = hover
    {
        tile_panel(editor, ui, &map, &hit);
    } else if view.route.is_some() {
        route_panel(editor, ui, &map.atlas);
    } else if view.quest_line.is_some() {
        line_panel(editor, ui, &map);
    } else if view.color.is_some() {
        color_panel(editor, ui, &map.atlas);
    } else if !view.lenses.is_empty() {
        lens_panel(editor, ui, &map.atlas);
    } else {
        overview(editor, ui, &map);
    }
}

pub fn inspect_parts(editor: &mut Editor, ui: &mut Ui, selection: &RowSelection) {
    let Some(map) = OpenWorldMap::of(editor) else {
        return;
    };
    let parts: Vec<MapPart> = selection
        .parts
        .iter()
        .filter_map(|part| part.parse().ok())
        .collect();
    let cards = map.card_ids(&parts);
    match parts.first() {
        _ if !cards.is_empty() => inspect_cards(editor, ui, &map, &cards),
        Some(MapPart::Edge(from, to)) => inspect_edge(editor, ui, &map.atlas, from, to),
        Some(MapPart::Region(index)) if *index < map.layout.regions.len() => {
            inspect_region(editor, ui, &map, *index);
        }
        Some(MapPart::Note(index)) if *index < map.layout.notes.len() => {
            inspect_note(editor, ui, &map, *index);
        }
        _ => inspect_world(editor, ui),
    }
}

fn crumb(ui: &mut Ui, tail: &str) {
    ui.label(
        theme::mono(format!("World · {tail}"))
            .size(12.5)
            .color(theme::MUTED),
    );
    ui.add_space(6.0);
}

fn open(editor: &mut Editor, opens: &OpenTarget) {
    match opens {
        OpenTarget::Area {
            key,
            module: Some(module),
        } => area_view::open_area_at(editor, key, None, Some(format!("/modules/{module}"))),
        OpenTarget::Area { key, module: None } => {
            browser::act(editor, area::AreaDef::TABLE, key, BrowserAction::Open)
        }
        OpenTarget::Row { table, key } => browser::act(editor, table, key, BrowserAction::Open),
    }
}

fn go_to(editor: &mut Editor, map: &OpenWorldMap, area: AreaId) {
    let zoom = editor.views.world.zoom;
    editor.views.world.look_at(map, area, zoom);
}

fn overview(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap) {
    let atlas = &map.atlas;
    crumb(
        ui,
        &format!("{}/{}", WorldMapDef::TABLE, WorldMap::World.key()),
    );
    widgets::section(ui, "Layers");
    toolbar::layer_rows(editor, ui, map);
    ui.add_space(8.0);
    widgets::section(ui, "This world");
    let one_way = atlas
        .warps
        .iter()
        .filter(|warp| !atlas.both_ways(warp))
        .count();
    let unreached = atlas
        .areas
        .keys()
        .filter(|area| !atlas.distances.contains_key(area))
        .count();
    let furthest = atlas.distances.values().copied().max().unwrap_or(0);
    widgets::kv_text(ui, "areas", atlas.areas.len().to_string());
    widgets::kv_text(
        ui,
        "warps",
        match one_way {
            0 => format!("{}, each with a way back", atlas.warps.len()),
            _ => format!("{}, {one_way} one way", atlas.warps.len()),
        },
    );
    widgets::kv_text(
        ui,
        "from the initial spawn",
        match unreached {
            0 => format!("every area, at most {furthest} warps away"),
            _ => format!("{unreached} areas unreached"),
        },
    );
    let residents: usize = atlas.areas.values().map(|info| info.residents.len()).sum();
    let populations: usize = atlas
        .areas
        .values()
        .map(|info| info.populations.len())
        .sum();
    widgets::kv_text(
        ui,
        "npcs placed",
        format!("{residents} residents, {populations} populations"),
    );
    ui.add_space(8.0);
}

fn lens_panel(editor: &mut Editor, ui: &mut Ui, atlas: &WorldAtlas) {
    let Some(lit) = filters::lit_areas(atlas, &editor.views.world.lenses) else {
        return;
    };
    crumb(
        ui,
        &editor
            .views
            .world
            .lenses
            .iter()
            .map(|lens| lens.crumb())
            .collect::<Vec<_>>()
            .join(" · "),
    );
    let mut by_kind: BTreeMap<RowSourceKind, BTreeMap<&RowSource, Vec<&str>>> = BTreeMap::new();
    for (area, sources) in &lit {
        for source in sources {
            by_kind
                .entry(source.kind)
                .or_default()
                .entry(source)
                .or_default()
                .push(atlas.key(*area));
        }
    }
    if by_kind.is_empty() {
        ui.label(theme::muted("Nothing on the map matches."));
        return;
    }
    let mut opened: Option<OpenTarget> = None;
    for (kind, sources) in &by_kind {
        let count = if kind.per_area() {
            sources.values().flatten().collect::<BTreeSet<_>>().len()
        } else {
            sources.len()
        };
        widgets::section(
            ui,
            &format!(
                "{} · {count} {}{}",
                kind.title(),
                kind.noun(),
                widgets::plural(count)
            ),
        );
        for (source, areas) in sources {
            ui.horizontal(|ui| {
                let name = ui
                    .add(
                        egui::Label::new(theme::mono(&source.name).size(12.5))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if name.clicked() {
                    opened = Some(source.opens.clone());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::faint(areas.join(" · ")).monospace().size(12.0));
                });
            });
        }
        ui.add_space(6.0);
    }
    ui.label(
        theme::faint(
            "Click a source to open its row. Works for any row, module type or field value.",
        )
        .size(11.5),
    );
    if let Some(opens) = opened {
        open(editor, &opens);
    }
}

struct DistanceRow<'a> {
    key: &'a str,
    distance: Option<u32>,
    range: AreaRange,
}

fn color_panel(editor: &mut Editor, ui: &mut Ui, atlas: &WorldAtlas) {
    crumb(ui, "color by");
    let Some(color) = editor.views.world.color.clone() else {
        return;
    };
    let Some(project) = editor.project.as_ref() else {
        return;
    };
    let ranges = ColorRanges::of(atlas, &project.files, &color);
    let fields = ColorField::all(editor);
    let mut next = color.clone();
    widgets::kv(ui, "value", |ui| {
        widgets::combo_box("color-value")
            .width(ui.available_width() - 8.0)
            .selected_text(theme::mono(next.field.text()).size(12.5))
            .show_ui(ui, |ui| {
                for field in fields {
                    let text = field.text();
                    ui.selectable_value(&mut next.field, field, theme::mono(text));
                }
            });
    });
    if next.field.table == ColorTable::Npc {
        widgets::kv(ui, "of", |ui| {
            widgets::combo_box("color-of")
                .width(ui.available_width() - 8.0)
                .selected_text(theme::mono(next.of.label()).size(12.5))
                .show_ui(ui, |ui| {
                    for &of in ColorOf::VARIANTS {
                        ui.selectable_value(&mut next.of, of, of.label());
                    }
                });
        });
    }
    widgets::kv(ui, "show", |ui| {
        widgets::combo_box("color-show")
            .width(ui.available_width() - 8.0)
            .selected_text(theme::mono(next.show.label()).size(12.5))
            .show_ui(ui, |ui| {
                for &show in ColorShow::VARIANTS {
                    ui.selectable_value(&mut next.show, show, show.label());
                }
            });
    });
    ui.add_space(6.0);
    color_bar(ui, &ranges, color.field.name());
    ui.add_space(8.0);
    widgets::section(ui, "By distance from the initial spawn");
    let mut rows: Vec<DistanceRow> = ranges
        .areas
        .iter()
        .map(|(area, range)| DistanceRow {
            key: atlas.key(*area),
            distance: atlas.distances.get(area).copied(),
            range: *range,
        })
        .collect();
    rows.sort_by_key(|row| (row.distance.unwrap_or(u32::MAX), row.key));
    let shown = |row: &DistanceRow| row.range.shown(color.show);
    let bumps: Vec<bool> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            rows[index + 1..].iter().any(|later| {
                later.distance.unwrap_or(u32::MAX) > row.distance.unwrap_or(u32::MAX)
                    && shown(later) < shown(row)
            })
        })
        .collect();
    egui::Grid::new("color-rows")
        .num_columns(3)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            ui.label(theme::faint("area").size(11.0));
            ui.label(theme::faint("warps").size(11.0));
            ui.label(theme::faint(color.field.name()).size(11.0));
            ui.end_row();
            for (row, bump) in rows.iter().zip(&bumps) {
                ui.label(
                    theme::mono(format!("{}{}", row.key, if *bump { " ▲" } else { "" }))
                        .size(12.5)
                        .color(if *bump { theme::WARN } else { theme::TEXT }),
                );
                ui.label(
                    theme::mono(
                        row.distance
                            .map_or_else(|| "–".to_owned(), |d| d.to_string()),
                    )
                    .size(12.5),
                );
                ui.label(theme::mono(row.range.text()).size(12.5));
                ui.end_row();
            }
        });
    if bumps.contains(&true) {
        ui.add_space(6.0);
        ui.label(theme::muted("▲ above the curve: stronger than areas further out.").size(12.0));
    }
    if next != color {
        editor.views.world.color = Some(next);
    }
}

fn color_bar(ui: &mut Ui, ranges: &ColorRanges, unit: &str) {
    const STEPS: usize = 40;
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
    for step in 0..STEPS {
        let from = step as f32 / STEPS as f32;
        let to = (step + 1) as f32 / STEPS as f32;
        let slice = Rect::from_min_max(
            egui::pos2(bar.min.x + bar.width() * from, bar.min.y),
            egui::pos2(bar.min.x + bar.width() * to, bar.max.y),
        );
        ui.painter().rect_filled(slice, 0.0, viridis(from));
    }
    let number = |value: f64| theme::faint(value.to_string()).monospace().size(11.0);
    ui.horizontal(|ui| {
        ui.label(number(ranges.low));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                theme::faint(format!("{} {unit}", ranges.high))
                    .monospace()
                    .size(11.0),
            );
            ui.label(number((ranges.low + ranges.high) / 2.0));
        });
    });
}

fn line_panel(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap) {
    crumb(ui, "quest line");
    let Some(category) = editor.views.world.quest_line.clone() else {
        return;
    };
    let atlas = &map.atlas;
    let quests = atlas
        .lines
        .iter()
        .find(|line| line.category == category)
        .map_or(0, |line| line.quests.len());
    widgets::section(ui, &format!("{category} · {quests} quests"));
    let mut go: Option<AreaId> = None;
    for step in atlas.line_steps(&category) {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            line_badge(ui.painter(), rect.center(), &step.badge, step.branch);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(&step.label).size(12.5));
                ui.label(theme::faint("·").size(12.0));
                for &area in &step.areas {
                    let label =
                        egui::Label::new(theme::faint(atlas.key(area)).monospace().size(12.0))
                            .sense(Sense::click());
                    if ui.add(label).clicked() {
                        go = Some(area);
                    }
                }
            });
        });
    }
    ui.add_space(8.0);
    widgets::section(ui, "Other quest lines");
    for line in atlas.lines.iter().filter(|line| line.category != category) {
        let name = egui::Label::new(RichText::new(&line.category).size(12.5)).sense(Sense::click());
        if ui.add(name).clicked() {
            editor.views.world.quest_line = Some(line.category.clone());
        }
        ui.label(
            theme::mono(line.quests.join(" → "))
                .size(11.5)
                .color(theme::MUTED),
        );
        ui.add_space(4.0);
    }
    if let Some(area) = go {
        go_to(editor, map, area);
    }
}

fn route_panel(editor: &mut Editor, ui: &mut Ui, atlas: &WorldAtlas) {
    crumb(ui, "route");
    let Some(route) = editor.views.world.route.clone() else {
        return;
    };
    let hops = atlas
        .id(&route.from)
        .zip(atlas.id(&route.to))
        .and_then(|(from, to)| atlas.route(from, to));
    let heading = format!("{} → {}", route.from, route.to);
    match hops {
        Some(hops) => {
            widgets::section(ui, &format!("{heading} · {} warps", hops.len()));
            for hop in hops {
                ui.horizontal(|ui| {
                    ui.label(theme::mono(atlas.key(hop.from)).size(12.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            theme::faint(&atlas.owners[hop.owner].name)
                                .monospace()
                                .size(12.0),
                        );
                    });
                });
            }
        }
        None => {
            widgets::section(ui, &heading);
            ui.label(theme::muted("No chain of warps leads there."));
        }
    }
    ui.add_space(8.0);
    widgets::section(ui, "Reach");
    let unreached: Vec<&str> = atlas
        .areas
        .iter()
        .filter(|(area, _)| !atlas.distances.contains_key(area))
        .map(|(_, info)| info.key.as_str())
        .collect();
    check(
        ui,
        unreached.is_empty(),
        &if unreached.is_empty() {
            format!(
                "all {} areas can be reached from the initial spawn",
                atlas.areas.len()
            )
        } else {
            format!(
                "{} areas no warp reaches: {}",
                unreached.len(),
                unreached.join(", ")
            )
        },
    );
    let one_way: Vec<String> = atlas
        .warps
        .iter()
        .filter(|warp| !atlas.both_ways(warp))
        .map(|warp| format!("{} → {}", atlas.key(warp.from), atlas.key(warp.to)))
        .collect();
    check(
        ui,
        one_way.is_empty(),
        &if one_way.is_empty() {
            "no one-way warps".to_owned()
        } else {
            format!("{} one-way warps: {}", one_way.len(), one_way.join(", "))
        },
    );
    ui.add_space(8.0);
}

fn check(ui: &mut Ui, ok: bool, text: &str) {
    ui.horizontal_wrapped(|ui| {
        if ok {
            ui.label(RichText::new("✓").color(theme::OK))
        } else {
            ui.label(RichText::new("·").color(theme::WARN))
        };
        ui.label(RichText::new(text).size(12.5));
    });
}

fn tile_panel(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, hit: &MapTileHit) {
    let atlas = &map.atlas;
    let Some(id) = atlas.id(&hit.area) else {
        return;
    };
    let info = &atlas.areas[&id];
    let cell = hit.at.snap();
    crumb(ui, &format!("{} · tile {}, {}", hit.area, cell.x, cell.y));
    let loaded = area::load(&map.content, id);
    let safe = loaded.safe_zones.iter().any(|zone| zone.contains(cell));
    widgets::kv_text(
        ui,
        "walkable",
        if loaded.grid.walkable(cell) {
            "yes"
        } else {
            "no"
        },
    );
    widgets::kv_text(
        ui,
        "safe zone",
        if safe { "inside one" } else { "none here" },
    );
    let nearest_npc = info
        .residents
        .iter()
        .chain(&info.populations)
        .map(|npc| (npc.key.as_str(), npc.at.distance(cell).0))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    widgets::kv_text(
        ui,
        "nearest npc",
        nearest_npc.map_or_else(
            || "none placed".to_owned(),
            |(npc, tiles)| format!("{npc}, {} tiles", tiles.round()),
        ),
    );
    let nearest_door = info
        .doors
        .iter()
        .map(|door| {
            (
                atlas.owners[door.owner].name.as_str(),
                door.at.distance(cell).0,
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    widgets::kv_text(
        ui,
        "nearest door",
        nearest_door.map_or_else(
            || "none".to_owned(),
            |(door, tiles)| format!("{door}, {} tiles", tiles.round()),
        ),
    );
    ui.add_space(8.0);
    widgets::section(ui, "Test character");
    let characters = play::test_characters(editor);
    let current = play::current_character(editor).unwrap_or_default();
    let mut picked = current.clone();
    let shown = characters
        .iter()
        .find(|character| character.key == current)
        .map_or_else(
            || "none".to_owned(),
            |character| format!("{} · level {}", character.key, character.level),
        );
    widgets::combo_box("world-play-character")
        .width(ui.available_width() - 8.0)
        .selected_text(theme::mono(shown).size(12.5))
        .show_ui(ui, |ui| {
            for character in &characters {
                ui.selectable_value(
                    &mut picked,
                    character.key.clone(),
                    format!("{} · level {}", character.key, character.level),
                );
            }
        });
    if picked != current {
        editor.play.character = Some(picked);
    }
    ui.add_space(6.0);
}

fn inspect_edge(editor: &mut Editor, ui: &mut Ui, atlas: &WorldAtlas, a: &str, b: &str) {
    crumb(ui, &format!("{a} ↔ {b}"));
    let (Some(a), Some(b)) = (atlas.id(a), atlas.id(b)) else {
        return;
    };
    let mut opened: Option<OpenTarget> = None;
    for (from, to) in [(a, b), (b, a)] {
        widgets::section(ui, &format!("{} → {}", atlas.key(from), atlas.key(to)));
        let mut any = false;
        for warp in atlas.warps_between(from, to) {
            any = true;
            let owner = &atlas.owners[warp.owner];
            ui.horizontal(|ui| {
                widgets::badge(ui, owner.kind.into());
                let name =
                    egui::Label::new(theme::mono(&owner.name).size(12.5)).sense(Sense::click());
                if ui.add(name).on_hover_text(&owner.label).clicked() {
                    opened = Some(owner.opens.clone());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        theme::faint(format!("→ {}, {}", warp.landing.x, warp.landing.y))
                            .monospace()
                            .size(12.0),
                    );
                });
            });
        }
        if !any {
            ui.label(theme::muted("no warp this way").size(12.5));
        }
        ui.add_space(6.0);
    }
    if let Some(opens) = opened {
        open(editor, &opens);
    }
}

fn inspect_cards(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, cards: &[AreaId]) {
    let atlas = &map.atlas;
    let first = cards[0];
    let first_key = atlas.key(first);
    let card = map.layout.cards[&first];
    let title = match cards.len() {
        1 => first_key.to_owned(),
        n => format!("{n} cards"),
    };
    crumb(
        ui,
        &match cards.len() {
            1 => format!("card · {first_key}"),
            _ => title.clone(),
        },
    );
    let mut next = map.layout.clone();
    let mut label: Option<String> = None;
    let mut open_area = false;
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, &title);
        if cards.len() == 1 {
            widgets::kv(ui, "area", |ui| {
                open_area =
                    widgets::keyed_button(ui, &format!("area/ {first_key}"), "→", true).clicked();
            });
            widgets::kv(ui, "at", |ui| {
                let (mut x, mut y) = (card.at.x, card.at.y);
                let response = forms::boxed_input(ui, Some("canvas"), |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let a = ui.add(egui::DragValue::new(&mut x).speed(1.0).fixed_decimals(0));
                        ui.label(theme::faint(","));
                        let b = ui.add(egui::DragValue::new(&mut y).speed(1.0).fixed_decimals(0));
                        a | b
                    })
                    .inner
                });
                if response.changed()
                    && let Some(moved) = next.cards.get_mut(&first)
                {
                    super::move_card(moved, egui::pos2(x, y));
                    label = Some(format!("Move {first_key} on the world map"));
                }
            });
        }
        widgets::kv(ui, "size", |ui| {
            let mut size = card.size;
            let by: i32 = match (
                ui.small_button("−").clicked(),
                ui.small_button("+").clicked(),
            ) {
                (true, _) => -10,
                (_, true) => 10,
                _ => 0,
            };
            let response = forms::boxed_input(ui, Some("%"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut size)
                        .range(MIN_SIZE..=MAX_SIZE)
                        .speed(1.0),
                )
            });
            if response.changed() || by != 0 {
                let size = size.saturating_add_signed(by).clamp(MIN_SIZE, MAX_SIZE);
                for area in cards {
                    if let Some(resized) = next.cards.get_mut(area) {
                        resized.size = size;
                    }
                }
                label = Some(format!("Resize {title} on the world map"));
            }
        });
        widgets::kv(ui, "pinned", |ui| {
            let mut pinned = cards.iter().all(|area| map.layout.cards[area].pinned);
            if widgets::toggle(ui, &mut pinned).changed() {
                for area in cards {
                    if let Some(pin) = next.cards.get_mut(area) {
                        pin.pinned = pinned;
                    }
                }
                let verb = if pinned { "Pin" } else { "Unpin" };
                label = Some(format!("{verb} {title}"));
            }
        });
        if let Some(region) = map
            .layout
            .regions
            .iter()
            .find(|region| region.cards.contains(&first))
        {
            widgets::kv_text(ui, "region", region.name.clone());
        }
    });
    if let Some(label) = label {
        map.write(editor, ContentEdit::new(label).merging(), &next);
    }
    if open_area {
        editor.open_tab(EditorTab::Area {
            key: first_key.to_owned(),
        });
    }
    widgets::section(ui, "Keys");
    for (keys, text) in [
        (&["−", "="][..], "size of every selected card, 10% a step"),
        (&["Shift"][..], "with them, 1% a step"),
        (&["←", "→", "↑", "↓"][..], "move the selection"),
        (&["P"][..], "pin or unpin the selection"),
        (&["Ctrl A"][..], "select every card"),
    ] {
        ui.horizontal(|ui| {
            for key in keys {
                widgets::key_hint(ui, key);
            }
            ui.label(theme::muted(text));
        });
    }
    widgets::section(
        ui,
        &format!(
            "Stored in {}/{}.json",
            WorldMapDef::TABLE,
            WorldMap::World.key()
        ),
    );
    let stored: serde_json::Map<String, serde_json::Value> = cards
        .iter()
        .filter_map(|area| {
            let json = value::json(map.layout.cards.get(area)?).ok()?;
            Some((atlas.key(*area).to_owned(), json))
        })
        .collect();
    let text = serde_json::to_string_pretty(&stored).unwrap_or_default();
    ui.label(theme::mono(text).size(12.0).color(theme::CODE));
    if cards.len() == 1 {
        let info = &atlas.areas[&first];
        ui.add_space(8.0);
        ui.label(
            theme::faint(format!(
                "{} · {} × {} tiles · {}",
                info.name,
                info.size.width,
                info.size.height,
                match atlas.distances.get(&first) {
                    Some(0) => "the initial spawn".to_owned(),
                    Some(warps) => format!("{warps} warps from the initial spawn"),
                    None => "no warp reaches it".to_owned(),
                }
            ))
            .size(11.5),
        );
    }
}

fn inspect_region(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, index: usize) {
    crumb(ui, "region");
    let region = &map.layout.regions[index];
    let mut next = map.layout.clone();
    let mut label: Option<String> = None;
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, &region.name);
        widgets::kv(ui, "name", |ui| {
            let id = ui.id().with(("region-name", index));
            let typed = forms::drafted(ui, id, &region.name, |ui, draft| {
                ui.add(egui::TextEdit::singleline(draft).desired_width(ui.available_width() - 8.0))
            });
            if let Some(typed) = typed
                && !typed.trim().is_empty()
                && typed.trim() != region.name
            {
                next.regions[index].name = typed.trim().to_owned();
                label = Some(format!("Rename region to {}", typed.trim()));
            }
        });
        widgets::kv(ui, "color", |ui| {
            let mut picked = region.color;
            let current = region_color_name(picked);
            widgets::combo_box(("region-color", index))
                .width(ui.available_width() - 8.0)
                .selected_text(theme::mono(current).size(12.5).color(region_color(picked)))
                .show_ui(ui, |ui| {
                    for color in [None]
                        .into_iter()
                        .chain(RegionColor::VARIANTS.iter().copied().map(Some))
                    {
                        ui.selectable_value(
                            &mut picked,
                            color,
                            RichText::new(region_color_name(color)).color(region_color(color)),
                        );
                    }
                });
            if picked != region.color {
                next.regions[index].color = picked;
                label = Some(format!("Color region {}", region.name));
            }
        });
        let holds: Vec<&str> = region
            .cards
            .iter()
            .map(|area| map.atlas.key(*area))
            .collect();
        widgets::kv_text(
            ui,
            "holds",
            format!("{} cards: {}", holds.len(), holds.join(", ")),
        );
    });
    if let Some(label) = label {
        map.write(editor, ContentEdit::new(label), &next);
        return;
    }
    ui.add_space(8.0);
    if widgets::danger(ui, "Delete region").clicked() {
        let label = format!("Delete region {}", region.name);
        if map.change(editor, ContentEdit::new(label), |layout| {
            layout.regions.remove(index);
        }) {
            select(editor, Vec::new());
        }
    }
}

fn inspect_note(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, index: usize) {
    crumb(ui, "note");
    let note = &map.layout.notes[index];
    widgets::section(ui, "Note");
    let id = ui.id().with(("note-text", index));
    let mut count = note.text.chars().count();
    let typed = forms::drafted(ui, id, &note.text, |ui, draft| {
        let response = ui.add(
            egui::TextEdit::multiline(draft)
                .desired_rows(4)
                .desired_width(ui.available_width())
                .char_limit(MAX_NOTE),
        );
        count = draft.chars().count();
        response
    });
    ui.label(theme::faint(format!("{count} of {MAX_NOTE} characters")).size(11.5));
    if let Some(typed) = typed
        && !typed.trim().is_empty()
    {
        map.change(editor, ContentEdit::new("Edit a note"), |layout| {
            layout.notes[index].text = typed;
        });
        return;
    }
    ui.add_space(8.0);
    if widgets::danger(ui, "Delete note").clicked()
        && map.change(editor, ContentEdit::new("Delete a note"), |layout| {
            layout.notes.remove(index);
        })
    {
        select(editor, Vec::new());
    }
}
