use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect as ERect, RichText, Sense, Stroke, Ui, Vec2};
use engine::systems::actor::Action;
use engine::systems::area::tileset::TileFrame;

use super::{OpenTileset, SheetRect, change_tileset, sheet, tile_mut};
use crate::app::Editor;
use crate::forms;
use crate::project::ContentEdit;
use crate::ui::{patterns, theme, widgets};
use crate::views::model::StripFrame;
use crate::views::model::preview;

pub const MAX_FRAMES: usize = 256;
const PREVIEW_W: f32 = 224.0;
const TIMING_HINT: &str =
    "Timing edits as a model's do: pick a frame, then Alt ← → or its length in the inspector.";

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum OffsetUnit {
    #[default]
    Columns,
    Rows,
}

impl OffsetUnit {
    fn label(self) -> &'static str {
        match self {
            OffsetUnit::Columns => "columns",
            OffsetUnit::Rows => "rows",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OffsetFrame {
    pub offset: i64,
    pub ms: u32,
}

impl OffsetFrame {
    fn on(self, tile: u32) -> TileFrame {
        TileFrame {
            tile: (i64::from(tile) + self.offset).max(0) as u32,
            ms: self.ms,
        }
    }
}

pub struct OddTile {
    pub tile: u32,
    pub frames: Vec<OffsetFrame>,
}

pub struct CommonAnimation {
    pub frames: Vec<OffsetFrame>,
    pub tiles: Vec<u32>,
    pub odd: Vec<OddTile>,
    pub still: Vec<u32>,
}

impl CommonAnimation {
    pub fn of(tileset: &OpenTileset, selection: SheetRect) -> Option<CommonAnimation> {
        let mut patterns: Vec<(Vec<OffsetFrame>, Vec<u32>)> = Vec::new();
        let mut still = Vec::new();
        for tile in selection.tiles(tileset.sheet.columns()) {
            let frames = offsets_of(tileset.def.animation(tile), tile);
            if frames.is_empty() {
                still.push(tile);
                continue;
            }
            match patterns
                .iter_mut()
                .find(|(seen, _)| same_offsets(seen, &frames))
            {
                Some((_, tiles)) => tiles.push(tile),
                None => patterns.push((frames, vec![tile])),
            }
        }
        let (most, _) = patterns
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, tiles))| tiles.len())?;
        let (frames, tiles) = patterns.remove(most);
        let odd = patterns
            .into_iter()
            .flat_map(|(frames, tiles)| {
                tiles.into_iter().map(move |tile| OddTile {
                    tile,
                    frames: frames.clone(),
                })
            })
            .collect();
        Some(CommonAnimation {
            frames,
            tiles,
            odd,
            still,
        })
    }

    fn frame(&self, index: Option<usize>) -> Option<usize> {
        index.filter(|index| *index < self.frames.len())
    }
}

