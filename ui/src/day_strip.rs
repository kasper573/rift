//! A strip of a whole range, such as the hours of a day, showing how much of something is there
//! at each point of it, over the colours of the spans that divide the range.

use std::ops::RangeInclusive;

use egui::epaint::Mesh;
use egui::{Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use crate::timespans::{self, Timespan};
use crate::{ink, paint, theme};

/// Draws how much of something is there over `range`, 0 to 1 as `share` says of each value of
/// it: a filled rise over the colours of `spans`, hatched wherever there is none of it. Pointing
/// at it says the value under the pointer.
pub fn day_strip(
    ui: &mut Ui,
    range: RangeInclusive<f32>,
    spans: &[Timespan],
    share: impl Fn(f32) -> f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().min(WIDEST), HEIGHT),
        Sense::hover(),
    );
    let (start, length) = (*range.start(), range.end() - range.start());
    let value_at = |along: f32| start + along * length;
    let painter = ui.painter();
    paint::band(painter, rect, |along| {
        ink::wash(timespans::color_at(spans, &range, value_at(along)), 0.45)
    });

    let columns = (rect.width() / COLUMN).ceil().max(1.0) as usize;
    let shares: Vec<f32> = (0..=columns)
        .map(|column| share(value_at(column as f32 / columns as f32)).clamp(0.0, 1.0))
        .collect();
    let x_of = |column: usize| rect.left() + rect.width() * column as f32 / columns as f32;
    let mut rise = Mesh::default();
    let mut empty_from = None;
    for (column, &share) in shares.iter().enumerate() {
        let top = rect.bottom() - rect.height() * share;
        rise.colored_vertex(egui::pos2(x_of(column), top), ink::ink_wash(0.55));
        rise.colored_vertex(egui::pos2(x_of(column), rect.bottom()), ink::ink_wash(0.55));
        if column > 0 {
            let at = (column as u32 - 1) * 2;
            rise.add_triangle(at, at + 1, at + 2);
            rise.add_triangle(at + 1, at + 2, at + 3);
        }
        let empty = share <= 0.0 && column < columns;
        match (empty, empty_from) {
            (true, None) => empty_from = Some(column),
            (false, Some(from)) => {
                let gap = Rect::from_x_y_ranges(x_of(from)..=x_of(column), rect.y_range());
                paint::hatched(painter, gap, theme::MUTED, 8.0);
                empty_from = None;
            }
            _ => {}
        }
    }
    painter.add(Shape::mesh(rise));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    match response.hover_pos() {
        Some(pointer) => {
            let value = value_at((pointer.x - rect.left()) / rect.width());
            painter.vline(pointer.x, rect.y_range(), Stroke::new(1.0, ink::INK));
            response.on_hover_text(format!(
                "{value:.1}: {:.0}%",
                share(value).clamp(0.0, 1.0) * 100.0
            ))
        }
        None => response,
    }
}

const HEIGHT: f32 = 18.0;
const WIDEST: f32 = 360.0;
const COLUMN: f32 = 3.0;
