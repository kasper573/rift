use egui::Ui;

use super::{Candidate, Category, Dragged, Pick, PickSource};
use crate::forms::FormCx;
use crate::ui::assets::{self, AssetKind};

/// Files under `assets/` of the given kinds; several kinds become the popover's category chips.
pub struct AssetSource {
    kinds: Vec<AssetKind>,
}

impl AssetSource {
    pub fn of(kinds: &[AssetKind]) -> AssetSource {
        AssetSource {
            kinds: kinds.to_vec(),
        }
    }

    fn spoken(&self) -> String {
        self.kinds
            .iter()
            .map(|kind| kind.id().to_owned())
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

impl PickSource for AssetSource {
    fn placeholder(&self) -> String {
        format!("{} file", self.spoken())
    }

    fn categories(&self, _cx: &FormCx) -> Vec<Category> {
        if self.kinds.len() < 2 {
            return Vec::new();
        }
        self.kinds
            .iter()
            .map(|kind| Category {
                id: kind.id().to_owned(),
                label: kind.id().to_owned(),
            })
            .collect()
    }

    fn candidates(&self, cx: &FormCx) -> Vec<Candidate> {
        cx.files
            .paths_under("assets/")
            .filter_map(|file| file.strip_prefix("assets/"))
            .filter_map(|path| {
                let kind = kind_of(path).filter(|kind| self.kinds.contains(kind))?;
                Some(Candidate {
                    pick: Pick::new(kind.id(), path),
                    category: kind.id().to_owned(),
                    name: String::new(),
                    hint: path
                        .rsplit_once('/')
                        .map(|(folder, _)| folder)
                        .unwrap_or("")
                        .to_owned(),
                    blocked: None,
                })
            })
            .collect()
    }

    fn accepts(&self, _cx: &FormCx, dragged: &Dragged) -> Result<Pick, String> {
        let Dragged::Asset(path) = dragged else {
            return Err(format!(
                "this field takes {} files, not rows",
                self.spoken()
            ));
        };
        match kind_of(path) {
            Some(kind) if self.kinds.contains(&kind) => Ok(Pick::new(kind.id(), path.as_str())),
            _ => Err(format!(
                "this field takes {} files, not {path}",
                self.spoken()
            )),
        }
    }

    fn shown(&self, pick: &Pick) -> (String, String) {
        ("assets/".to_owned(), pick.key.clone())
    }

    fn thumbnail(&self, ui: &mut Ui, cx: &mut FormCx, pick: &Pick, size: f32) {
        match kind_of(&pick.key) {
            Some(AssetKind::Image) => {
                let ctx = ui.ctx().clone();
                if let Some(thumb) = cx.state.thumbnails.image(&ctx, cx.files, &pick.key) {
                    let [w, h] = thumb.size;
                    let scale = (size / h.max(1) as f32)
                        .min(size / w.max(1) as f32)
                        .min(1.0);
                    ui.add(egui::Image::from_texture((
                        thumb.texture.id(),
                        egui::vec2(w as f32 * scale, h as f32 * scale),
                    )));
                }
            }
            Some(AssetKind::Audio) => {
                ui.label(crate::ui::theme::muted("♪"));
            }
            Some(AssetKind::Font) => {
                ui.label(crate::ui::theme::muted("A"));
            }
            None => {}
        }
    }

    fn footer(&self, _cx: &FormCx, pick: &Pick) -> String {
        format!("assets/{}", pick.key)
    }
}

pub fn kind_of(path: &str) -> Option<AssetKind> {
    assets::kind(path)
}
