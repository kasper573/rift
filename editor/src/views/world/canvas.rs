use crate::views::navigation::PX_PER_METER;
use std::collections::{BTreeMap, HashSet};
use ui::chip;
use ui::paint;
use ui::theme;
use ui::words;

use engine::core::content::name_of;
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::world_map::WorldMapDef;
use engine::systems::area::{self, AreaId};
use ui::egui::epaint::QuadraticBezierShape;
use ui::egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    Vec2,
};
use ui::glyph::{self, Mark};
use ui::ink;

use super::atlas::{AtlasSource, AtlasWarp, WorldAtlas};
use super::filters::{self, ColorRanges};
use super::interact::{self, MapDrag};
use super::{
    MAX_ZOOM, MIN_ZOOM, MapLayers, MapPart, MapTileHit, OpenWorldMap, region_color, shows_areas,
};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::tabs::shown_path;
use crate::views::navigation::{self, Viewport};
use crate::views::snap;

const LABEL: f32 = 28.0;
const PAD: f32 = 3.0;
const DOT_STEP: f32 = 32.0;
const EDGE_HIT: f32 = 7.0;
const ROUTE: Color32 = ink::AZURE;
const WARP: Color32 = ink::INK3;
const DIM: Color32 = Color32::from_rgb(120, 112, 96);
const DOT: Color32 = ink::HONEY2;
const POPULATION: Color32 = Color32::from_rgb(240, 140, 60);
const NO_VALUE: Color32 = Color32::from_rgb(110, 110, 120);
const NOTE_PAPER: Color32 = ink::LIGHT;
const NOTE_EDGE: Color32 = ink::HONEY2;
const NOTE_INK: Color32 = ink::INK;
const NOTE_WIDTH: f32 = 188.0;
const NOTE_PAD: f32 = 6.0;
const MINIMAP_CARD: Color32 = ink::INK3;

pub struct MapCard {
    pub area: AreaId,
    pub frame: Rect,
    pub art: Rect,
    pub compact: bool,
    pub caption_width: f32,
}

pub struct MapEdge {
    pub from: AreaId,
    pub to: AreaId,
    pub a: Pos2,
    pub b: Pos2,
    pub out: usize,
    pub back: usize,
}

pub struct MapCanvas {
    pub response: Response,
    pub viewport: Viewport<Meters>,
    pub cards: Vec<MapCard>,
    pub edges: Vec<MapEdge>,
    pub region_labels: Vec<(usize, Rect)>,
    pub notes: Vec<Rect>,
    pub hovered: Option<AreaId>,
    pub typing: bool,
}

impl MapCanvas {
    pub fn card(&self, area: AreaId) -> Option<&MapCard> {
        self.cards.iter().find(|card| card.area == area)
    }

    pub fn card_at(&self, p: Pos2) -> Option<AreaId> {
        topmost_card_at(&self.cards, p)
    }

    pub fn tile_at(&self, atlas: &WorldAtlas, p: Pos2) -> Option<MapTileHit> {
        let card = self.card(self.card_at(p)?)?;
        let size = atlas.size(card.area);
        let rel = (p - card.art.min) / card.art.size().max(Vec2::splat(1.0));
        Some(MapTileHit {
            area: atlas.key(card.area).to_owned(),
            at: Pos::new(rel.x * size.width, rel.y * size.height),
        })
    }

    pub fn edge_at(&self, p: Pos2) -> Option<&MapEdge> {
        self.edges
            .iter()
            .map(|edge| (edge, distance_to_segment(p, edge.a, edge.b)))
            .filter(|(_, distance)| *distance <= EDGE_HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(edge, _)| edge)
    }

    pub fn note_at(&self, p: Pos2) -> Option<usize> {
        self.notes.iter().position(|note| note.contains(p))
    }

    pub fn region_label_at(&self, p: Pos2) -> Option<usize> {
        self.region_labels
            .iter()
            .find(|(_, rect)| rect.contains(p))
            .map(|(index, _)| *index)
    }
}

