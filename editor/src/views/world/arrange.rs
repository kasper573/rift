use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use ui::dialog;
use ui::egui;
use ui::sheet;
use ui::theme;

use engine::systems::area::AreaId;
use engine::systems::area::world_map::WorldMapDef;
use ui::egui::{Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};

use super::{OpenWorldMap, card_at, hundredths, move_card, selected};
use crate::app::Editor;
use crate::keymap::EditorKey;
use crate::modal::ModalDialog;
use crate::project::ContentEdit;
use crate::views::navigation;

const GAP: f32 = 18.75;
const MARGIN: f32 = 16.0;
const ITERATIONS: usize = 200;
const STILL: f32 = 1.0;
const SETTLED: f32 = 0.0001;
const HINT: f32 = 0.5;
const SLACK: f32 = 0.1;
const ROUNDS: usize = 50;
const PREVIEW: Vec2 = Vec2::new(330.0, 250.0);
const PREVIEW_LINK: Color32 = Color32::from_rgba_premultiplied(19, 14, 9, 115);
const PREVIEW_PINNED: Color32 = ui::ink::AZURE;
const PREVIEW_CARD: Color32 = ui::ink::INK3;

pub struct ArrangeDialog {
    options: ArrangeOptions,
    solved: Option<Solved>,
}

#[derive(Clone, PartialEq)]
struct ArrangeOptions {
    everything: bool,
    keep_regions: bool,
    doors_hint: bool,
    selected: Vec<String>,
}

struct Solved {
    generation: u64,
    options: ArrangeOptions,
    after: WorldMapDef,
}

pub fn open(editor: &mut Editor) {
    let Some(map) = OpenWorldMap::of(editor) else {
        return;
    };
    let selected = map
        .card_ids(&selected(editor))
        .into_iter()
        .map(|area| map.atlas.key(area).to_owned())
        .collect();
    editor.dialogs.push(ModalDialog::Arrange(ArrangeDialog {
        options: ArrangeOptions {
            everything: true,
            keep_regions: true,
            doors_hint: true,
            selected,
        },
        solved: None,
    }));
}

pub(crate) fn arrange_everything(editor: &mut Editor) {
    let Some(map) = OpenWorldMap::of(editor) else {
        return;
    };
    let after = solve(&map, &map.layout, true, true);
    if map.write(editor, ContentEdit::new("Arrange the world map"), &after) {
        editor.views.world.request_fit();
    }
}

pub fn dialog(
    editor: &mut Editor,
    ctx: &egui::Context,
    mut dialog: ArrangeDialog,
) -> Option<ArrangeDialog> {
    let map = OpenWorldMap::of(editor)?;
    let before = &map.layout;
    let generation = map.atlas.generation;
    let solved = match dialog.solved.take() {
        Some(solved) if solved.generation == generation && solved.options == dialog.options => {
            solved
        }
        _ => Solved {
            generation,
            options: dialog.options.clone(),
            after: arranged(&map, &dialog.options),
        },
    };
    let after = &solved.after;
    let pinned = before.cards.values().filter(|card| card.pinned).count();
    let unpinned = before.cards.len() - pinned;
    let moved = after
        .cards
        .iter()
        .filter(|(area, card)| before.cards.get(area).is_some_and(|was| was.at != card.at))
        .count();
    let mut answer = None;
    dialog::card(ctx, "arrange-world", 720.0, |ui| {
        ui.label(RichText::new("Arrange the world map").size(17.0).strong());
        ui.add_space(8.0);
        toggle_row(
            ui,
            &mut dialog.options.everything,
            &format!("Arrange the {unpinned} unpinned cards"),
            &match pinned {
                0 => "nothing is pinned".to_owned(),
                _ => format!("{pinned} pinned stay put"),
            },
        );
        if !dialog.options.everything {
            ui.label(
                theme::faint(match dialog.options.selected.len() {
                    0 => "nothing is selected, so nothing moves".to_owned(),
                    count => format!("only the {count} selected cards move"),
                })
                .size(12.0),
            );
        }
        toggle_row(
            ui,
            &mut dialog.options.keep_regions,
            "Keep each region's cards together",
            "",
        );
        toggle_row(
            ui,
            &mut dialog.options.doors_hint,
            "Let doors hint at direction",
            "a door on the east edge pulls its area east",
        );
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                sheet::section(ui, "Now");
                preview(ui, &map, before);
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                sheet::section(ui, "After");
                preview(ui, &map, after);
            });
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!(
                "This replaces the positions of {moved} cards. {} brings your current layout back.",
                EditorKey::Undo.name()
            ))
            .size(13.0),
        );
        ui.add_space(8.0);
        answer = dialog::answer_row(ui, &format!("Arrange {moved} cards"));
    });
    editor.status.left = vec!["World · arrange preview".to_owned()];
    match answer {
        Some(dialog::Answer::Confirm) => {
            if moved > 0
                && map.write(
                    editor,
                    ContentEdit::new(format!("Arrange {moved} world map cards")),
                    after,
                )
            {
                editor.views.world.request_fit();
            }
            None
        }
        Some(dialog::Answer::Cancel) => None,
        None => {
            dialog.solved = Some(solved);
            Some(dialog)
        }
    }
}

