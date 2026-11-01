use engine::core::math::Pos;
use engine::core::units::Meters;
use serde_json::Value;

use crate::views::area::nodes::CopiedNodes;

/// What was copied last: nodes of a scene, or modules of a row's list.
#[derive(Clone)]
pub enum Clipboard {
    Scene(SceneClip),
    Modules(ModuleClip),
}

/// Nodes copied out of the scene of the area or blueprint `from`, moved so what they cover starts
/// at the map's top-left; `origin` is where it started.
#[derive(Clone)]
pub struct SceneClip {
    pub from: String,
    pub nodes: CopiedNodes,
    pub count: usize,
    pub origin: Pos<Meters>,
}

/// Modules copied out of the list of the row `from`.
#[derive(Clone)]
pub struct ModuleClip {
    pub from: String,
    pub modules: Vec<Value>,
}

impl Clipboard {
    pub fn scene(&self) -> Option<&SceneClip> {
        match self {
            Clipboard::Scene(clip) => Some(clip),
            Clipboard::Modules(_) => None,
        }
    }
}