pub fn show(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let rect = rect.intersect(ui.clip_rect());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CANVAS);
    if !editor.views.world.fitted {
        fit(editor, map, rect);
    }
    let viewport = navigate(editor, ui, &response, rect);
    paint::dot_grid(
        &painter,
        viewport.screen,
        viewport.to_screen(Pos::origin()),
        DOT_STEP,
        DOT,
    );
    let preview = editor
        .views
        .world
        .drag
        .as_ref()
        .and_then(|drag| drag.preview(map));
    let shown = preview.as_ref().unwrap_or(&map.layout);
    let atlas = &map.atlas;
    let cards = place(map, shown, &viewport);
    let edges = edges(atlas, &cards);
    let hovered = response
        .hover_pos()
        .and_then(|p| topmost_card_at(&cards, p));
    let drawing = Drawing {
        map,
        shown,
        overlays: Overlays::of(editor, map),
        selected: super::selected(editor).into_iter().collect(),
        hovered,
        layers: editor.views.world.layers,
        zoomed: shows_areas(viewport.zoom),
    };
    if drawing.layers.regions {
        draw_regions(&painter, &viewport, &drawing);
    }
    if drawing.layers.warps {
        draw_edges(&painter, &edges, &drawing);
    }
    let visible: Vec<&MapCard> = cards
        .iter()
        .filter(|card| card.frame.intersects(rect))
        .collect();
    for card in &visible {
        draw_card(editor, &painter, &drawing, card);
    }
    draw_overlaps(&painter, &visible);
    for card in &visible {
        draw_card_chips(&painter, &drawing, card);
    }
    if drawing.layers.warps
        && let Some(route) = &drawing.overlays.route
    {
        draw_route(&painter, &cards, route);
    }
    if let Some(line) = &editor.views.world.quest_line {
        draw_quest_line(&painter, atlas, &cards, line);
    }
    let (notes, region_labels) = if drawing.layers.regions {
        (
            draw_notes(&painter, &viewport, &drawing),
            draw_region_labels(&painter, &viewport, &drawing),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    if drawing.zoomed {
        draw_edge_tags(&painter, atlas, &cards, rect);
    }
    if let Some(drag) = &editor.views.world.drag {
        draw_drag(&painter, map, &viewport, &cards, drag, response.hover_pos());
    }
    if drawing.layers.warps && !drawing.zoomed {
        legend(&painter, rect);
    }
    minimap(&painter, map, shown, &viewport);
    if drawing
        .overlays
        .lit
        .as_ref()
        .is_some_and(BTreeMap::is_empty)
    {
        painter.text(
            rect.center_top() + Vec2::new(0.0, 8.0),
            Align2::CENTER_TOP,
            "Nothing matches",
            FontId::proportional(14.0),
            theme::DANGER,
        );
    }
    let canvas = MapCanvas {
        typing: ui.ctx().text_edit_focused(),
        response,
        viewport,
        cards,
        edges,
        region_labels,
        notes,
        hovered,
    };
    interact::run(editor, ui, map, &canvas);
    status(editor, map, &canvas, &drawing.overlays);
}

pub fn viridis(t: f32) -> Color32 {
    const STOPS: [[u8; 3]; 9] = [
        [68, 1, 84],
        [72, 36, 117],
        [65, 68, 135],
        [53, 95, 141],
        [42, 120, 142],
        [33, 145, 140],
        [34, 168, 132],
        [122, 209, 81],
        [253, 231, 37],
    ];
    let t = t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let i = (t.floor() as usize).min(STOPS.len() - 2);
    let f = t - i as f32;
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
    Color32::from_rgb(
        mix(STOPS[i][0], STOPS[i + 1][0]),
        mix(STOPS[i][1], STOPS[i + 1][1]),
        mix(STOPS[i][2], STOPS[i + 1][2]),
    )
}

struct Overlays<'a> {
    lit: Option<BTreeMap<AreaId, Vec<AtlasSource>>>,
    colors: Option<ColorRanges>,
    route: Option<Vec<&'a AtlasWarp>>,
}

impl<'a> Overlays<'a> {
    fn of(editor: &Editor, map: &'a OpenWorldMap) -> Overlays<'a> {
        let view = &editor.views.world;
        let atlas = &map.atlas;
        Overlays {
            lit: filters::lit_areas(atlas, &view.lenses),
            colors: view
                .color
                .as_ref()
                .zip(editor.project.as_ref())
                .map(|(color, project)| ColorRanges::of(atlas, &project.files, color)),
            route: view
                .route
                .as_ref()
                .and_then(|route| atlas.route(atlas.id(&route.from)?, atlas.id(&route.to)?)),
        }
    }
}

struct Drawing<'a> {
    map: &'a OpenWorldMap,
    shown: &'a WorldMapDef,
    overlays: Overlays<'a>,
    selected: HashSet<MapPart>,
    hovered: Option<AreaId>,
    layers: MapLayers,
    zoomed: bool,
}