fn arranged(map: &OpenWorldMap, options: &ArrangeOptions) -> WorldMapDef {
    let mut start = map.layout.clone();
    if !options.everything {
        for (area, card) in &mut start.cards {
            if !options
                .selected
                .iter()
                .any(|key| *key == map.atlas.key(*area))
            {
                card.pinned = true;
            }
        }
    }
    let mut after = solve(map, &start, options.keep_regions, options.doors_hint);
    for (area, card) in &mut after.cards {
        card.pinned = map.layout.cards.get(area).is_some_and(|was| was.pinned);
    }
    after
}

fn toggle_row(ui: &mut Ui, on: &mut bool, label: &str, note: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(on, "");
        ui.label(RichText::new(label).size(13.0));
        if !note.is_empty() {
            ui.label(theme::faint(format!("· {note}")).size(12.0));
        }
    });
}

fn preview(ui: &mut Ui, map: &OpenWorldMap, layout: &WorldMapDef) {
    let (rect, _) = ui.allocate_exact_size(PREVIEW, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Outside,
    );
    let Some(bounds) = map.bounds(layout) else {
        return;
    };
    let scale = navigation::pixels_per_unit(
        rect.size() - Vec2::splat(24.0),
        Vec2::new(bounds.width(), bounds.height()),
    );
    let to_screen = |p: Pos2| rect.center() + (p - bounds.center()) * scale;
    for warp in &map.atlas.warps {
        let (Some(a), Some(b)) = (layout.cards.get(&warp.from), layout.cards.get(&warp.to)) else {
            continue;
        };
        painter.line_segment(
            [to_screen(card_at(a)), to_screen(card_at(b))],
            Stroke::new(1.0, PREVIEW_LINK),
        );
    }
    for (area, card) in &layout.cards {
        let r = map.card_rect(*area, card);
        painter.rect_filled(
            Rect::from_min_max(to_screen(r.min), to_screen(r.max)).expand(1.5),
            2.0,
            if card.pinned {
                PREVIEW_PINNED
            } else {
                PREVIEW_CARD
            },
        );
    }
}

fn solve(
    map: &OpenWorldMap,
    layout: &WorldMapDef,
    keep_regions: bool,
    doors_hint: bool,
) -> WorldMapDef {
    let areas: Vec<AreaId> = layout.cards.keys().copied().collect();
    let graph = Graph::of(map, layout, &areas, keep_regions, doors_hint);
    let start: Vec<Vec2> = areas
        .iter()
        .map(|area| card_at(&layout.cards[area]).to_vec2())
        .collect();
    let mut positions = start.clone();
    for _ in 0..ROUNDS {
        let before = positions.clone();
        graph.relax(&mut positions);
        graph.push_apart(&mut positions);
        for position in &mut positions {
            *position = stored(*position);
        }
        if graph.settled(&before, &positions) {
            break;
        }
    }
    graph.place_components(&mut positions);
    let mut out = layout.clone();
    for ((area, position), was) in areas.iter().zip(positions).zip(start) {
        if let Some(card) = out.cards.get_mut(area)
            && (position - was).length() >= STILL
        {
            move_card(card, position.to_pos2());
        }
    }
    out
}

struct Graph {
    sizes: Vec<Vec2>,
    pinned: Vec<bool>,
    hints: Pairs<Vec2>,
    components: Vec<Vec<usize>>,
    hops: Pairs<Option<u32>>,
}

/// A value for every ordered pair of cards, looked up in place: the solver asks for one per pair on
/// every step.
struct Pairs<T> {
    cards: usize,
    values: Vec<T>,
}

impl<T: Copy> Pairs<T> {
    fn new(cards: usize, value: T) -> Pairs<T> {
        Pairs {
            cards,
            values: vec![value; cards * cards],
        }
    }

    fn get(&self, a: usize, b: usize) -> T {
        self.values[a * self.cards + b]
    }

    fn set(&mut self, a: usize, b: usize, value: T) {
        self.values[a * self.cards + b] = value;
    }
}

