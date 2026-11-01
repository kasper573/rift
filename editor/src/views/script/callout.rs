use super::graph::ScriptGraph;
use super::{doc::ScriptRow, graph};
use crate::app::Editor;
use std::time::Instant;
use ui::egui;
use ui::egui::{Pos2, Rect, Stroke, Ui, vec2};
use ui::theme;

pub struct ScriptCallout {
    pub at: ScriptRow,
    pub node: String,
    pub to: Option<String>,
    pub rule: String,
    pub text: String,
    pub since: Instant,
}

impl ScriptCallout {
    pub fn live(&self, at: &ScriptRow) -> bool {
        self.at == *at && self.since.elapsed().as_secs_f32() < 12.0
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut Ui, at: &ScriptRow) {
    let Some(callout) = editor
        .views
        .script
        .callout
        .as_ref()
        .filter(|callout| callout.live(at))
    else {
        return;
    };
    let Some(view) = editor.views.script.view(at) else {
        return;
    };
    let Some(to_global) = view.to_global else {
        return;
    };
    let on_screen = |name: &str| view.rects.get(name).map(|rect| to_global * *rect);
    let Some(anchor) = on_screen(&callout.node) else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            ui.id().with("callout"),
        ))
        .with_clip_rect(ui.max_rect());
    let stroke = Stroke::new(1.5, theme::DANGER);
    if let Some(target) = callout.to.as_deref().and_then(on_screen) {
        callout_arrow(&painter, anchor.right_center(), target.left_center());
        painter.rect_stroke(target, 6.0, stroke, egui::StrokeKind::Outside);
    }
    painter.rect_stroke(anchor, 6.0, stroke, egui::StrokeKind::Outside);
    callout_card(ui, &painter, anchor, callout);
}

pub(crate) fn callout_arrow(painter: &egui::Painter, from: Pos2, to: Pos2) {
    let lift = vec2(0.0, -(24.0 + (from.x - to.x).abs() * 0.06).min(80.0));
    let curve = egui::epaint::CubicBezierShape::from_points_stroke(
        [
            from,
            from + lift + vec2(60.0, 0.0),
            to + lift - vec2(60.0, 0.0),
            to,
        ],
        false,
        egui::Color32::TRANSPARENT,
        Stroke::NONE,
    );
    let points: Vec<Pos2> = curve.flatten(Some(0.5));
    painter.add(egui::Shape::dashed_line(
        &points,
        Stroke::new(1.5, theme::DANGER),
        6.0,
        5.0,
    ));
}

pub(crate) fn callout_card(
    ui: &Ui,
    painter: &egui::Painter,
    anchor: Rect,
    callout: &ScriptCallout,
) {
    let width = 300.0;
    let title = ui.fonts_mut(|fonts| {
        fonts.layout(
            callout.rule.clone(),
            egui::FontId::proportional(13.5),
            theme::DANGER,
            width,
        )
    });
    let body = ui.fonts_mut(|fonts| {
        fonts.layout(
            callout.text.clone(),
            egui::FontId::proportional(12.5),
            theme::TEXT,
            width,
        )
    });
    let pad = 10.0;
    let size = vec2(
        width + pad * 2.0,
        title.size().y + body.size().y + pad * 2.0 + 4.0,
    );
    let mut top_left = anchor.left_bottom() + vec2(0.0, 10.0);
    top_left.x = top_left
        .x
        .min(ui.max_rect().right() - size.x - 8.0)
        .max(ui.max_rect().left() + 8.0);
    if top_left.y + size.y > ui.max_rect().bottom() - 8.0 {
        top_left.y = anchor.top() - size.y - 10.0;
    }
    let rect = Rect::from_min_size(top_left, size);
    ui::slip::paint(painter, rect);
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme::DANGER),
        egui::StrokeKind::Inside,
    );
    let title_height = title.size().y;
    painter.galley(rect.min + vec2(pad, pad), title, theme::DANGER);
    painter.galley(
        rect.min + vec2(pad, pad + title_height + 4.0),
        body,
        theme::TEXT,
    );
}

pub(crate) fn beside(graph: &ScriptGraph, from: &str) -> Pos2 {
    let placed = graph.positions();
    let Some(origin) = placed.get(from) else {
        return graph::FIRST;
    };
    let mut pos = Pos2::new(origin.x + 280.0, origin.y);
    while placed
        .values()
        .any(|p| (p.x - pos.x).abs() < 200.0 && (p.y - pos.y).abs() < 60.0)
    {
        pos.y += 130.0;
    }
    pos
}