pub fn legend(ui: &mut Ui) {
    ui.horizontal(|ui| {
        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        ui.painter().rect_stroke(
            swatch,
            2.0,
            Stroke::new(1.5, Color32::WHITE),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .rect_filled(swatch.shrink(4.0), 1.0, Color32::WHITE);
        ui.label(theme::muted("animated"));
        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        widgets::dashed_rect(
            ui.painter(),
            swatch,
            Stroke::new(1.0, theme::SELECTION),
            3.0,
            2.0,
        );
        ui.label(theme::muted("its frames"));
    });
}

pub fn show(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let selection = editor.views.tileset.view(key).selection;
    let common = selection.and_then(|selection| CommonAnimation::of(tileset, selection));
    let odd: BTreeSet<u32> = common
        .iter()
        .flat_map(|common| common.odd.iter().map(|odd| odd.tile))
        .collect();
    let canvas = sheet::canvas(editor, ui, tileset, |painter, tile, cell| {
        if tileset.animated(tile) {
            painter.rect_stroke(
                cell,
                0.0,
                Stroke::new(1.5, Color32::from_white_alpha(200)),
                egui::StrokeKind::Inside,
            );
            let dot =
                ERect::from_center_size(cell.right_top() + Vec2::new(-5.0, 5.0), Vec2::splat(5.0));
            painter.rect_filled(dot, 1.0, Color32::WHITE);
        }
        if odd.contains(&tile) {
            let badge = cell.center();
            painter.circle_filled(badge, (cell.width() * 0.22).max(6.0), theme::WARN);
            painter.text(
                badge,
                egui::Align2::CENTER_CENTER,
                "!",
                egui::FontId::proportional((cell.width() * 0.3).max(9.0)),
                Color32::BLACK,
            );
        }
    });
    sheet::select(editor, ui, tileset, &canvas);
    sheet::draw_selection(editor, ui, key, &canvas);
    if let (Some(selection), Some(common)) = (selection, &common) {
        let painter = ui.painter_at(canvas.layout.rect.expand(24.0));
        let shown = canvas.layout.shown;
        let columns = i64::from(tileset.sheet.columns());
        for (index, frame) in common.frames.iter().enumerate() {
            let start = i64::from(selection.y) * columns + i64::from(selection.x) + frame.offset;
            let (x, y) = (start.rem_euclid(columns), start.div_euclid(columns));
            let rect = ERect::from_min_size(
                canvas.layout.rect.min + Vec2::new(x as f32 * shown, y as f32 * shown),
                Vec2::new(selection.w as f32 * shown, selection.h as f32 * shown),
            );
            let label = if index == 0 {
                format!("frame +0 · {} tiles", common.tiles.len())
            } else {
                widgets::dashed_rect(&painter, rect, Stroke::new(1.5, theme::SELECTION), 6.0, 4.0);
                format!("frame {}", signed(frame.offset))
            };
            widgets::floating_chip(
                &painter,
                Pos2::new(rect.center().x, rect.max.y + 3.0),
                egui::Align2::CENTER_TOP,
                &label,
                Color32::from_black_alpha(200),
                Color32::WHITE,
                11.0,
            );
        }
    }
    let frames = reference_frames(common.as_ref());
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let view = editor.views.tileset.view_mut(key);
    preview::keys(ui, &mut view.preview, &frames);
    preview::tick(&mut view.preview, dt, &frames, Action::Idle);
    if let Some(common) = &common {
        keys(editor, ui, key, common);
    }
    ui.ctx().request_repaint();
}

pub fn card(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let Some(selection) = editor.views.tileset.view(key).selection else {
        widgets::hint(ui, "Drag over tiles to select them.");
        return;
    };
    ui.label(
        theme::mono(format!("tileset/{key} › {} tiles", selection.count()))
            .color(theme::CODE)
            .size(12.0),
    );
    ui.add_space(4.0);
    let Some(common) = CommonAnimation::of(tileset, selection) else {
        still_card(editor, ui, tileset, selection);
        return;
    };
    let columns = i64::from(tileset.sheet.columns());
    let view = editor.views.tileset.view(key);
    let frame = common.frame(view.frame);
    let unit = view.offsets_in;
    let mut picked_frame = None;
    let mut picked_unit = unit;
    let mut add = false;
    let mut typed_ms = None;
    let mut fix_odd = None;
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, "animation");
        ui.add_space(4.0);
        widgets::kv(ui, "frames", |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (index, each) in common.frames.iter().enumerate() {
                let text = offset_text(each.offset, unit, columns);
                if widgets::mode_chip(ui, &text, frame == Some(index)).clicked() {
                    picked_frame = Some(index);
                }
            }
            if common.frames.len() < MAX_FRAMES
                && widgets::chip(ui, "+")
                    .on_hover_text("another frame, one step further")
                    .clicked()
            {
                add = true;
            }
        });
        widgets::kv(ui, "offset by", |ui| {
            widgets::combo_box(("offset-unit", key))
                .selected_text(unit.label())
                .width(140.0)
                .show_ui(ui, |ui| {
                    for option in [OffsetUnit::Columns, OffsetUnit::Rows] {
                        ui.selectable_value(&mut picked_unit, option, option.label());
                    }
                });
        });
        let first = common.frames[0].ms;
        let same = common.frames.iter().all(|each| each.ms == first);
        let label = if frame.is_some() {
            "this frame"
        } else {
            "each"
        };
        widgets::kv(ui, label, |ui| {
            let current = match frame {
                Some(index) => common.frames[index].ms.to_string(),
                None if same => first.to_string(),
                None => String::new(),
            };
            let id = ui.id().with(("each-ms", key));
            let typed = forms::drafted(ui, id, &current, |ui, draft| {
                forms::boxed_input(ui, Some("ms"), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(70.0)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("mixed"),
                    )
                })
            });
            if let Some(typed) = typed
                && typed.trim() != current
                && let Ok(ms) = typed.trim().parse::<u32>()
                && ms >= 1
            {
                typed_ms = Some(ms);
            }
        });
        widgets::kv_text(ui, "plays", "loops, as every tile animation");
    });
    if let Some(odd) = common.odd.first() {
        ui.add_space(6.0);
        let last = odd.frames.last().map_or(0, |frame| frame.offset);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("⚠").color(theme::WARN));
            ui.label(
                theme::muted(format!(
                    "Tile {} has {} frames, +0 to {}; the other {} have {}.",
                    odd.tile,
                    odd.frames.len(),
                    signed(last),
                    common.tiles.len(),
                    common.frames.len()
                ))
                .size(12.5),
            );
        });
        if ui
            .button(format!(
                "Give tile {} the same {} frames",
                odd.tile,
                common.frames.len()
            ))
            .clicked()
        {
            fix_odd = Some(odd.tile);
        }
        if common.odd.len() > 1 {
            widgets::hint(
                ui,
                &format!("… and {} more out of step", common.odd.len() - 1),
            );
        }
    }
    if !common.still.is_empty() {
        widgets::hint(
            ui,
            &format!(
                "{} of the selected tiles do not animate.",
                common.still.len()
            ),
        );
    }
    ui.add_space(6.0);
    preview_block(editor, ui, tileset, selection, &common);
    let view = editor.views.tileset.view_mut(key);
    view.offsets_in = picked_unit;
    if let Some(index) = picked_frame {
        view.toggle_frame(index);
    }
    let tiles = common.tiles.len();
    if add {
        let mut frames = common.frames.clone();
        let last = frames[frames.len() - 1];
        let step = match frames.len() {
            1 => i64::from(selection.w),
            n => last.offset - frames[n - 2].offset,
        };
        frames.push(OffsetFrame {
            offset: last.offset + step,
            ms: last.ms,
        });
        let label = format!(
            "Add frame {} to {tiles} animated tiles of {key}",
            signed(last.offset + step)
        );
        animate(editor, key, &common.tiles, &frames, ContentEdit::new(label));
    }
    if let Some(ms) = typed_ms {
        let mut frames = common.frames.clone();
        let label = match frame {
            Some(index) => {
                frames[index].ms = ms;
                format!(
                    "Frame {} lasts {ms} ms on {tiles} tiles of {key}",
                    signed(frames[index].offset)
                )
            }
            None => {
                frames.iter_mut().for_each(|each| each.ms = ms);
                format!("Every frame lasts {ms} ms on {tiles} tiles of {key}")
            }
        };
        animate(editor, key, &common.tiles, &frames, ContentEdit::new(label));
    }
    if let Some(tile) = fix_odd {
        let label = format!(
            "Give tile {tile} the same {} frames on {key}",
            common.frames.len()
        );
        animate(
            editor,
            key,
            &[tile],
            &common.frames,
            ContentEdit::new(label),
        );
    }
}