impl Graph {
    fn of(
        map: &OpenWorldMap,
        layout: &WorldMapDef,
        areas: &[AreaId],
        keep_regions: bool,
        doors_hint: bool,
    ) -> Graph {
        let index: HashMap<AreaId, usize> = areas
            .iter()
            .enumerate()
            .map(|(i, area)| (*area, i))
            .collect();
        let mut links: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut hints = Pairs::new(areas.len(), Vec2::ZERO);
        for warp in &map.atlas.warps {
            let (Some(&a), Some(&b)) = (index.get(&warp.from), index.get(&warp.to)) else {
                continue;
            };
            if a == b {
                continue;
            }
            links.insert((a.min(b), a.max(b)));
            if doors_hint && let Some(at) = warp.at {
                let size = map.atlas.size(warp.from);
                let half = Vec2::new(size.width, size.height) / 2.0;
                let rel = Vec2::new(at.x - half.x, at.y - half.y);
                let side = if rel.x.abs() / half.x.max(1.0) > rel.y.abs() / half.y.max(1.0) {
                    Vec2::new(rel.x.signum(), 0.0)
                } else {
                    Vec2::new(0.0, rel.y.signum())
                };
                hints.set(a, b, hints.get(a, b) + side);
            }
        }
        if keep_regions {
            for region in &layout.regions {
                let members: Vec<usize> = region
                    .cards
                    .iter()
                    .filter_map(|area| index.get(area).copied())
                    .collect();
                for pair in members.windows(2) {
                    links.insert((pair[0].min(pair[1]), pair[0].max(pair[1])));
                }
            }
        }
        let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); areas.len()];
        for &(a, b) in &links {
            adjacency[a].push(b);
            adjacency[b].push(a);
        }
        let components = components(&adjacency);
        let mut hops = Pairs::new(areas.len(), None);
        for component in &components {
            for ((a, b), d) in hop_distances(&adjacency, component) {
                hops.set(a, b, Some(d));
                hops.set(b, a, Some(d));
            }
        }
        Graph {
            sizes: areas
                .iter()
                .map(|area| map.card_rect(*area, &layout.cards[area]).size())
                .collect(),
            pinned: areas.iter().map(|area| layout.cards[area].pinned).collect(),
            hints,
            hops,
            components,
        }
    }

    fn ideal(&self, a: usize, b: usize, hops: f32) -> f32 {
        let sizes = &self.sizes;
        let span = (sizes[a].x + sizes[b].x + sizes[a].y + sizes[b].y) / 4.0;
        hops * (span + GAP)
    }

    fn relax(&self, positions: &mut [Vec2]) {
        let mut springs = Vec::new();
        for component in &self.components {
            let start = centroid(component, positions);
            for _ in 0..ITERATIONS {
                let before = positions.to_vec();
                for &i in component.iter().filter(|&&i| !self.pinned[i]) {
                    self.springs(i, positions[i], component, positions, &mut springs);
                    if let Some(to) = centre(&springs) {
                        positions[i] = to;
                    }
                }
                if self.settled(&before, positions) {
                    break;
                }
            }
            if component.iter().all(|&i| !self.pinned[i]) {
                let drift = start - centroid(component, positions);
                for &i in component {
                    positions[i] += drift;
                }
            }
        }
    }

    fn settled(&self, before: &[Vec2], after: &[Vec2]) -> bool {
        let bounds = after
            .iter()
            .zip(&self.sizes)
            .map(|(at, size)| Rect::from_center_size(at.to_pos2(), *size))
            .fold(Rect::NOTHING, |all, card| all.union(card));
        let change = before
            .iter()
            .zip(after)
            .map(|(was, now)| (*now - *was).length())
            .fold(0.0f32, f32::max);
        change < SETTLED * bounds.size().max_elem().max(1.0)
    }

    fn springs(
        &self,
        i: usize,
        at: Vec2,
        component: &[usize],
        positions: &[Vec2],
        springs: &mut Vec<Spring>,
    ) {
        springs.clear();
        for &j in component.iter().filter(|&&j| j != i) {
            let Some(d) = self.hops.get(i, j) else {
                continue;
            };
            let d = d as f32;
            let weight = 1.0 / (d * d);
            let delta = at - positions[j];
            let direction = delta / delta.length().max(0.001);
            springs.push(Spring {
                weight,
                rest: self.beside(i, j, d, direction, positions),
            });
            let pull = self.hints.get(j, i) - self.hints.get(i, j);
            if d == 1.0 && pull != Vec2::ZERO {
                springs.push(Spring {
                    weight: weight * HINT,
                    rest: self.beside(i, j, d, pull.normalized(), positions),
                });
            }
        }
    }

    fn beside(&self, i: usize, j: usize, hops: f32, direction: Vec2, positions: &[Vec2]) -> Vec2 {
        positions[j] + self.ideal(i, j, hops).max(self.clearance(i, j, direction)) * direction
    }

    fn overlap(&self, i: usize, at: Vec2, j: usize, other: Vec2) -> Option<Vec2> {
        let need = (self.sizes[i] + self.sizes[j]) / 2.0 + Vec2::splat(MARGIN);
        let delta = at - other;
        let overlap = need - delta.abs();
        if overlap.x <= SLACK || overlap.y <= SLACK {
            return None;
        }
        let away = |d: f32| if d >= 0.0 { 1.0 } else { -1.0 };
        Some(if overlap.x < overlap.y {
            Vec2::new(overlap.x * away(delta.x), 0.0)
        } else {
            Vec2::new(0.0, overlap.y * away(delta.y))
        })
    }

    fn clearance(&self, a: usize, b: usize, direction: Vec2) -> f32 {
        let need = (self.sizes[a] + self.sizes[b]) / 2.0 + Vec2::splat(MARGIN);
        (need.x / direction.x.abs().max(f32::EPSILON))
            .min(need.y / direction.y.abs().max(f32::EPSILON))
    }

    fn push_apart(&self, positions: &mut [Vec2]) {
        for _ in 0..ITERATIONS {
            let mut any = false;
            for i in 0..positions.len() {
                for j in (i + 1)..positions.len() {
                    if self.pinned[i] && self.pinned[j] {
                        continue;
                    }
                    let Some(push) = self.overlap(i, positions[i], j, positions[j]) else {
                        continue;
                    };
                    any = true;
                    match (self.pinned[i], self.pinned[j]) {
                        (true, false) => positions[j] -= push,
                        (false, true) => positions[i] += push,
                        _ => {
                            positions[i] += push / 2.0;
                            positions[j] -= push / 2.0;
                        }
                    }
                    positions[i] = stored(positions[i]);
                    positions[j] = stored(positions[j]);
                }
            }
            if !any {
                break;
            }
        }
    }

    fn place_components(&self, positions: &mut [Vec2]) {
        let bounds = |component: &[usize], positions: &[Vec2]| -> Rect {
            component
                .iter()
                .map(|&i| Rect::from_center_size(positions[i].to_pos2(), self.sizes[i]))
                .reduce(|a, b| a.union(b))
                .unwrap_or(Rect::NOTHING)
        };
        let mut cursor: Option<f32> = None;
        let mut top: Option<f32> = None;
        for component in &self.components {
            let rect = bounds(component, positions);
            if component.iter().any(|&i| self.pinned[i]) {
                cursor = Some(cursor.map_or(rect.max.x, |x| x.max(rect.max.x)));
                top = Some(top.map_or(rect.min.y, |y| y.min(rect.min.y)));
                continue;
            }
            let (Some(x), Some(y)) = (cursor, top) else {
                cursor = Some(rect.max.x);
                top = Some(rect.min.y);
                continue;
            };
            let shift = Vec2::new(x + GAP - rect.min.x, y - rect.min.y);
            for &i in component {
                positions[i] = stored(positions[i] + shift);
            }
            cursor = Some(rect.max.x + shift.x);
        }
    }
}