fn draw_card(editor: &mut Editor, painter: &Painter, drawing: &Drawing, card: &MapCard) {
    let Drawing {
        map,
        overlays,
        selected,
        ..
    } = drawing;
    let atlas = &map.atlas;
    let info = &atlas.areas[&card.area];
    let key = &info.key;
    let is_selected = selected.contains(&MapPart::Card(key.clone()));
    let is_lit = overlays
        .lit
        .as_ref()
        .map(|lit| lit.contains_key(&card.area));
    let dimmed = is_lit == Some(false);
    if !drawing.zoomed {
        painter.rect_filled(
            card.frame,
            4.0,
            if dimmed { theme::PANEL } else { theme::RAISED },
        );
    }
    let px_per_tile = card.art.width() / info.size.width.max(1.0);
    painter.rect_filled(card.art, 2.0, theme::INPUT);
    if let Some(project) = editor.project.as_ref()
        && let Some(content) = project.content.clone()
        && let Some(id) = content.by_path::<area::AreaDef>(key)
    {
        let art = editor.views.area.grounds.of_area(
            &painter.ctx().clone(),
            &format!("world:{key}"),
            atlas.generation,
            &content,
            id,
        );
        let canvas = Viewport {
            screen: card.art,
            center: Pos::new(info.size.width / 2.0 - 0.5, info.size.height / 2.0 - 0.5),
            zoom: px_per_tile / PX_PER_METER,
        };
        art.draw(
            &painter.with_clip_rect(card.art),
            canvas,
            if dimmed { DIM } else { Color32::WHITE },
        );
    }
    if let Some(colors) = &overlays.colors
        && let Some(color) = &editor.views.world.color
    {
        match colors.areas.get(&card.area) {
            Some(range) => {
                let tint = viridis(colors.fraction(range.shown(color.show)));
                painter.rect_filled(card.art, 2.0, ink::wash(tint, 0.59));
                chip::floating(
                    painter,
                    card.art.center(),
                    Align2::CENTER_CENTER,
                    &range.text(),
                    theme::RAISED,
                    theme::TEXT,
                    12.0,
                );
            }
            None => paint::hatched(painter, card.art, NO_VALUE, 8.0),
        }
    }
    let on_art = |at: Pos<Meters>| card.art.min + Vec2::new(at.x, at.y) * px_per_tile;
    if drawing.zoomed {
        for door in &info.doors {
            painter.rect_stroke(
                Rect::from_center_size(on_art(door.at), Vec2::splat(px_per_tile.max(4.0))),
                1.0,
                Stroke::new(1.5, ROUTE),
                StrokeKind::Outside,
            );
        }
    }
    if drawing.layers.residents {
        for npc in &info.residents {
            painter.circle(
                on_art(npc.at),
                3.0,
                theme::OK,
                Stroke::new(1.0, Color32::BLACK),
            );
        }
    }
    if drawing.layers.populations {
        for npc in &info.populations {
            let at = on_art(npc.at);
            painter.circle(at, 4.0, POPULATION, Stroke::new(1.0, Color32::BLACK));
            if !card.compact {
                painter.text(
                    at + Vec2::new(5.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("×{}", npc.count),
                    FontId::proportional(10.0),
                    Color32::WHITE,
                );
            }
        }
    }
    if drawing.layers.givers {
        for giver in atlas.givers(card.area) {
            let at = giver.at.map_or(card.art.center(), on_art);
            glyph::alert(painter, at, 6.0);
        }
    }
    if drawing.shown.cards[&card.area].pinned {
        Mark::Pin.paint(
            painter,
            Rect::from_min_size(
                card.art.right_top() + Vec2::new(-14.0, 2.0),
                Vec2::splat(11.0),
            ),
            Color32::WHITE,
        );
    }
    if overlays.route.is_some() {
        let text = atlas
            .distances
            .get(&card.area)
            .map_or_else(|| "–".to_owned(), u32::to_string);
        let at = card.art.center();
        painter.circle(at, 9.0, theme::RAISED, Stroke::new(1.0, theme::BORDER));
        painter.text(
            at,
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(11.0),
            theme::TEXT,
        );
    }
    if !drawing.zoomed {
        draw_caption(painter, atlas, card, dimmed);
    }
    let stroke = if is_selected {
        Stroke::new(2.0, theme::SELECTION)
    } else if is_lit == Some(true) {
        Stroke::new(2.0, theme::WARN)
    } else if drawing.hovered == Some(card.area) {
        Stroke::new(1.0, theme::MUTED)
    } else {
        Stroke::new(1.0, theme::BORDER)
    };
    if !drawing.zoomed || is_selected {
        painter.rect_stroke(card.frame, 4.0, stroke, StrokeKind::Outside);
    }
    if is_selected && selected.len() == 1 {
        for corner in corners(card.frame) {
            painter.rect_filled(
                Rect::from_center_size(corner, Vec2::splat(7.0)),
                1.0,
                theme::SELECTION,
            );
        }
    }
}

fn draw_overlaps(painter: &Painter, cards: &[&MapCard]) {
    for (index, card) in cards.iter().enumerate() {
        for other in &cards[index + 1..] {
            let shared = card.frame.intersect(other.frame);
            if shared.is_positive() {
                paint::hatched(painter, shared, theme::DANGER, 8.0);
            }
        }
    }
}

fn draw_card_chips(painter: &Painter, drawing: &Drawing, card: &MapCard) {
    let tags = drawing
        .overlays
        .lit
        .as_ref()
        .and_then(|lit| lit.get(&card.area))
        .filter(|sources| !sources.is_empty())
        .map(|sources| {
            chip::floating(
                painter,
                card.frame.right_top() + Vec2::new(-4.0, -4.0),
                Align2::RIGHT_BOTTOM,
                &filters::tag_line(sources),
                theme::RAISED,
                theme::WARN,
                11.0,
            )
        });
    if card.area == drawing.map.atlas.initial_spawn {
        let bottom = tags.map_or(card.frame.top() - 6.0, |tags| tags.top() - 2.0);
        chip::floating(
            painter,
            Pos2::new(card.frame.left(), bottom),
            Align2::LEFT_BOTTOM,
            "initial spawn",
            theme::PANEL,
            theme::OK,
            11.0,
        );
    }
}

pub fn corners(rect: Rect) -> [Pos2; 4] {
    [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ]
}

fn draw_caption(painter: &Painter, atlas: &WorldAtlas, card: &MapCard, dimmed: bool) {
    let name = atlas.file_name(card.area);
    let ink = if dimmed { theme::FAINT } else { theme::TEXT };
    let top = card.art.max.y + 2.0;
    let width = card.caption_width - 2.0 * PAD;
    let line = |text: &str, font: FontId, color: Color32, y: f32| {
        paint::text_within(
            painter,
            Pos2::new(card.frame.center().x, y),
            Align2::CENTER_TOP,
            text,
            font,
            color,
            width,
        );
    };
    if card.compact {
        for (i, part) in split_name(name).iter().enumerate() {
            line(part, FontId::monospace(10.0), ink, top + 10.0 * i as f32);
        }
        return;
    }
    line(name, FontId::monospace(11.0), ink, top);
    let ways = atlas
        .exits(card.area)
        .iter()
        .filter(|to| **to != card.area)
        .count();
    line(
        &match ways {
            0 => "no way out".to_owned(),
            1 => "1 way out".to_owned(),
            n => format!("{n} ways out"),
        },
        FontId::proportional(10.0),
        theme::FAINT,
        top + 12.0,
    );
}

fn place(map: &OpenWorldMap, layout: &WorldMapDef, viewport: &Viewport<Meters>) -> Vec<MapCard> {
    let mut cards = layout
        .cards
        .iter()
        .map(|(&area, card)| {
            let art = viewport.rect(navigation::rect_from_egui(map.art_rect(area, card)));
            if shows_areas(viewport.zoom) {
                return MapCard {
                    area,
                    frame: art,
                    art,
                    compact: false,
                    caption_width: 0.0,
                };
            }
            let outer = viewport.rect(navigation::rect_from_egui(map.card_rect(area, card)));
            let room = Vec2::new(
                outer.width() - 2.0 * PAD,
                outer.height() - LABEL - 2.0 * PAD,
            );
            let scale = (room.x / art.width().max(1.0))
                .min(room.y / art.height().max(1.0))
                .min(1.0);
            let shown = art.size() * scale;
            let fitted = Rect::from_min_size(
                Pos2::new(
                    outer.center().x - shown.x / 2.0,
                    outer.min.y + PAD + (room.y - shown.y) / 2.0,
                ),
                shown,
            );
            MapCard {
                area,
                frame: outer,
                art: fitted,
                compact: fitted.width() < 64.0 || fitted.height() < 30.0,
                caption_width: outer.width(),
            }
        })
        .collect::<Vec<_>>();
    let widths: Vec<f32> = cards
        .iter()
        .map(|card| caption_width(card, &cards))
        .collect();
    for (card, width) in cards.iter_mut().zip(widths) {
        card.caption_width = width;
    }
    cards
}

fn caption_width(card: &MapCard, cards: &[MapCard]) -> f32 {
    let frame = card.frame;
    let band = card.art.max.y..=frame.max.y;
    let centre = frame.center().x;
    let reach = cards
        .iter()
        .filter(|other| other.area != card.area)
        .map(|other| other.frame)
        .filter(|other| other.max.y >= *band.start() && other.min.y <= *band.end())
        .filter_map(|other| {
            if other.min.x >= frame.max.x {
                Some((frame.max.x + other.min.x) / 2.0 - centre)
            } else if other.max.x <= frame.min.x {
                Some(centre - (frame.min.x + other.max.x) / 2.0)
            } else {
                None
            }
        })
        .fold(f32::INFINITY, f32::min);
    (2.0 * reach).max(frame.width())
}

fn edges(atlas: &WorldAtlas, cards: &[MapCard]) -> Vec<MapEdge> {
    let card = |area: AreaId| cards.iter().find(|card| card.area == area);
    let mut drawn: HashSet<(AreaId, AreaId)> = HashSet::new();
    let mut out = Vec::new();
    for warp in &atlas.warps {
        if warp.from == warp.to
            || drawn.contains(&(warp.to, warp.from))
            || !drawn.insert((warp.from, warp.to))
        {
            continue;
        }
        let (Some(from), Some(to)) = (card(warp.from), card(warp.to)) else {
            continue;
        };
        out.push(MapEdge {
            from: warp.from,
            to: warp.to,
            a: edge_point(from.frame, to.frame.center()),
            b: edge_point(to.frame, from.frame.center()),
            out: atlas.warps_between(warp.from, warp.to).count(),
            back: atlas.warps_between(warp.to, warp.from).count(),
        });
    }
    out
}

fn status(editor: &mut Editor, map: &OpenWorldMap, canvas: &MapCanvas, overlays: &Overlays) {
    let view = &editor.views.world;
    let atlas = &map.atlas;
    let left = match &view.drag {
        Some(MapDrag::Cards {
            moved: true,
            cards,
            snap,
            ..
        }) => vec![
            format!(
                "moving {} card{}{}",
                cards.len(),
                words::plural(cards.len()),
                snap.as_ref()
                    .map(|snap| format!(" · {snap}"))
                    .unwrap_or_default()
            ),
            format!(
                "{} moves without snapping · {} pins",
                EditorKey::Unsnapped.name(),
                EditorKey::Pin.name()
            ),
        ],
        Some(MapDrag::Resize { area, size, .. }) => {
            let tiles = atlas.id(area).map(|id| atlas.size(id)).unwrap_or_default();
            vec![
                format!(
                    "resizing {area} · {size}% · {} × {} tiles",
                    tiles.width, tiles.height
                ),
                format!(
                    "{} steps by 1% · {} cancels",
                    EditorKey::FineStep.name(),
                    EditorKey::Cancel.name()
                ),
            ]
        }
        _ if shows_areas(canvas.viewport.zoom) => {
            let zoom = canvas.viewport.zoom * 100.0;
            let hover = canvas
                .response
                .hover_pos()
                .and_then(|p| canvas.tile_at(atlas, p));
            match hover {
                Some(hit) => vec![
                    format!(
                        "World · zoom {zoom:.0}% · {} {}, {}",
                        hit.area,
                        hit.at.x.floor(),
                        hit.at.y.floor()
                    ),
                    tile_facts(map, &hit),
                    "right-click a tile to play there".to_owned(),
                ],
                None => vec![
                    format!("World · zoom {zoom:.0}%"),
                    "right-click a tile to play there".to_owned(),
                ],
            }
        }
        _ => overview_status(editor, map, overlays),
    };
    editor.status.left = left;
}

fn overview_status(editor: &Editor, map: &OpenWorldMap, overlays: &Overlays) -> Vec<String> {
    let view = &editor.views.world;
    let atlas = &map.atlas;
    if let Some(route) = &view.route {
        return vec![match &overlays.route {
            Some(hops) => format!(
                "World · route {} → {} · {} warps",
                name_of(&route.from),
                name_of(&route.to),
                hops.len()
            ),
            None => format!(
                "World · route {} → {} · no chain of warps leads there",
                name_of(&route.from),
                name_of(&route.to)
            ),
        }];
    }
    if let Some(line) = &view.quest_line {
        let quests = atlas
            .lines
            .iter()
            .find(|known| known.category == *line)
            .map_or(0, |known| known.quests.len());
        let areas = atlas.line_areas(line).len();
        return vec![format!(
            "World · quest line {line} · {areas} areas, {quests} quests"
        )];
    }
    if let (Some(color), Some(_)) = (&view.color, &overlays.colors) {
        return vec![format!("World · {}", color.sentence())];
    }
    if let (Some(lit), Some(lens)) = (&overlays.lit, view.lenses.first()) {
        let sources: usize = lit.values().map(Vec::len).sum();
        return vec![format!(
            "World · {} · {sources} sources in {} areas",
            lens.chip(),
            lit.len()
        )];
    }
    vec![
        world_summary(atlas),
        format!(
            "drag to arrange · scroll to zoom · {} drag to pan",
            EditorKey::Pan.name()
        ),
    ]
}

fn fit(editor: &mut Editor, map: &OpenWorldMap, rect: Rect) {
    let view = &mut editor.views.world;
    match map.bounds(&map.layout) {
        Some(bounds) => {
            view.center = navigation::pos_from_egui(bounds.center());
            view.zoom = navigation::fit_zoom(
                rect.size() - Vec2::splat(80.0),
                Vec2::new(bounds.width(), bounds.height()),
                PX_PER_METER,
            )
            .clamp(MIN_ZOOM, MAX_ZOOM);
        }
        None => {
            view.center = Pos::origin();
            view.zoom = 0.1;
        }
    }
    view.fitted = true;
}

fn navigate(editor: &mut Editor, ui: &Ui, response: &Response, rect: Rect) -> Viewport<Meters> {
    let view = &mut editor.views.world;
    let mut viewport = Viewport {
        screen: rect,
        center: view.center,
        zoom: view.zoom,
    };
    navigation::zoom_and_pan(ui, response, &mut viewport, (MIN_ZOOM, MAX_ZOOM));
    view.center = viewport.center;
    view.zoom = viewport.zoom;
    viewport
}

fn split_name(name: &str) -> Vec<&str> {
    match name.rfind('_') {
        Some(at) if name.chars().count() > 12 => vec![&name[..=at], &name[at + 1..]],
        _ => vec![name],
    }
}

fn edge_point(rect: Rect, toward: Pos2) -> Pos2 {
    let c = rect.center();
    let d = toward - c;
    if d == Vec2::ZERO {
        return c;
    }
    let reach = |half: f32, along: f32| {
        if along.abs() > 1e-6 {
            half / along.abs()
        } else {
            f32::INFINITY
        }
    };
    c + d * reach(rect.width() / 2.0, d.x).min(reach(rect.height() / 2.0, d.y))
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() <= f32::EPSILON {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}

fn draw_regions(painter: &Painter, viewport: &Viewport<Meters>, drawing: &Drawing) {
    let layout = drawing.shown;
    for (index, region) in layout.regions.iter().enumerate() {
        let Some(r) = drawing.map.region_rect(layout, region) else {
            continue;
        };
        let screen = viewport.rect(navigation::rect_from_egui(r));
        let lit = drawing.selected.contains(&MapPart::Region(index));
        let color = region_color(region.color);
        painter.rect_filled(screen, 6.0, ink::wash(color, 0.05));
        let stroke = if lit {
            Stroke::new(2.0, theme::SELECTION)
        } else {
            Stroke::new(1.0, color)
        };
        paint::dashed_rect(painter, screen, stroke, 6.0, 4.0);
    }
}

fn draw_region_labels(
    painter: &Painter,
    viewport: &Viewport<Meters>,
    drawing: &Drawing,
) -> Vec<(usize, Rect)> {
    let layout = drawing.shown;
    layout
        .regions
        .iter()
        .enumerate()
        .filter_map(|(index, region)| {
            let screen = viewport.rect(navigation::rect_from_egui(
                drawing.map.region_rect(layout, region)?,
            ));
            let label = chip::floating(
                painter,
                screen.left_top() + Vec2::new(8.0, -2.0),
                Align2::LEFT_BOTTOM,
                &region.name,
                theme::PANEL,
                region_color(region.color),
                11.0,
            );
            Some((index, label))
        })
        .collect()
}

fn draw_edges(painter: &Painter, edges: &[MapEdge], drawing: &Drawing) {
    let atlas = &drawing.map.atlas;
    let route = &drawing.overlays.route;
    let joins = |from: AreaId, to: AreaId, a: AreaId, b: AreaId| {
        (from == a && to == b) || (from == b && to == a)
    };
    for edge in edges {
        let (a, b) = (edge.a, edge.b);
        let on_route = route
            .iter()
            .flatten()
            .any(|hop| joins(hop.from, hop.to, edge.from, edge.to));
        let is_selected = drawing.selected.iter().any(|part| {
            matches!(part, MapPart::Edge(x, y) if atlas.id(x).zip(atlas.id(y)).is_some_and(|(x, y)| joins(x, y, edge.from, edge.to)))
        });
        let both = edge.back > 0;
        let stroke = if is_selected {
            Stroke::new(2.5, theme::SELECTION)
        } else if on_route {
            Stroke::new(3.0, ROUTE)
        } else if both {
            Stroke::new(1.5, WARP)
        } else {
            Stroke::new(1.5, theme::WARN)
        };
        if both {
            painter.line_segment([a, b], stroke);
            glyph::arrow_head(painter, a, a - b, 8.0, stroke.color);
        } else {
            painter.add(Shape::dashed_line(&[a, b], stroke, 6.0, 4.0));
        }
        glyph::arrow_head(painter, b, b - a, 8.0, stroke.color);
        if (a - b).length() >= 70.0 {
            let (fill, ink) = if is_selected {
                (theme::ACCENT_SOFT, theme::TEXT)
            } else {
                (theme::PANEL, theme::MUTED)
            };
            chip::floating(
                painter,
                a + (b - a) / 2.0,
                Align2::CENTER_CENTER,
                &format!("{} · {}", edge.out, edge.back),
                fill,
                ink,
                10.0,
            );
        }
    }
}

fn draw_route(painter: &Painter, cards: &[MapCard], route: &[&AtlasWarp]) {
    let card = |area: AreaId| cards.iter().find(|card| card.area == area);
    let stroke = Stroke::new(3.0, ROUTE);
    for hop in route {
        let (Some(from), Some(to)) = (card(hop.from), card(hop.to)) else {
            continue;
        };
        let a = edge_point(from.frame, to.frame.center());
        let b = edge_point(to.frame, from.frame.center());
        painter.line_segment([a, b], stroke);
        glyph::arrow_head(painter, b, b - a, 8.0, stroke.color);
    }
}

fn draw_quest_line(painter: &Painter, atlas: &WorldAtlas, cards: &[MapCard], category: &str) {
    let card = |area: AreaId| cards.iter().find(|card| card.area == area);
    let stroke = Stroke::new(2.0, theme::WARN);
    let mut used: BTreeMap<AreaId, usize> = BTreeMap::new();
    let mut previous: Vec<Pos2> = Vec::new();
    for step in atlas.line_steps(category) {
        let mut here = Vec::new();
        for &area in &step.areas {
            let Some(card) = card(area) else {
                continue;
            };
            let slot = used.entry(area).or_default();
            let at = card.art.left_bottom()
                + Vec2::new(
                    12.0 + 22.0 * (*slot % 4) as f32,
                    -12.0 - 22.0 * (*slot / 4) as f32,
                );
            *slot += 1;
            here.push(at);
            line_badge(painter, at, &step.badge, step.branch);
        }
        if step.branch {
            continue;
        }
        for from in &previous {
            for to in here.iter().filter(|to| (**to - *from).length() > 30.0) {
                let bend = from.lerp(*to, 0.5) + Vec2::new(0.0, -40.0);
                let points = QuadraticBezierShape::from_points_stroke(
                    [*from, bend, *to],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::NONE,
                )
                .flatten(Some(1.0));
                painter.add(Shape::dashed_line(&points, stroke, 6.0, 5.0));
                glyph::arrow_head(painter, *to, *to - bend, 8.0, stroke.color);
            }
        }
        if !here.is_empty() {
            previous = here;
        }
    }
}

pub fn line_badge(painter: &Painter, at: Pos2, badge: &str, branch: bool) {
    let (fill, ink) = if branch {
        (theme::DANGER, ink::LIGHT)
    } else {
        (theme::WARN, ink::LIGHT)
    };
    painter.circle(at, 9.0, fill, Stroke::new(1.0, ink::INK));
    painter.text(
        at,
        Align2::CENTER_CENTER,
        badge,
        FontId::proportional(11.0),
        ink,
    );
}

fn draw_notes(painter: &Painter, viewport: &Viewport<Meters>, drawing: &Drawing) -> Vec<Rect> {
    drawing
        .shown
        .notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let galley = painter.layout(
                note.text.clone(),
                FontId::proportional(12.0),
                NOTE_INK,
                NOTE_WIDTH,
            );
            let at = viewport.to_screen(Pos::new(note.x, note.y));
            let paper = Rect::from_min_size(at, galley.size() + Vec2::splat(2.0 * NOTE_PAD));
            let stroke = if drawing.selected.contains(&MapPart::Note(index)) {
                Stroke::new(2.0, theme::SELECTION)
            } else {
                Stroke::new(1.0, NOTE_EDGE)
            };
            painter.rect_filled(paper, 3.0, NOTE_PAPER);
            painter.rect_stroke(paper, 3.0, stroke, StrokeKind::Outside);
            painter.galley(paper.min + Vec2::splat(NOTE_PAD), galley, NOTE_INK);
            paper
        })
        .collect()
}

