use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::math::{Offset, Pos};
use crate::core::units::{Meters, TilePos};

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(extend("x-unit" = "°"))]
pub struct Degrees(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Radians(pub f32);

const OUTLINE_STEPS: usize = 40;

impl Degrees {
    pub fn radians(self) -> Radians {
        Radians(self.0.to_radians())
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }

    /// The angle as whole quarter turns, when it is one: a placement turns only by right angles,
    /// so what it places keeps its rectangles square to the world.
    pub fn quarter_turns(self) -> Option<u8> {
        let quarters = self.0 / 90.0;
        let whole = quarters.round();
        ((quarters - whole).abs() < 1e-4).then(|| whole.rem_euclid(4.0) as u8)
    }

    /// `offset` turned by the angle about the origin; a right angle turns it exactly.
    pub fn rotate(self, offset: Offset<Meters>) -> Offset<Meters> {
        let Offset { x, y, .. } = offset;
        match self.quarter_turns() {
            Some(0) => offset,
            Some(1) => Offset::new(-y, x),
            Some(2) => Offset::new(-x, -y),
            Some(3) => Offset::new(y, -x),
            _ => {
                let (sin, cos) = self.radians().0.sin_cos();
                Offset::new(x * cos - y * sin, x * sin + y * cos)
            }
        }
    }
}

impl Radians {
    /// Toward +y, which the camera looks down on: the way everyone first stands.
    pub const SOUTH: Radians = Radians(std::f32::consts::FRAC_PI_2);

    pub fn of(heading: Offset<Meters>) -> Radians {
        Radians(heading.y.atan2(heading.x))
    }

    pub fn heading(self) -> Offset<Meters> {
        Offset::new(self.0.cos(), self.0.sin())
    }
}

/// An actor's body for hit tests: a capsule standing on its feet, `radius` wide around its
/// position and `height` tall.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capsule {
    pub radius: Meters,
    pub height: Meters,
}

impl Capsule {
    /// Whether a point on the ground lies under the capsule standing at `at`.
    pub fn covers(self, at: Pos<Meters>, point: Pos<Meters>) -> bool {
        (point - at).length() <= self.radius.0
    }
}

/// A region in metres, laid down at a point and turned toward a heading: the circle and the
/// rectangle sit centred on the point, the cone's apex and the line's start are the point.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Circle {
        #[schemars(extend("default" = 2.0))]
        radius: Meters,
    },
    Rect {
        #[schemars(extend("default" = 3.0))]
        width: Meters,
        #[schemars(extend("default" = 3.0))]
        height: Meters,
    },
    Cone {
        #[schemars(extend("default" = 60.0))]
        angle: Degrees,
        #[schemars(extend("default" = 3.0))]
        length: Meters,
    },
    Line {
        #[schemars(extend("default" = 4.0))]
        length: Meters,
        #[schemars(extend("default" = 1.0))]
        width: Meters,
    },
}

impl Shape {
    pub fn grown(self, by: Meters) -> Shape {
        let grow = |metres: Meters| Meters((metres.0 + by.0).max(0.0));
        match self {
            Shape::Circle { radius } => Shape::Circle {
                radius: grow(radius),
            },
            Shape::Rect { width, height } => Shape::Rect {
                width: grow(width),
                height: grow(height),
            },
            Shape::Cone { angle, length } => Shape::Cone {
                angle,
                length: grow(length),
            },
            Shape::Line { length, width } => Shape::Line {
                length: grow(length),
                width,
            },
        }
    }

    /// The farthest a point of the shape lies from its anchor.
    pub fn reach(self) -> Meters {
        match self {
            Shape::Circle { radius } => radius,
            Shape::Rect { width, height } => {
                Meters((width.0 * width.0 + height.0 * height.0).sqrt() / 2.0)
            }
            Shape::Cone { length, .. } | Shape::Line { length, .. } => length,
        }
    }

    pub fn describe(self) -> String {
        match self {
            Shape::Circle { radius } => format!("{} m around", radius.0),
            Shape::Rect { width, height } => format!("{} by {} m", width.0, height.0),
            Shape::Cone { angle, length } => {
                format!("a {}° cone, {} m long", angle.0, length.0)
            }
            Shape::Line { length, width } => {
                format!("a line {} m long, {} wide", length.0, width.0)
            }
        }
    }
}

/// A shape placed in the world. It travels the wire inside replicated components, and postcard
/// cannot read the internally tagged `Shape`, so it serializes through an externally tagged twin.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(from = "WirePlacedShape", into = "WirePlacedShape")]
pub struct PlacedShape {
    pub at: Pos<Meters>,
    pub facing: Radians,
    pub shape: Shape,
}

