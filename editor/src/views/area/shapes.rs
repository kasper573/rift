use super::camera::AreaCanvas;
use engine::core::audio::soundscape::SoundscapeShape;
use engine::core::content::name_of;
use engine::core::math::{Offset, Pos, Rect, Size};
use engine::core::units::{Degrees, Meters};
use engine::systems::area::prop::Footprint;
use engine::systems::area::prop::Scatter;
use engine::systems::area::scene::SceneNode;
use engine::systems::area::{AreaModule, ObjectShape, sixteenth};
use ui::egui;
use ui::egui::Stroke as EStroke;

use crate::project::IdPaths;

/// Where `module` takes room on the map; a placement by what its blueprint brings, when known.
pub fn bounds(module: &AreaModule, placement_bounds: Option<Rect<Meters>>) -> Option<Rect<Meters>> {
    match module {
        AreaModule::Object(object) => Some(object.shape.bounds()),
        AreaModule::SafeZone(zone) => Some(zone.rect),
        AreaModule::Soundscape(soundscape) => Some(soundscape.shape.bounds()),
        AreaModule::Prop(prop) => Some(cell_rect(prop.at)),
        AreaModule::Scatter(scatter) => Some(scatter.region),
        AreaModule::Light(light) => Some(cell_rect(light.at)),
        AreaModule::Blueprint(placement) => {
            placement_bounds.or_else(|| Some(cell_rect(placement.at)))
        }
        AreaModule::Population(population) => population.roams,
        AreaModule::Resident(resident) => Some(cell_rect(resident.at)),
        AreaModule::Script(_) => None,
    }
}

pub(crate) fn is_ellipse(module: &AreaModule) -> bool {
    matches!(
        module,
        AreaModule::Soundscape(soundscape) if matches!(soundscape.shape, SoundscapeShape::Ellipse(_))
    )
}

pub(crate) fn is_point(module: &AreaModule) -> bool {
    match module {
        AreaModule::Object(object) => matches!(object.shape, ObjectShape::Point(_)),
        AreaModule::Resident(_) | AreaModule::Prop(_) | AreaModule::Light(_) => true,
        _ => false,
    }
}

pub(crate) fn cell_rect(at: Pos<Meters>) -> Rect<Meters> {
    Rect::new(at - Offset::new(0.5, 0.5), Size::splat(1.0))
}

/// Sizes the box of `module` `w` by `h`, snapped to sixteenths of a metre, when it has one.
pub fn resize(module: &mut AreaModule, w: f32, h: f32) {
    let size = Size::new(sixteenth(w.max(0.0625)), sixteenth(h.max(0.0625)));
    if let Some(rect) = sized_rect(module) {
        rect.size = size;
    }
}

pub(crate) fn has_size(module: &AreaModule) -> bool {
    match module {
        AreaModule::Object(object) => matches!(object.shape, ObjectShape::Rect(_)),
        AreaModule::Soundscape(_) | AreaModule::SafeZone(_) | AreaModule::Scatter(_) => true,
        AreaModule::Population(population) => population.roams.is_some(),
        _ => false,
    }
}

/// Turns `module` a quarter turn, clockwise or counter, when it has a way to face: a placement, a
/// prop or a resident.
pub fn turn(module: &mut AreaModule, clockwise: bool) {
    let quarter = Degrees(if clockwise { 90.0 } else { -90.0 });
    let turned = |yaw: Degrees| Degrees((yaw.0 + quarter.0).rem_euclid(360.0));
    match module {
        AreaModule::Blueprint(placement) => placement.yaw = turned(placement.yaw),
        AreaModule::Prop(prop) => prop.yaw = turned(prop.yaw),
        AreaModule::Resident(resident) => resident.facing = turned(resident.facing),
        _ => {}
    }
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
        AreaModule::Prop(prop) => format!("prop {}", named(paths.path_of(prop.prop))),
        AreaModule::Scatter(scatter) => format!("scatter of {}", scatter_names(scatter, paths)),
        AreaModule::Script(script) => format!("script {}", named(paths.path_of(script.script))),
        other => other.kind().tag().replace('_', " "),
    }
}

