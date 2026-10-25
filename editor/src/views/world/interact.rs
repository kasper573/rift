use engine::systems::area::world_map::{MAX_SIZE, MIN_SIZE, WorldMapDef};
use engine::systems::area::{self, AreaId};
use engine::systems::script::Place;
use ui::egui::{Modifiers, PointerButton, Pos2, Rect, Ui, Vec2};
use ui::menu;

use super::canvas::{MapCanvas, corners};
use super::{
    GAP, MapPart, MapRoute, MapTileHit, OpenWorldMap, card_at, hundredths, move_card, select,
    selected, tidy_regions,
};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::play;
use crate::project::ContentEdit;
use crate::shell::widgets;
use crate::views::snap;
use crate::views::{area as area_view, navigation};

const SNAP_PX: f32 = 8.0;
const DRAG_SLOP: f32 = 3.0;
const CORNER_HIT: f32 = 8.0;
const SIZE_STEP: f32 = 5.0;

pub enum MapDrag {
    Cards {
        from: Pos2,
        cards: Vec<String>,
        moved: bool,
        offset: Vec2,
        snap: Option<String>,
    },
    Note {
        index: usize,
        from: Pos2,
        offset: Vec2,
    },
    Resize {
        area: String,
        start_size: u32,
        size: u32,
        from: Pos2,
        center: Pos2,
    },
    Box {
        from: Pos2,
    },
}

impl MapDrag {
    pub fn preview(&self, map: &OpenWorldMap) -> Option<WorldMapDef> {
        let mut layout = map.layout.clone();
        match self {
            MapDrag::Cards {
                cards,
                moved: true,
                offset,
                ..
            } => {
                for id in cards.iter().filter_map(|key| map.atlas.id(key)) {
                    if let Some(card) = layout.cards.get_mut(&id) {
                        move_card(card, card_at(card) + *offset);
                    }
                }
            }
            MapDrag::Note { index, offset, .. } => {
                let note = layout.notes.get_mut(*index)?;
                note.x = hundredths(note.x + offset.x);
                note.y = hundredths(note.y + offset.y);
            }
            MapDrag::Resize { area, size, .. } => {
                layout.cards.get_mut(&map.atlas.id(area)?)?.size = *size;
            }
            MapDrag::Cards { .. } | MapDrag::Box { .. } => return None,
        }
        Some(layout)
    }

    fn follow(&mut self, map: &OpenWorldMap, p: Pos2, scale: f32, modifiers: Modifiers) {
        match self {
            MapDrag::Cards {
                from,
                cards,
                moved,
                offset,
                snap,
            } => {
                *moved |= (p - *from).length() > DRAG_SLOP;
                let delta = (p - *from) / scale;
                (*offset, *snap) = if EditorKey::Unsnapped.held_among(modifiers) {
                    (delta, None)
                } else {
                    snapped(map, cards, delta, scale)
                };
            }
            MapDrag::Note { from, offset, .. } => *offset = (p - *from) / scale,
            MapDrag::Resize {
                start_size,
                size,
                from,
                center,
                ..
            } => {
                let raw = *start_size as f32 * (p - *center).length()
                    / (*from - *center).length().max(1.0);
                let stepped = if EditorKey::FineStep.held_among(modifiers) {
                    raw.round()
                } else {
                    (raw / SIZE_STEP).round() * SIZE_STEP
                };
                *size = (stepped as u32).clamp(MIN_SIZE, MAX_SIZE);
            }
            MapDrag::Box { .. } => {}
        }
    }

    fn label(&self) -> String {
        match self {
            MapDrag::Cards { cards, .. } => match cards.as_slice() {
                [one] => format!("Move {one} on the world map"),
                _ => format!("Move {} world map cards", cards.len()),
            },
            MapDrag::Note { .. } => "Move a note".to_owned(),
            MapDrag::Resize { area, .. } => format!("Resize {area} on the world map"),
            MapDrag::Box { .. } => "Select on the world map".to_owned(),
        }
    }
}

