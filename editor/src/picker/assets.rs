use engine::core::content::AssetKind;

use super::{Dragged, PickLayout, PickSource, PickedKey, PickerCandidate, PickerCategory};
use crate::forms::FormCx;
use crate::icons::{self, KindIcon};
use crate::moves::file_name;

/// Media files of one kind, wherever they sit outside a bundle.
pub struct AssetSource {
    kind: AssetKind,
}

impl AssetSource {
    pub fn of(kind: AssetKind) -> AssetSource {
        AssetSource { kind }
    }

    fn spoken(&self) -> &'static str {
        self.kind.into()
    }
}

impl PickSource for AssetSource {
    fn placeholder(&self) -> String {
        format!("{} file", self.spoken())
    }

    fn categories(&self, _cx: &FormCx) -> Vec<PickerCategory> {
        Vec::new()
    }

    fn candidates(&self, cx: &FormCx) -> Vec<PickerCandidate> {
        let index = cx.files.index();
        index
            .media
            .iter()
            .filter(|path| AssetKind::of(path) == Some(self.kind))
            .map(|path| {
                let (nearness, hint) = cx.seen_from_here(path);
                PickerCandidate {
                    pick: PickedKey::new(self.spoken(), path),
                    category: self.spoken().to_owned(),
                    name: String::new(),
                    hint,
                    blocked: None,
                    nearness,
                }
            })
            .collect()
    }

    fn accepts(&self, _cx: &FormCx, dragged: &Dragged) -> Result<PickedKey, String> {
        let Dragged::Asset(path) = dragged else {
            return Err(format!(
                "this field takes {} files, not content files",
                self.spoken()
            ));
        };
        if AssetKind::of(path) != Some(self.kind) {
            return Err(format!(
                "this field takes {} files, not {path}",
                self.spoken()
            ));
        }
        Ok(PickedKey::new(self.spoken(), path.as_str()))
    }

    fn shown(&self, pick: &PickedKey) -> (String, String) {
        match pick.key.rsplit_once('/') {
            Some((folder, file)) => (format!("{folder}/"), file.to_owned()),
            None => (String::new(), pick.key.clone()),
        }
    }

    fn short(&self, pick: &PickedKey) -> String {
        file_name(&pick.key).to_owned()
    }

    fn icon(&self, pick: &PickedKey) -> KindIcon {
        icons::of_media(&pick.key)
    }

    fn describe(&self, cx: &FormCx, pick: &PickedKey) -> String {
        let uses = cx.usages.of_media(&pick.key).len();
        format!("used by {uses}")
    }

    fn layout(&self) -> PickLayout {
        match self.kind {
            AssetKind::Image => PickLayout::Grid,
            _ => PickLayout::List,
        }
    }
}