fn draw_edge_tags(painter: &Painter, atlas: &WorldAtlas, cards: &[MapCard], rect: Rect) {
    let centre = rect.center();
    let Some(card) = cards
        .iter()
        .filter(|card| card.frame.intersects(rect))
        .min_by(|a, b| {
            (a.frame.center() - centre)
                .length()
                .total_cmp(&(b.frame.center() - centre).length())
        })
    else {
        return;
    };
    chip::floating(
        painter,
        card.frame.left_bottom() + Vec2::new(0.0, 4.0),
        Align2::LEFT_TOP,
        atlas.file_name(card.area),
        theme::PANEL,
        theme::TEXT,
        11.0,
    );
    let origin = card.frame.center().clamp(rect.min, rect.max);
    let inset = rect.shrink2(Vec2::new(70.0, 14.0));
    for &to in atlas.exits(card.area) {
        let Some(other) = cards.iter().find(|other| other.area == to) else {
            continue;
        };
        if other.frame.intersects(rect) {
            continue;
        }
        let d = other.frame.center() - origin;
        let reach = |along: f32, low: f32, high: f32, from: f32| {
            if along.abs() > 1e-3 {
                ((if along > 0.0 { high } else { low }) - from) / along
            } else {
                f32::INFINITY
            }
        };
        let tx = reach(d.x, inset.min.x, inset.max.x, origin.x);
        let ty = reach(d.y, inset.min.y, inset.max.y, origin.y);
        let at = (origin + d * tx.min(ty).max(0.0)).clamp(inset.min, inset.max);
        let name = atlas.file_name(to);
        let (text, anchor) = if tx < ty {
            if d.x > 0.0 {
                (format!("{name} →"), Align2::RIGHT_CENTER)
            } else {
                (format!("← {name}"), Align2::LEFT_CENTER)
            }
        } else {
            if d.y > 0.0 {
                (format!("↓ {name}"), Align2::CENTER_BOTTOM)
            } else {
                (format!("↑ {name}"), Align2::CENTER_TOP)
            }
        };
        chip::floating(painter, at, anchor, &text, theme::PANEL, theme::TEXT, 11.0);
    }
}

