use engine::core::audio::soundscape::SoundscapeShape;
use engine::core::content::name_of;
use engine::core::math::{Offset, Pos, Rect, Size};
use engine::core::units::Meters;
use engine::systems::area::scene::SceneNode;
use engine::systems::area::{AreaModule, Flip, ObjectShape, Scenery};

use crate::project::IdPaths;

pub fn bounds(module: &AreaModule, placement_bounds: Option<Rect<Meters>>) -> Option<Rect<Meters>> {
    match module {
        AreaModule::Object(object) => Some(object.shape.bounds()),
        AreaModule::SafeZone(zone) => Some(zone.rect),
        AreaModule::Soundscape(soundscape) => Some(soundscape.shape.bounds()),
        AreaModule::Scenery(scenery) => Some(scenery.bounds()),
        AreaModule::Blueprint(placement) => {
            placement_bounds.or_else(|| Some(cell_rect(placement.at)))
        }
        AreaModule::Population(population) => population.roams,
        AreaModule::Resident(resident) => Some(cell_rect(resident.at)),
        AreaModule::Script(_) => None,
    }
}

pub fn is_ellipse(module: &AreaModule) -> bool {
    matches!(
        module,
        AreaModule::Soundscape(soundscape) if matches!(soundscape.shape, SoundscapeShape::Ellipse(_))
    )
}

pub fn is_point(module: &AreaModule) -> bool {
    match module {
        AreaModule::Object(object) => matches!(object.shape, ObjectShape::Point(_)),
        AreaModule::Resident(_) => true,
        _ => false,
    }
}

pub fn cell_rect(at: Pos<Meters>) -> Rect<Meters> {
    Rect::new(at - Offset::new(0.5, 0.5), Size::splat(1.0))
}

pub fn scenery_rect(scenery: &Scenery) -> Rect<Meters> {
    scenery.bounds()
}

/// Moves what places `module` on the map by `dx`, `dy`, snapped to sixteenths of a tile.
pub fn shift(module: &mut AreaModule, dx: f32, dy: f32) {
    let moved = |at: Pos<Meters>| Pos::new(sixteenth(at.x + dx), sixteenth(at.y + dy));
    let moved_rect = |rect: Rect<Meters>| Rect::new(moved(rect.origin), rect.size);
    match module {
        AreaModule::Object(object) => {
            object.shape = match object.shape {
                ObjectShape::Point(at) => ObjectShape::Point(moved(at)),
                ObjectShape::Rect(rect) => ObjectShape::Rect(moved_rect(rect)),
            };
        }
        AreaModule::Soundscape(soundscape) => {
            soundscape.shape = match soundscape.shape {
                SoundscapeShape::Rect(rect) => SoundscapeShape::Rect(moved_rect(rect)),
                SoundscapeShape::Ellipse(rect) => SoundscapeShape::Ellipse(moved_rect(rect)),
            };
        }
        AreaModule::SafeZone(zone) => zone.rect = moved_rect(zone.rect),
        AreaModule::Scenery(scenery) => scenery.at = moved(scenery.at),
        AreaModule::Blueprint(placement) => placement.at = moved(placement.at),
        AreaModule::Resident(resident) => resident.at = moved(resident.at),
        AreaModule::Population(population) => population.roams = population.roams.map(moved_rect),
        AreaModule::Script(_) => {}
    }
}

/// Sizes the box of `module` `w` by `h`, snapped to sixteenths of a tile, when it has one.
pub fn resize(module: &mut AreaModule, w: f32, h: f32) {
    let size = Size::new(sixteenth(w.max(0.0625)), sixteenth(h.max(0.0625)));
    if let Some(rect) = sized_rect(module) {
        rect.size = size;
    }
}

pub fn has_size(module: &AreaModule) -> bool {
    match module {
        AreaModule::Object(object) => matches!(object.shape, ObjectShape::Rect(_)),
        AreaModule::Soundscape(_) | AreaModule::SafeZone(_) => true,
        AreaModule::Population(population) => population.roams.is_some(),
        _ => false,
    }
}

/// Turns the flip of a scenery or a placement over the axis `horizontal` names.
pub fn flip_field(module: &mut AreaModule, horizontal: bool) {
    let turn = Flip::of(horizontal, !horizontal);
    match module {
        AreaModule::Scenery(scenery) => scenery.flip = scenery.flip.compose(turn),
        AreaModule::Blueprint(placement) => placement.flip = placement.flip.compose(turn),
        _ => {}
    }
}