pub fn timeline(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset) {
    let key = tileset.key.as_str();
    let Some(selection) = editor.views.tileset.view(key).selection else {
        ui.add_space(8.0);
        widgets::hint(ui, "Select animated tiles to see their frames here.");
        return;
    };
    let Some(common) = CommonAnimation::of(tileset, selection) else {
        ui.add_space(8.0);
        widgets::hint(ui, "None of the selected tiles animates.");
        return;
    };
    let frame = editor.views.tileset.view(key).frame;
    let total: u32 = common.frames.iter().map(|each| each.ms).sum();
    let mut pick = None;
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 6.0;
        for (index, each) in common.frames.iter().enumerate() {
            let on = frame == Some(index);
            if timeline_card(ui, tileset, selection, index, *each, on).clicked() {
                pick = Some(index);
            }
        }
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.add_space(40.0);
            ui.label(theme::muted(format!("{total} ms in all. {TIMING_HINT}")).size(12.5));
        });
    });
    if let Some(index) = pick {
        editor.views.tileset.view_mut(key).toggle_frame(index);
    }
}

fn offsets_of(frames: &[TileFrame], tile: u32) -> Vec<OffsetFrame> {
    frames
        .iter()
        .map(|frame| OffsetFrame {
            offset: i64::from(frame.tile) - i64::from(tile),
            ms: frame.ms,
        })
        .collect()
}

