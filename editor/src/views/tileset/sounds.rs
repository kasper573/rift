use engine::core::audio::playback::{SfxDef, SfxScalar};
use engine::systems::area::tileset::SurfaceDef;
use ui::egui;
use ui::egui::{Color32, Rect as ERect};

use crate::app::Editor;
use crate::audio::Waveform;

pub fn wave_of_sfx(editor: &mut Editor, ctx: &egui::Context, sfx: &str) -> Option<Waveform> {
    let path = sfx_file(editor, sfx)?;
    let Editor { project, views, .. } = editor;
    let project = project.as_ref()?;
    views.model.bank.wave(ctx, &project.files, &path)
}

/// The clip the sfx at `sfx` plays.
pub fn sfx_file(editor: &Editor, sfx: &str) -> Option<String> {
    let sfx = editor.read::<SfxDef>(sfx)?.ok()?;
    Some(sfx.file.as_str().to_owned())
}

pub fn draw_wave(painter: &egui::Painter, rect: ERect, bins: &[f32], color: Color32) {
    if bins.is_empty() || rect.width() <= 0.0 {
        return;
    }
    let mid = rect.center().y;
    let half = rect.height() / 2.0;
    let mut top = Vec::with_capacity(bins.len());
    let mut bottom = Vec::with_capacity(bins.len());
    for (index, amplitude) in bins.iter().enumerate() {
        let x = rect.min.x + rect.width() * index as f32 / (bins.len() - 1).max(1) as f32;
        let h = (amplitude.min(1.0) * half).max(0.5);
        top.push(egui::pos2(x, mid - h));
        bottom.push(egui::pos2(x, mid + h));
    }
    bottom.reverse();
    top.extend(bottom);
    painter.add(egui::Shape::convex_polygon(
        top,
        color.gamma_multiply(0.85),
        egui::Stroke::NONE,
    ));
}

pub fn first_sfx(editor: &Editor) -> Option<String> {
    sfx_keys(editor).into_iter().next()
}

pub fn sfx_keys(editor: &Editor) -> Vec<String> {
    editor
        .reader()
        .map(|reader| reader.paths_of::<SfxDef>().to_vec())
        .unwrap_or_default()
}

pub struct FootstepSurface {
    pub key: String,
    pub name: String,
    pub footstep: String,
}

pub fn surfaces(editor: &Editor) -> Vec<FootstepSurface> {
    let Some(reader) = editor.reader() else {
        return Vec::new();
    };
    reader
        .paths_of::<SurfaceDef>()
        .iter()
        .filter_map(|key| {
            let surface = editor.read::<SurfaceDef>(key)?.ok()?;
            Some(FootstepSurface {
                name: surface.name.clone(),
                key: key.clone(),
                footstep: reader.path_of(surface.footstep)?.to_owned(),
            })
        })
        .collect()
}

pub fn footstep_of(editor: &Editor, surface: Option<&str>) -> Option<String> {
    let surfaces = surfaces(editor);
    let chosen = match surface {
        Some(key) => surfaces.iter().find(|s| s.key == key),
        None => surfaces.first(),
    };
    chosen.map(|surface| surface.footstep.clone())
}

pub fn volume_of(editor: &Editor, sfx: &str) -> String {
    scalar_text(scalar_of(editor, sfx, |sfx| sfx.volume))
}

pub fn pitch_of(editor: &Editor, sfx: &str) -> String {
    scalar_text(scalar_of(editor, sfx, |sfx| sfx.pitch))
}

fn scalar_of(editor: &Editor, sfx: &str, field: impl Fn(&SfxDef) -> SfxScalar) -> SfxScalar {
    editor
        .read::<SfxDef>(sfx)
        .and_then(Result::ok)
        .map(|sfx| field(&sfx))
        .unwrap_or_default()
}

fn scalar_text(scalar: SfxScalar) -> String {
    match scalar {
        SfxScalar::Fixed(fixed) => format!("{fixed} · the row's"),
        SfxScalar::Random { min, max } => format!("{min} to {max}, at random · the row's"),
    }
}