fn stored(at: Vec2) -> Vec2 {
    Vec2::new(hundredths(at.x), hundredths(at.y))
}

struct Spring {
    weight: f32,
    rest: Vec2,
}

fn centre(springs: &[Spring]) -> Option<Vec2> {
    let weight: f32 = springs.iter().map(|spring| spring.weight).sum();
    (weight > 0.0).then(|| {
        springs
            .iter()
            .fold(Vec2::ZERO, |sum, spring| sum + spring.weight * spring.rest)
            / weight
    })
}

fn centroid(component: &[usize], positions: &[Vec2]) -> Vec2 {
    component
        .iter()
        .fold(Vec2::ZERO, |sum, &i| sum + positions[i])
        / component.len() as f32
}

fn components(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; adjacency.len()];
    let mut out = Vec::new();
    for start in 0..adjacency.len() {
        if seen[start] {
            continue;
        }
        let mut stack = vec![start];
        let mut component = Vec::new();
        seen[start] = true;
        while let Some(node) = stack.pop() {
            component.push(node);
            for &next in &adjacency[node] {
                if !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        component.sort();
        out.push(component);
    }
    out
}

fn hop_distances(adjacency: &[Vec<usize>], component: &[usize]) -> BTreeMap<(usize, usize), u32> {
    let mut out = BTreeMap::new();
    for &start in component {
        let mut distance: HashMap<usize, u32> = HashMap::from([(start, 0)]);
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            let here = distance[&node];
            for &next in &adjacency[node] {
                if let Entry::Vacant(slot) = distance.entry(next) {
                    slot.insert(here + 1);
                    queue.push_back(next);
                }
            }
        }
        for (node, d) in distance.into_iter().filter(|(node, _)| *node > start) {
            out.insert((start, node), d);
        }
    }
    out
}