fn same_offsets(a: &[OffsetFrame], b: &[OffsetFrame]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.offset == b.offset)
}

fn signed(offset: i64) -> String {
    format!("{offset:+}")
}

fn offset_text(offset: i64, unit: OffsetUnit, columns: i64) -> String {
    match unit {
        OffsetUnit::Rows if offset % columns == 0 => {
            format!("{} rows", signed(offset / columns))
        }
        OffsetUnit::Rows | OffsetUnit::Columns => signed(offset),
    }
}

fn reference_frames(common: Option<&CommonAnimation>) -> Vec<StripFrame> {
    let Some(common) = common else {
        return Vec::new();
    };
    let anchor = common.tiles[0];
    common
        .frames
        .iter()
        .map(|frame| StripFrame {
            frame: frame.on(anchor).tile,
            ms: frame.ms,
            ..StripFrame::default()
        })
        .collect()
}

fn keys(editor: &mut Editor, ui: &Ui, key: &str, common: &CommonAnimation) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let Some(index) = common.frame(editor.views.tileset.view(key).frame) else {
        return;
    };
    let (left, right, alt, shift, delete) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.modifiers.alt,
            i.modifiers.shift,
            i.key_pressed(egui::Key::Delete),
        )
    });
    if alt && (left || right) {
        let step = if shift { 1 } else { 10 };
        let mut frames = common.frames.clone();
        let ms = &mut frames[index].ms;
        *ms = if right {
            (*ms + step).min(60_000)
        } else {
            ms.saturating_sub(step).max(1)
        };
        let label = format!(
            "Retime frame {} of {} animated tiles of {key}",
            signed(frames[index].offset),
            common.tiles.len()
        );
        animate(
            editor,
            key,
            &common.tiles,
            &frames,
            ContentEdit::new(label).merging(),
        );
    }
    if delete {
        remove_frame(editor, key, common, index);
    }
}

fn remove_frame(editor: &mut Editor, key: &str, common: &CommonAnimation, index: usize) {
    if common.frames.len() <= 2 {
        editor.toast("A tile animation keeps at least two frames");
        return;
    }
    let mut frames = common.frames.clone();
    let removed = frames.remove(index);
    let label = format!(
        "Remove frame {} from {} animated tiles of {key}",
        signed(removed.offset),
        common.tiles.len()
    );
    if animate(editor, key, &common.tiles, &frames, ContentEdit::new(label)) {
        editor.views.tileset.view_mut(key).frame = None;
    }
}