impl PlacedShape {
    /// Where a cast from `from` toward `point` lays a shape down: a circle or a rectangle sits on
    /// the point, a cone or a line reaches out from the caster toward it.
    pub fn cast(shape: Shape, from: Pos<Meters>, point: Pos<Meters>) -> PlacedShape {
        let at = match shape {
            Shape::Cone { .. } | Shape::Line { .. } => from,
            Shape::Circle { .. } | Shape::Rect { .. } => point,
        };
        PlacedShape {
            at,
            facing: facing_from(from, point),
            shape,
        }
    }

    pub fn contains(&self, point: Pos<Meters>) -> bool {
        let offset = point - self.at;
        match self.shape {
            Shape::Circle { radius } => offset.length() <= radius.0,
            Shape::Rect { width, height } => {
                let (along, across) = self.local(offset);
                along.abs() <= width.0 / 2.0 && across.abs() <= height.0 / 2.0
            }
            Shape::Cone { angle, length } => {
                let distance = offset.length();
                if distance > length.0 {
                    return false;
                }
                if distance == 0.0 {
                    return true;
                }
                let (along, across) = self.local(offset);
                let turned = across.atan2(along).abs();
                turned <= angle.radians().0 / 2.0
            }
            Shape::Line { length, width } => {
                let (along, across) = self.local(offset);
                (0.0..=length.0).contains(&along) && across.abs() <= width.0 / 2.0
            }
        }
    }

    pub fn reach(&self) -> Meters {
        self.shape.reach()
    }

    /// The points around the shape's edge, enough for a drawn ring or a bounding box.
    pub fn outline(&self) -> Vec<Pos<Meters>> {
        let steps = OUTLINE_STEPS;
        let heading = self.facing.heading();
        let side = Offset::new(-heading.y, heading.x);
        let along = |a: f32, b: f32| self.at + heading * a + side * b;
        match self.shape {
            Shape::Circle { radius } => (0..steps)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / steps as f32;
                    self.at + Offset::new(angle.cos(), angle.sin()) * radius.0
                })
                .collect(),
            Shape::Rect { width, height } => {
                let (w, h) = (width.0 / 2.0, height.0 / 2.0);
                vec![along(-w, -h), along(w, -h), along(w, h), along(-w, h)]
            }
            Shape::Cone { angle, length } => {
                let half = angle.radians().0 / 2.0;
                let arc = steps.max(2);
                std::iter::once(self.at)
                    .chain((0..=arc).map(|step| {
                        let turn = -half + angle.radians().0 * step as f32 / arc as f32;
                        along(turn.cos() * length.0, turn.sin() * length.0)
                    }))
                    .collect()
            }
            Shape::Line { length, width } => {
                let w = width.0 / 2.0;
                vec![
                    along(0.0, -w),
                    along(length.0, -w),
                    along(length.0, w),
                    along(0.0, w),
                ]
            }
        }
    }

    fn local(&self, offset: Offset<Meters>) -> (f32, f32) {
        let heading = self.facing.heading();
        let along = offset.x * heading.x + offset.y * heading.y;
        let across = -offset.x * heading.y + offset.y * heading.x;
        (along, across)
    }
}

pub fn facing_from(from: Pos<Meters>, toward: Pos<Meters>) -> Radians {
    if from.distance(toward) == Meters(0.0) {
        Radians(std::f32::consts::FRAC_PI_2)
    } else {
        Radians::of(toward - from)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct WirePlacedShape {
    at: Pos<Meters>,
    facing: Radians,
    shape: WireShape,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
enum WireShape {
    Circle { radius: Meters },
    Rect { width: Meters, height: Meters },
    Cone { angle: Degrees, length: Meters },
    Line { length: Meters, width: Meters },
}

impl From<PlacedShape> for WirePlacedShape {
    fn from(placed: PlacedShape) -> WirePlacedShape {
        let shape = match placed.shape {
            Shape::Circle { radius } => WireShape::Circle { radius },
            Shape::Rect { width, height } => WireShape::Rect { width, height },
            Shape::Cone { angle, length } => WireShape::Cone { angle, length },
            Shape::Line { length, width } => WireShape::Line { length, width },
        };
        WirePlacedShape {
            at: placed.at,
            facing: placed.facing,
            shape,
        }
    }
}

impl From<WirePlacedShape> for PlacedShape {
    fn from(wire: WirePlacedShape) -> PlacedShape {
        let shape = match wire.shape {
            WireShape::Circle { radius } => Shape::Circle { radius },
            WireShape::Rect { width, height } => Shape::Rect { width, height },
            WireShape::Cone { angle, length } => Shape::Cone { angle, length },
            WireShape::Line { length, width } => Shape::Line { length, width },
        };
        PlacedShape {
            at: wire.at,
            facing: wire.facing,
            shape,
        }
    }
}