fn draw_drag(
    painter: &Painter,
    map: &OpenWorldMap,
    viewport: &Viewport<Meters>,
    cards: &[MapCard],
    drag: &MapDrag,
    pointer: Option<Pos2>,
) {
    match drag {
        MapDrag::Box { from } => {
            let Some(p) = pointer else {
                return;
            };
            let r = Rect::from_two_pos(*from, p);
            painter.rect_filled(r, 0.0, ink::wash(theme::SELECTION, 0.12));
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, theme::SELECTION),
                StrokeKind::Outside,
            );
        }
        MapDrag::Resize {
            area,
            start_size,
            size,
            ..
        } => {
            let Some(id) = map.atlas.id(area) else {
                return;
            };
            let (Some(card), Some(stored)) = (
                cards.iter().find(|card| card.area == id),
                map.layout.cards.get(&id),
            ) else {
                return;
            };
            let ghost = viewport.rect(navigation::rect_from_egui(map.art_rect(id, stored)));
            paint::dashed_rect(painter, ghost, Stroke::new(1.0, theme::MUTED), 4.0, 3.0);
            let tiles = map.atlas.size(id);
            chip::floating(
                painter,
                card.frame.left_bottom() + Vec2::new(0.0, 6.0),
                Align2::LEFT_TOP,
                &format!("{size}% · {} × {} tiles", tiles.width, tiles.height),
                theme::ACCENT_SOFT,
                Color32::WHITE,
                11.0,
            );
            chip::floating(
                painter,
                ghost.right_bottom() + Vec2::new(8.0, 6.0),
                Align2::LEFT_TOP,
                &format!("was {start_size}%"),
                theme::PANEL,
                theme::MUTED,
                11.0,
            );
        }
        MapDrag::Cards {
            snap: Some(snap), ..
        } => {
            if let Some(p) = pointer {
                chip::floating(
                    painter,
                    p + Vec2::new(16.0, 16.0),
                    Align2::LEFT_TOP,
                    snap,
                    theme::ACCENT_SOFT,
                    Color32::WHITE,
                    11.0,
                );
            }
        }
        MapDrag::Cards { .. } | MapDrag::Note { .. } => {}
    }
}