fn animate(
    editor: &mut Editor,
    key: &str,
    tiles: &[u32],
    frames: &[OffsetFrame],
    edit: ContentEdit,
) -> bool {
    change_tileset(editor, key, edit, |def| {
        for tile in tiles {
            tile_mut(def, *tile).animation = frames.iter().map(|frame| frame.on(*tile)).collect();
        }
    })
}

fn still_card(editor: &mut Editor, ui: &mut Ui, tileset: &OpenTileset, selection: SheetRect) {
    let mut animate_now = false;
    widgets::card_frame(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::card_title(ui, "animation");
        ui.add_space(4.0);
        widgets::hint(ui, "None of these tiles animates yet.");
        animate_now = ui.button("+ Animate with a second frame").clicked();
    });
    if animate_now {
        let tiles: Vec<u32> = selection.tiles(tileset.sheet.columns()).collect();
        let frames = [
            OffsetFrame { offset: 0, ms: 250 },
            OffsetFrame {
                offset: i64::from(selection.w),
                ms: 250,
            },
        ];
        let label = format!("Animate {} tiles of {}: 2 frames", tiles.len(), tileset.key);
        animate(
            editor,
            &tileset.key,
            &tiles,
            &frames,
            ContentEdit::new(label),
        );
    }
}

fn preview_block(
    editor: &mut Editor,
    ui: &mut Ui,
    tileset: &OpenTileset,
    selection: SheetRect,
    common: &CommonAnimation,
) {
    widgets::section(ui, "Preview");
    let frames = reference_frames(Some(common));
    let view = editor.views.tileset.view_mut(&tileset.key);
    let index = preview::index_at(&frames, view.preview.position, false);
    let shown = (PREVIEW_W / selection.w as f32).clamp(4.0, 48.0).floor();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(selection.w as f32 * shown, selection.h as f32 * shown),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CANVAS);
    draw_block(&painter, rect.min, shown, tileset, selection, index);
    ui.horizontal(|ui| {
        if widgets::chip(ui, if view.preview.playing { "⏸" } else { "▶" }).clicked() {
            view.preview.playing = !view.preview.playing;
        }
        widgets::chip(ui, &format!("frame {} / {}", index + 1, frames.len()));
    });
}

fn timeline_card(
    ui: &mut Ui,
    tileset: &OpenTileset,
    selection: SheetRect,
    index: usize,
    frame: OffsetFrame,
    on: bool,
) -> egui::Response {
    let (response, painter) = patterns::card_slot(ui, Vec2::new(136.0, 132.0), on);
    let rect = response.rect;
    let art = ERect::from_min_max(
        rect.min + Vec2::new(8.0, 6.0),
        rect.right_bottom() - Vec2::new(8.0, 26.0),
    );
    let shown = (art.width() / selection.w as f32)
        .min(art.height() / selection.h as f32)
        .floor()
        .max(1.0);
    let block = ERect::from_center_size(
        art.center(),
        Vec2::new(selection.w as f32 * shown, selection.h as f32 * shown),
    );
    draw_block(&painter, block.min, shown, tileset, selection, index);
    painter.text(
        Pos2::new(rect.center().x, rect.max.y - 12.0),
        egui::Align2::CENTER_CENTER,
        format!("{} · {} ms", signed(frame.offset), frame.ms),
        egui::FontId::monospace(12.0),
        if on { Color32::WHITE } else { theme::TEXT },
    );
    response
}

fn draw_block(
    painter: &egui::Painter,
    origin: Pos2,
    shown: f32,
    tileset: &OpenTileset,
    selection: SheetRect,
    frame: usize,
) {
    let columns = tileset.sheet.columns();
    for offset in selection.offsets() {
        let tile = selection.tile_at(offset, columns);
        let cell = ERect::from_min_size(
            origin + Vec2::new(offset.x as f32 * shown, offset.y as f32 * shown),
            Vec2::splat(shown),
        );
        let shown_tile = tileset
            .def
            .animation(tile)
            .get(frame)
            .map_or(tile, |each| each.tile);
        tileset.sheet.draw(painter, cell, shown_tile);
    }
}
