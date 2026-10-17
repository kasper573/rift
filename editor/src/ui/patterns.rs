use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::theme;

pub fn card_slot(ui: &mut Ui, size: Vec2, selected: bool) -> (Response, Painter) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    paint_card(&painter, rect, selected);
    (response, painter)
}

pub fn paint_card(painter: &Painter, rect: Rect, selected: bool) {
    let (fill, border) = if selected {
        (theme::ACCENT_SOFT, theme::ACCENT)
    } else {
        (theme::INPUT, theme::BORDER)
    };
    painter.rect(
        rect,
        3.0,
        fill,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
}

pub fn hatch(painter: &Painter, rect: Rect, color: Color32, scale: f32) {
    painter.rect_filled(rect, 0.0, theme::alpha(color, color.a() / 3));
    let painter = painter.with_clip_rect(rect);
    let step = (scale / 2.0).clamp(3.0, 12.0);
    let mut x = rect.min.x - rect.height();
    while x < rect.max.x {
        let a = Pos2::new(x, rect.max.y);
        let b = Pos2::new(x + rect.height(), rect.min.y);
        painter.line_segment([a, b], Stroke::new(1.0, color));
        x += step;
    }
}

pub fn dots(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, theme::alpha(color, 60));
    let step = (rect.width() / 4.0).max(3.0);
    let mut y = rect.min.y + step / 2.0;
    while y < rect.max.y {
        let mut x = rect.min.x + step / 2.0;
        while x < rect.max.x {
            painter.circle_filled(Pos2::new(x, y), (step / 5.0).max(1.0), color);
            x += step;
        }
        y += step;
    }
}

pub fn grid_lines(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, theme::alpha(color, 50));
    let step = (rect.width() / 3.0).max(3.0);
    let stroke = Stroke::new(1.0, color);
    let mut x = rect.min.x + step;
    while x < rect.max.x {
        painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], stroke);
        x += step;
    }
    let mut y = rect.min.y + step;
    while y < rect.max.y {
        painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], stroke);
        y += step;
    }
}

pub fn stripes(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, theme::alpha(color, 50));
    let step = (rect.height() / 4.0).max(3.0);
    let stroke = Stroke::new(1.0, color);
    let mut y = rect.min.y + step / 2.0;
    while y < rect.max.y {
        painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], stroke);
        y += step;
    }
}