pub fn run(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, canvas: &MapCanvas) {
    let response = &canvas.response;
    let modifiers = ui.input(|i| i.modifiers);
    let space = ui.input(|i| EditorKey::Pan.held(i));
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos());
    if response.drag_started_by(PointerButton::Primary) && !space {
        let from = ui
            .input(|i| i.pointer.press_origin())
            .or(pointer)
            .unwrap_or_default();
        editor.views.world.drag = start_drag(editor, map, canvas, from);
    }
    if response.dragged_by(PointerButton::Primary)
        && let Some(p) = pointer
        && let Some(drag) = &mut editor.views.world.drag
    {
        drag.follow(map, p, canvas.viewport.scale(), modifiers);
    }
    if response.drag_stopped()
        && let Some(drag) = editor.views.world.drag.take()
    {
        finish_drag(editor, map, canvas, drag, pointer, modifiers);
    }
    if response.clicked_by(PointerButton::Primary) && !space {
        click(editor, map, canvas, modifiers);
    }
    if response.double_clicked()
        && let Some(area) = canvas.hovered
    {
        area_view::open_area_at(editor, map.atlas.key(area), None, None);
    }
    let hover = response
        .hover_pos()
        .and_then(|p| canvas.tile_at(&map.atlas, p));
    editor.views.world.hover = hover.clone();
    if response.secondary_clicked() {
        editor.views.world.menu_target = hover.clone();
    }
    let menu_target = editor.views.world.menu_target.clone();
    response.context_menu(|ui| {
        ui.set_min_width(240.0);
        match &menu_target {
            Some(hit) => card_menu(editor, ui, map, hit),
            None => canvas_menu(editor, ui, map, canvas),
        }
    });
    if response.hovered() && !canvas.typing {
        keys(editor, ui, map, canvas, hover);
    }
}

fn start_drag(
    editor: &mut Editor,
    map: &OpenWorldMap,
    canvas: &MapCanvas,
    from: Pos2,
) -> Option<MapDrag> {
    let atlas = &map.atlas;
    let selected = selected(editor);
    if let Some((area, center)) = corner_at(map, canvas, &selected, from) {
        let size = map.layout.cards[&atlas.id(&area)?].size;
        return Some(MapDrag::Resize {
            area,
            start_size: size,
            size,
            from,
            center,
        });
    }
    if let Some(index) = canvas.note_at(from) {
        select(editor, vec![MapPart::Note(index)]);
        return Some(MapDrag::Note {
            index,
            from,
            offset: Vec2::ZERO,
        });
    }
    if let Some(area) = canvas.card_at(from) {
        let part = MapPart::Card(atlas.key(area).to_owned());
        if !selected.contains(&part) {
            select(editor, vec![part]);
        }
        return Some(cards_drag(editor, map, from));
    }
    if let Some(edge) = canvas.edge_at(from) {
        let part = MapPart::Edge(
            atlas.key(edge.from).to_owned(),
            atlas.key(edge.to).to_owned(),
        );
        select(editor, vec![part]);
        return None;
    }
    if let Some(index) = canvas.region_label_at(from) {
        let parts = map.layout.regions[index]
            .cards
            .iter()
            .map(|area| MapPart::Card(atlas.key(*area).to_owned()))
            .collect();
        select(editor, parts);
        return Some(cards_drag(editor, map, from));
    }
    Some(MapDrag::Box { from })
}

fn cards_drag(editor: &Editor, map: &OpenWorldMap, from: Pos2) -> MapDrag {
    MapDrag::Cards {
        from,
        cards: map
            .card_ids(&selected(editor))
            .into_iter()
            .map(|area| map.atlas.key(area).to_owned())
            .collect(),
        moved: false,
        offset: Vec2::ZERO,
        snap: None,
    }
}

fn finish_drag(
    editor: &mut Editor,
    map: &OpenWorldMap,
    canvas: &MapCanvas,
    drag: MapDrag,
    pointer: Option<Pos2>,
    modifiers: Modifiers,
) {
    if let MapDrag::Box { from } = drag {
        let Some(p) = pointer else {
            return;
        };
        let region = Rect::from_two_pos(from, p);
        let mut parts: Vec<MapPart> = canvas
            .cards
            .iter()
            .filter(|card| region.intersects(card.frame))
            .map(|card| MapPart::Card(map.atlas.key(card.area).to_owned()))
            .collect();
        if EditorKey::ExtendSelection.held_among(modifiers)
            || EditorKey::ToggleSelection.held_among(modifiers)
        {
            for part in selected(editor) {
                if !parts.contains(&part) {
                    parts.push(part);
                }
            }
        }
        select(editor, parts);
        return;
    }
    if let Some(next) = drag.preview(map) {
        map.write(editor, ContentEdit::new(drag.label()), &next);
    }
}