/// What a node reads as in a scene graph: its label, or what the module is: a placement its
/// blueprint, a resident its npc, a prop its file.
pub(crate) fn node_label(node: &SceneNode, paths: &impl IdPaths) -> String {
    if let Some(label) = node.label() {
        return label.to_owned();
    }
    let named = |path: Option<&str>| name_of(path.unwrap_or_default()).to_owned();
    let module = match node {
        SceneNode::Folder(_) => return "folder".to_owned(),
        SceneNode::Module(module) => &module.module,
    };
    match module {
        AreaModule::Object(object) => object.name.clone().unwrap_or_else(|| "object".to_owned()),
        AreaModule::Resident(resident) => named(paths.path_of(resident.npc)),
        AreaModule::Population(population) => named(paths.path_of(population.npc)),
        AreaModule::Prop(prop) => named(paths.path_of(prop.prop)),
        AreaModule::Scatter(scatter) => scatter_names(scatter, paths),
        AreaModule::Script(script) => named(paths.path_of(script.script)),
        AreaModule::Blueprint(placement) => named(paths.path_of(placement.blueprint)),
        other => other.kind().tag().replace('_', " "),
    }
}

/// What a module node's row says after its name: where a warp leads, how many a population keeps,
/// where a point stands, or how big a shape is.
pub(crate) fn node_detail(module: &AreaModule, paths: &impl IdPaths) -> String {
    if let AreaModule::Object(object) = module
        && let Some((area, _)) = super::warps::warp_destinations(object, paths)
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
        AreaModule::Prop(prop) => point(prop.at),
        AreaModule::Light(light) => point(light.at),
        AreaModule::Scatter(scatter) => format!("× {}", scatter.count()),
        AreaModule::Object(object) if is_point(module) => point(object.shape.bounds().center()),
        AreaModule::Blueprint(_) | AreaModule::Script(_) => String::new(),
        _ => bounds(module, None).map(size).unwrap_or_default(),
    }
}

fn scatter_names(scatter: &Scatter, paths: &impl IdPaths) -> String {
    let names: Vec<String> = scatter
        .props
        .iter()
        .map(|prop| name_of(paths.path_of(*prop).unwrap_or_default()).to_owned())
        .collect();
    match names.as_slice() {
        [] => "nothing".to_owned(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} +{}", rest.len()),
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
        AreaModule::Scatter(scatter) => Some(&mut scatter.region),
        AreaModule::Population(population) => population.roams.as_mut(),
        _ => None,
    }
}

/// A prop's footprint on the ground and, outside it, the outline the navmesh keeps walkers clear
/// of; a stake where the prop takes no ground.
pub(crate) fn draw_footprint(
    painter: &egui::Painter,
    canvas: &AreaCanvas,
    footprint: &Footprint,
    at: Pos<Meters>,
    yaw: Degrees,
    scale: f32,
    stroke: EStroke,
) {
    let outline = footprint.outline(at, yaw, scale);
    if outline.len() >= 3 {
        painter.add(egui::Shape::closed_line(canvas.closed(&outline), stroke));
        let grown = grown(&outline, canvas.area().bodies.radius.0);
        painter.add(egui::Shape::dashed_line(
            &canvas.closed(&grown),
            EStroke::new(1.0, ui::ink::wash(stroke.color, 0.5)),
            4.0,
            4.0,
        ));
    } else {
        painter.add(egui::Shape::closed_line(canvas.circle(at, 0.25), stroke));
    }
    let heading = at + yaw.rotate(engine::core::math::Offset::new(0.0, -0.5));
    painter.add(egui::Shape::line(canvas.polyline(&[at, heading]), stroke));
}

/// The polygon `outline` pushed out by `by` metres: each corner moved out along the bisector of
/// its two edges, as far as a mitre there needs.
fn grown(outline: &[Pos<Meters>], by: f32) -> Vec<Pos<Meters>> {
    let count = outline.len();
    if count < 3 {
        return outline.to_vec();
    }
    let signed_area: f32 = (0..count)
        .map(|i| {
            let (a, b) = (outline[i], outline[(i + 1) % count]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let winding = if signed_area >= 0.0 { 1.0 } else { -1.0 };
    let normal = |from: Pos<Meters>, to: Pos<Meters>| {
        let edge = to - from;
        let length = edge.length().max(f32::EPSILON);
        engine::core::math::Offset::new(edge.y, -edge.x) * (winding / length)
    };
    (0..count)
        .map(|i| {
            let previous = outline[(i + count - 1) % count];
            let here = outline[i];
            let next = outline[(i + 1) % count];
            let (before, after) = (normal(previous, here), normal(here, next));
            let along = (1.0 + before.dot(after)).max(0.2);
            here + (before + after) * (by / along)
        })
        .collect()
}
