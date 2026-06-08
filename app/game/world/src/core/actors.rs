//! The embedded actor registry: every `assets/actors/<name>/manifest.json` is one actor model,
//! sorted by name — the index is the wire id. Loading and rendering live in `actor`.

use std::sync::OnceLock;

use actor::ActorModel;
use rift::Wire;
use serde::{Deserialize, Deserializer};

use crate::core::assets;
use crate::core::protocol::Hitbox;

/// An actor model's index in [`models`]; content tables reference models by name.
#[derive(Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ActorModelId(pub u16);

impl<'de> Deserialize<'de> for ActorModelId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        model_index(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown actor model '{name}'")))
    }
}

pub fn models() -> &'static [ActorModel] {
    static MODELS: OnceLock<Vec<ActorModel>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let mut all: Vec<ActorModel> = assets::dir(assets::ACTORS)
            .filter_map(|(name, bytes)| {
                let folder = name[assets::ACTORS.len() + 1..].strip_suffix("/manifest.json")?;
                if folder.contains('/') {
                    return None;
                }
                let manifest = std::str::from_utf8(bytes)
                    .unwrap_or_else(|_| panic!("actor model {folder}'s manifest is not utf-8"));
                Some(ActorModel::load(folder, manifest, |file| {
                    assets::bytes(&format!("{}/{folder}/{file}", assets::ACTORS))
                }))
            })
            .collect();
        all.sort_unstable_by(|a, b| a.name().cmp(b.name()));
        all
    })
}

pub fn model_index(name: &str) -> Option<ActorModelId> {
    models()
        .iter()
        .position(|model| model.name() == name)
        .map(|index| ActorModelId(index as u16))
}

pub fn model_hitbox(model: ActorModelId) -> Hitbox {
    let size = models()[model.0 as usize].hitbox();
    Hitbox { size }
}