fn click(editor: &mut Editor, map: &OpenWorldMap, canvas: &MapCanvas, modifiers: Modifiers) {
    let p = canvas.response.interact_pointer_pos().unwrap_or_default();
    if EditorKey::Route.held_among(modifiers)
        && let Some(area) = canvas.hovered
    {
        let key = map.atlas.key(area).to_owned();
        match editor.views.world.route_from.take() {
            Some(from) if from != key => {
                editor.views.world.route = Some(MapRoute { from, to: key });
            }
            _ => start_route(editor, key),
        }
        return;
    }
    let toggle = EditorKey::ToggleSelection.held_among(modifiers);
    let extend = toggle || EditorKey::ExtendSelection.held_among(modifiers);
    let part = canvas.note_at(p).map(MapPart::Note).or_else(|| {
        canvas
            .hovered
            .map(|area| MapPart::Card(map.atlas.key(area).to_owned()))
    });
    match part {
        Some(part) => {
            let mut next = if extend { selected(editor) } else { Vec::new() };
            if toggle && next.contains(&part) {
                next.retain(|known| *known != part);
            } else if !next.contains(&part) {
                next.push(part);
            }
            select(editor, next);
        }
        None if !extend => {
            let region = map.layout.regions.iter().position(|region| {
                map.region_rect(&map.layout, region).is_some_and(|rect| {
                    canvas
                        .viewport
                        .rect(navigation::rect_from_egui(rect))
                        .contains(p)
                })
            });
            select(editor, region.map(MapPart::Region).into_iter().collect());
        }
        None => {}
    }
}

fn start_route(editor: &mut Editor, from: String) {
    editor.toast(format!(
        "route from {from} · {} click where it goes",
        EditorKey::Route.name()
    ));
    editor.views.world.route_from = Some(from);
}

fn card_menu(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, hit: &MapTileHit) {
    let Some(id) = map.atlas.id(&hit.area) else {
        return;
    };
    let cell = snap(hit.at);
    let place = Place { area: id, at: cell };
    if area::load(&map.content, id).mesh.walkable(cell) {
        let character = play::current_character(editor).unwrap_or_else(|| "…".to_owned());
        if ui
            .add(widgets::shortcut_button(
                format!("Play here as {character}"),
                EditorKey::PlayHere,
            ))
            .clicked()
        {
            play::start(editor, Some(place));
            ui.close();
        }
        menu::submenu(ui, "Play here as…", |ui| {
            for test in play::test_characters(editor) {
                if ui
                    .button(format!("{} · level {}", test.key, test.level))
                    .clicked()
                {
                    editor.play.character = Some(test.key);
                    play::start(editor, Some(place));
                    ui.close();
                }
            }
        });
        ui.separator();
    }
    if ui
        .add(widgets::shortcut_button(
            format!("Open {} here", hit.area),
            EditorKey::Confirm,
        ))
        .clicked()
    {
        area_view::open_area_at(editor, &hit.area, Some(cell), None);
        ui.close();
    }
    if ui
        .button(format!("Copy position {}, {}", cell.x, cell.y))
        .clicked()
    {
        ui.ctx().copy_text(format!("{}, {}", cell.x, cell.y));
        ui.close();
    }
    ui.separator();
    let pinned = map.layout.cards.get(&id).is_some_and(|card| card.pinned);
    let verb = if pinned { "Unpin" } else { "Pin" };
    if ui
        .add(widgets::shortcut_button(verb, EditorKey::Pin))
        .clicked()
    {
        map.change(
            editor,
            ContentEdit::new(format!("{verb} {}", hit.area)),
            |layout| {
                if let Some(card) = layout.cards.get_mut(&id) {
                    card.pinned = !pinned;
                }
            },
        );
        ui.close();
    }
    if ui.button("Route from here").clicked() {
        start_route(editor, hit.area.clone());
        ui.close();
    }
    if let Some(from) = editor.views.world.route_from.clone()
        && from != hit.area
        && ui.button(format!("Route from {from} to here")).clicked()
    {
        editor.views.world.route = Some(MapRoute {
            from,
            to: hit.area.clone(),
        });
        editor.views.world.route_from = None;
        ui.close();
    }
}