pub fn sixteenth(v: f32) -> f32 {
    (v * 16.0).round() / 16.0
}

/// What a module is, said by what it names.
pub fn label(module: &AreaModule, paths: &impl IdPaths) -> String {
    let named = |path: Option<&str>| name_of(path.unwrap_or_default()).to_owned();
    match module {
        AreaModule::Object(object) => object
            .name
            .as_ref()
            .map_or("object".to_owned(), |name| format!("object {name}")),
        AreaModule::Blueprint(placement) => {
            format!("{} placement", named(paths.path_of(placement.blueprint)))
        }
        AreaModule::Population(population) => format!(
            "{} × {}",
            named(paths.path_of(population.npc)),
            population.count
        ),
        AreaModule::Resident(resident) => {
            format!("resident {}", named(paths.path_of(resident.npc)))
        }
        AreaModule::Scenery(scenery) => {
            format!("scenery {}", named(paths.path_of(scenery.tileset)))
        }
        AreaModule::Script(script) => format!("script {}", named(paths.path_of(script.script))),
        other => other.kind().tag().replace('_', " "),
    }
}

/// What a node reads as in a scene graph: its label, a ground layer's name, or what the module is:
/// a stamp its tileset, a resident its npc.
pub fn node_label(node: &SceneNode, paths: &impl IdPaths) -> String {
    if let Some(label) = node.label() {
        return label.to_owned();
    }
    let named = |path: Option<&str>| name_of(path.unwrap_or_default()).to_owned();
    let module = match node {
        SceneNode::GroundLayer(layer) => return layer.name.clone(),
        SceneNode::Folder(_) => return "folder".to_owned(),
        SceneNode::Module(module) => &module.module,
    };
    match module {
        AreaModule::Object(object) => object.name.clone().unwrap_or_else(|| "object".to_owned()),
        AreaModule::Resident(resident) => named(paths.path_of(resident.npc)),
        AreaModule::Population(population) => named(paths.path_of(population.npc)),
        AreaModule::Scenery(scenery) => named(paths.path_of(scenery.tileset)),
        AreaModule::Script(script) => named(paths.path_of(script.script)),
        AreaModule::Blueprint(placement) => named(paths.path_of(placement.blueprint)),
        other => other.kind().tag().replace('_', " "),
    }
}

/// What a module node's row says after its name: where a warp leads, how many a population keeps,
/// where a point stands, or how big a shape is.
pub fn node_detail(module: &AreaModule, paths: &impl IdPaths) -> String {
    if let AreaModule::Object(object) = module
        && let Some((area, _)) = super::placement::warp_destinations(object, paths)
            .into_iter()
            .next()
    {
        return format!("→ {}", name_of(&area));
    }
    let size = |rect: Rect<Meters>| {
        format!(
            "{} × {}",
            crate::forms::metres(rect.size.width),
            crate::forms::metres(rect.size.height)
        )
    };
    let point = |at: Pos<Meters>| {
        format!(
            "{}, {}",
            crate::forms::metres(at.x),
            crate::forms::metres(at.y)
        )
    };
    match module {
        AreaModule::Population(population) => format!("× {}", population.count),
        AreaModule::Resident(resident) => point(resident.at),
        AreaModule::Object(object) if is_point(module) => point(object.shape.bounds().center()),
        AreaModule::Blueprint(_) | AreaModule::Script(_) => String::new(),
        _ => bounds(module, None).map(size).unwrap_or_default(),
    }
}

fn sized_rect(module: &mut AreaModule) -> Option<&mut Rect<Meters>> {
    match module {
        AreaModule::Object(object) => match &mut object.shape {
            ObjectShape::Rect(rect) => Some(rect),
            ObjectShape::Point(_) => None,
        },
        AreaModule::Soundscape(soundscape) => match &mut soundscape.shape {
            SoundscapeShape::Rect(rect) | SoundscapeShape::Ellipse(rect) => Some(rect),
        },
        AreaModule::SafeZone(zone) => Some(&mut zone.rect),
        AreaModule::Population(population) => population.roams.as_mut(),
        _ => None,
    }
}