fn legend(painter: &Painter, rect: Rect) {
    let r = Rect::from_min_size(
        rect.left_bottom() + Vec2::new(12.0, -62.0),
        Vec2::new(160.0, 50.0),
    );
    painter.rect_filled(r, 4.0, theme::PANEL);
    painter.rect_stroke(r, 4.0, Stroke::new(1.0, theme::BORDER), StrokeKind::Outside);
    let entry = |top: f32, stroke: Stroke, both: bool, text: &str| {
        let a = r.min + Vec2::new(12.0, top);
        let b = a + Vec2::new(30.0, 0.0);
        if both {
            painter.line_segment([a, b], stroke);
            glyph::arrow_head(painter, a, a - b, 8.0, stroke.color);
        } else {
            painter.add(Shape::dashed_line(&[a, b], stroke, 5.0, 3.0));
        }
        glyph::arrow_head(painter, b, b - a, 8.0, stroke.color);
        painter.text(
            b + Vec2::new(10.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            FontId::monospace(11.0),
            theme::MUTED,
        );
    };
    entry(16.0, Stroke::new(1.5, WARP), true, "warps both ways");
    entry(36.0, Stroke::new(1.5, theme::WARN), false, "one way");
}

fn minimap(
    painter: &Painter,
    map: &OpenWorldMap,
    layout: &WorldMapDef,
    viewport: &Viewport<Meters>,
) {
    let Some(bounds) = map.bounds(layout) else {
        return;
    };
    let rect = viewport.screen;
    let view = navigation::rect_to_egui(viewport.visible());
    if view.contains_rect(bounds) {
        return;
    }
    let bounds = bounds.union(view);
    let box_rect = Rect::from_min_size(
        rect.right_bottom() - Vec2::new(192.0, 132.0),
        Vec2::new(180.0, 120.0),
    );
    ui::slip::paint(painter, box_rect);
    let scale = ((box_rect.width() - 8.0) / bounds.width().max(1.0))
        .min((box_rect.height() - 8.0) / bounds.height().max(1.0));
    let to_box = |p: Pos2| box_rect.center() + (p - bounds.center()) * scale;
    for (area, card) in &layout.cards {
        let r = map.card_rect(*area, card);
        painter.rect_filled(
            Rect::from_min_max(to_box(r.min), to_box(r.max)).expand(1.0),
            0.0,
            MINIMAP_CARD,
        );
    }
    painter.rect_stroke(
        Rect::from_min_max(to_box(view.min), to_box(view.max)),
        0.0,
        Stroke::new(1.0, theme::WARN),
        StrokeKind::Outside,
    );
}

fn topmost_card_at(cards: &[MapCard], p: Pos2) -> Option<AreaId> {
    cards
        .iter()
        .filter(|card| card.frame.contains(p))
        .min_by(|a, b| a.frame.area().total_cmp(&b.frame.area()))
        .map(|card| card.area)
}

fn world_summary(atlas: &WorldAtlas) -> String {
    let one_way = atlas
        .warps
        .iter()
        .filter(|warp| !atlas.both_ways(warp))
        .count();
    let warps = match one_way {
        0 => format!("{} warps, each with a way back", atlas.warps.len()),
        _ => format!("{} warps, {one_way} one way", atlas.warps.len()),
    };
    let unreached = atlas
        .areas
        .keys()
        .filter(|area| !atlas.distances.contains_key(area))
        .count();
    let reach = match unreached {
        0 => format!(
            "every area at most {} warps from the initial spawn",
            atlas.distances.values().copied().max().unwrap_or(0)
        ),
        _ => format!("{unreached} areas unreached from the initial spawn"),
    };
    let residents: usize = atlas.areas.values().map(|info| info.residents.len()).sum();
    let populations: usize = atlas
        .areas
        .values()
        .map(|info| info.populations.len())
        .sum();
    format!(
        "World · {} areas · {warps} · {reach} · {residents} residents · {populations} populations",
        atlas.areas.len()
    )
}

fn tile_facts(map: &OpenWorldMap, hit: &MapTileHit) -> String {
    let atlas = &map.atlas;
    let Some(id) = atlas.id(&hit.area) else {
        return String::new();
    };
    let info = &atlas.areas[&id];
    let cell = snap(hit.at);
    let loaded = area::load(&map.content, id);
    let walkable = if loaded.mesh.walkable(cell) {
        "walkable"
    } else {
        "not walkable"
    };
    let safe = if loaded.safe_zones.iter().any(|zone| zone.contains(cell)) {
        "in a safe zone"
    } else {
        "no safe zone"
    };
    let nearest_npc = info
        .residents
        .iter()
        .chain(&info.populations)
        .map(|npc| (npc.key.as_str(), npc.at.distance(cell).0))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or_else(
            || "no npc placed".to_owned(),
            |(npc, metres)| format!("{} {} m away", shown_path(npc), metres.round()),
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
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or_else(
            || "no door".to_owned(),
            |(door, metres)| format!("door {door} {} m away", metres.round()),
        );
    format!("{walkable} · {safe} · {nearest_npc} · {nearest_door}")
}