fn canvas_menu(editor: &mut Editor, ui: &mut Ui, map: &OpenWorldMap, canvas: &MapCanvas) {
    if ui.button("Add note here").clicked() {
        let at = ui
            .input(|i| i.pointer.latest_pos())
            .map_or(canvas.viewport.center, |p| canvas.viewport.to_ground(p));
        super::add_note(editor, map, at);
        ui.close();
    }
    if ui.button("Region from selection").clicked() {
        super::region_from_selection(editor, map);
        ui.close();
    }
    if ui.button("Arrange…").clicked() {
        super::arrange::open(editor);
        ui.close();
    }
}

struct WorldKeys {
    nudge: Vec2,
    resize: i32,
    pin: bool,
    all: bool,
    enter: bool,
    play: bool,
    escape: bool,
    delete: bool,
}

impl WorldKeys {
    fn read(ui: &Ui) -> WorldKeys {
        ui.input(|i| {
            let step = if EditorKey::FineStep.held(i) { 1 } else { 10 };
            let [x, y] = EditorKey::Move.arrow(i).unwrap_or_default();
            let resize = if EditorKey::Shrink.pressed(i) {
                -1
            } else if EditorKey::Grow.pressed(i) {
                1
            } else {
                0
            };
            WorldKeys {
                nudge: Vec2::new(x as f32, y as f32) * step as f32,
                resize: step * resize,
                pin: EditorKey::Pin.pressed(i),
                all: EditorKey::SelectAll.pressed(i),
                enter: EditorKey::Confirm.pressed(i),
                play: EditorKey::PlayHere.pressed(i),
                escape: EditorKey::Cancel.pressed(i),
                delete: EditorKey::Delete.pressed(i),
            }
        })
    }
}

fn keys(
    editor: &mut Editor,
    ui: &Ui,
    map: &OpenWorldMap,
    canvas: &MapCanvas,
    hover: Option<MapTileHit>,
) {
    let keys = WorldKeys::read(ui);
    let parts = selected(editor);
    let cards = map.card_ids(&parts);
    let mut next = map.layout.clone();
    let mut labels: Vec<String> = Vec::new();
    let mut merging = true;
    if keys.nudge != Vec2::ZERO && !cards.is_empty() {
        for id in &cards {
            if let Some(card) = next.cards.get_mut(id) {
                move_card(card, card_at(card) + keys.nudge);
            }
        }
        labels.push(format!("Move {} world map cards", cards.len()));
    }
    if keys.resize != 0 && !cards.is_empty() {
        for id in &cards {
            if let Some(card) = next.cards.get_mut(id) {
                card.size = card
                    .size
                    .saturating_add_signed(keys.resize)
                    .clamp(MIN_SIZE, MAX_SIZE);
            }
        }
        labels.push(format!("Resize {} world map cards", cards.len()));
    }
    if keys.pin && !cards.is_empty() {
        let pinned = !cards.iter().all(|id| map.layout.cards[id].pinned);
        for id in &cards {
            if let Some(card) = next.cards.get_mut(id) {
                card.pinned = pinned;
            }
        }
        let verb = if pinned { "Pin" } else { "Unpin" };
        labels.push(format!("{verb} {} cards", cards.len()));
        merging = false;
    }
    let mut removed = false;
    if keys.delete {
        let mut notes: Vec<usize> = Vec::new();
        let mut regions: Vec<usize> = Vec::new();
        for part in &parts {
            match part {
                MapPart::Note(index) => notes.push(*index),
                MapPart::Region(index) => regions.push(*index),
                MapPart::Card(_) | MapPart::Edge(..) => {}
            }
        }
        notes.sort_unstable_by(|a, b| b.cmp(a));
        regions.sort_unstable_by(|a, b| b.cmp(a));
        let mut count = 0;
        for index in notes {
            if index < next.notes.len() {
                next.notes.remove(index);
                count += 1;
            }
        }
        for index in regions {
            if index < next.regions.len() {
                next.regions.remove(index);
                count += 1;
            }
        }
        tidy_regions(&mut next);
        if count > 0 {
            labels.push(format!("Remove {count} from the world map"));
            merging = false;
            removed = true;
        }
    }
    if !labels.is_empty() {
        let edit = ContentEdit::new(labels.join(" · "));
        let edit = if merging { edit.merging() } else { edit };
        if map.write(editor, edit, &next) && removed {
            select(editor, Vec::new());
        }
    }
    if keys.all {
        let parts = canvas
            .cards
            .iter()
            .map(|card| MapPart::Card(map.atlas.key(card.area).to_owned()))
            .collect();
        select(editor, parts);
    }
    if keys.enter {
        match (&hover, canvas.hovered) {
            (Some(hit), _) => area_view::open_area_at(editor, &hit.area, Some(snap(hit.at)), None),
            (None, Some(area)) => area_view::open_area_at(editor, map.atlas.key(area), None, None),
            (None, None) => {}
        }
    }
    if keys.play
        && let Some(hit) = &hover
        && let Some(id) = map.atlas.id(&hit.area)
        && area::load(&map.content, id).mesh.walkable(snap(hit.at))
    {
        play::start(
            editor,
            Some(Place {
                area: id,
                at: snap(hit.at),
            }),
        );
    }
    if keys.escape && editor.views.world.drag.take().is_none() {
        editor.views.world.route_from = None;
        select(editor, Vec::new());
    }
}

fn corner_at(
    map: &OpenWorldMap,
    canvas: &MapCanvas,
    selected: &[MapPart],
    p: Pos2,
) -> Option<(String, Pos2)> {
    let [MapPart::Card(key)] = selected else {
        return None;
    };
    let card = canvas.card(map.atlas.id(key)?)?;
    corners(card.frame)
        .into_iter()
        .any(|corner| (corner - p).length() <= CORNER_HIT)
        .then(|| (key.clone(), card.frame.center()))
}

struct Snap {
    shift: f32,
    note: String,
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

impl Axis {
    fn of(self, p: Pos2) -> f32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
        }
    }

    fn lines(self, mine: Rect, theirs: Rect) -> [(f32, f32, &'static str); 5] {
        let (after, before) = match self {
            Axis::X => ("right of", "left of"),
            Axis::Y => ("under", "above"),
        };
        [
            (self.of(mine.min), self.of(theirs.min), "aligned with"),
            (self.of(mine.max), self.of(theirs.max), "aligned with"),
            (
                self.of(mine.center()),
                self.of(theirs.center()),
                "centred on",
            ),
            (self.of(mine.min), self.of(theirs.max) + GAP, after),
            (self.of(mine.max), self.of(theirs.min) - GAP, before),
        ]
    }
}

fn snapped(
    map: &OpenWorldMap,
    moving: &[String],
    delta: Vec2,
    scale: f32,
) -> (Vec2, Option<String>) {
    let threshold = SNAP_PX / scale;
    let atlas = &map.atlas;
    let moving: Vec<AreaId> = moving.iter().filter_map(|key| atlas.id(key)).collect();
    let rect = |area: &AreaId| map.card_rect(*area, &map.layout.cards[area]);
    let mut best: [Option<Snap>; 2] = [None, None];
    for mover in moving
        .iter()
        .filter(|area| map.layout.cards.contains_key(area))
    {
        let mine = rect(mover).translate(delta);
        for other in map
            .layout
            .cards
            .keys()
            .filter(|area| !moving.contains(area))
        {
            let theirs = rect(other);
            for (slot, axis) in best.iter_mut().zip([Axis::X, Axis::Y]) {
                for (at, target, how) in axis.lines(mine, theirs) {
                    let shift = target - at;
                    if shift.abs() <= threshold
                        && slot
                            .as_ref()
                            .is_none_or(|known| shift.abs() < known.shift.abs())
                    {
                        *slot = Some(Snap {
                            shift,
                            note: format!(
                                "{} snapped {how} {}",
                                atlas.file_name(*mover),
                                atlas.file_name(*other)
                            ),
                        });
                    }
                }
            }
        }
    }
    let [x, y] = best;
    let note = y.as_ref().or(x.as_ref()).map(|snap| snap.note.clone());
    let shift = Vec2::new(
        x.map_or(0.0, |snap| snap.shift),
        y.map_or(0.0, |snap| snap.shift),
    );
    (delta + shift, note)
}
