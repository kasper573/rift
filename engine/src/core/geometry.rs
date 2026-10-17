use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::math::{Offset, Pos};
use crate::core::tiling::{TilePos, Tiles};

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
#[schemars(extend("x-unit" = "°"))]
pub struct Degrees(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Radians(pub f32);

impl Degrees {
    pub fn radians(self) -> Radians {
        Radians(self.0.to_radians())
    }
}

impl Radians {
    pub fn degrees(self) -> Degrees {
        Degrees(self.0.to_degrees())
    }

    pub fn of(heading: Offset<Tiles>) -> Radians {
        Radians(heading.y.atan2(heading.x))
    }

    pub fn heading(self) -> Offset<Tiles> {
        Offset::new(self.0.cos(), self.0.sin())
    }
}

/// A region in tiles, laid down at a point and turned toward a heading: the circle and the
/// rectangle sit centred on the point, the cone's apex and the line's start are the point.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Circle { radius: Tiles },
    Rect { width: Tiles, height: Tiles },
    Cone { angle: Degrees, length: Tiles },
    Line { length: Tiles, width: Tiles },
}

impl Shape {
    pub fn grown(self, by: Tiles) -> Shape {
        let grow = |tiles: Tiles| Tiles((tiles.0 + by.0).max(0.0));
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
    pub fn reach(self) -> Tiles {
        match self {
            Shape::Circle { radius } => radius,
            Shape::Rect { width, height } => {
                Tiles((width.0 * width.0 + height.0 * height.0).sqrt() / 2.0)
            }
            Shape::Cone { length, .. } | Shape::Line { length, .. } => length,
        }
    }

    pub fn describe(self) -> String {
        match self {
            Shape::Circle { radius } => format!("{} tiles around", radius.0),
            Shape::Rect { width, height } => format!("{} by {} tiles", width.0, height.0),
            Shape::Cone { angle, length } => {
                format!("a {}° cone, {} tiles long", angle.0, length.0)
            }
            Shape::Line { length, width } => {
                format!("a line {} tiles long, {} wide", length.0, width.0)
            }
        }
    }
}

/// A shape placed in the world. It travels the wire inside replicated components, and postcard
/// cannot read the internally tagged `Shape`, so it serializes through an externally tagged twin.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(from = "WireFootprint", into = "WireFootprint")]
pub struct Footprint {
    pub at: Pos<Tiles>,
    pub facing: Radians,
    pub shape: Shape,
}

impl Footprint {
    pub fn contains(&self, point: Pos<Tiles>) -> bool {
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

    pub fn reach(&self) -> Tiles {
        self.shape.reach()
    }

    pub fn outline(&self, steps: usize) -> Vec<Pos<Tiles>> {
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

    fn local(&self, offset: Offset<Tiles>) -> (f32, f32) {
        let heading = self.facing.heading();
        let along = offset.x * heading.x + offset.y * heading.y;
        let across = -offset.x * heading.y + offset.y * heading.x;
        (along, across)
    }
}

pub fn facing_from(from: Pos<Tiles>, toward: Pos<Tiles>) -> Radians {
    if from.distance(toward) == Tiles(0.0) {
        Radians(std::f32::consts::FRAC_PI_2)
    } else {
        Radians::of(toward - from)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct WireFootprint {
    at: Pos<Tiles>,
    facing: Radians,
    shape: WireShape,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
enum WireShape {
    Circle { radius: Tiles },
    Rect { width: Tiles, height: Tiles },
    Cone { angle: Degrees, length: Tiles },
    Line { length: Tiles, width: Tiles },
}

impl From<Footprint> for WireFootprint {
    fn from(footprint: Footprint) -> WireFootprint {
        let shape = match footprint.shape {
            Shape::Circle { radius } => WireShape::Circle { radius },
            Shape::Rect { width, height } => WireShape::Rect { width, height },
            Shape::Cone { angle, length } => WireShape::Cone { angle, length },
            Shape::Line { length, width } => WireShape::Line { length, width },
        };
        WireFootprint {
            at: footprint.at,
            facing: footprint.facing,
            shape,
        }
    }
}

impl From<WireFootprint> for Footprint {
    fn from(wire: WireFootprint) -> Footprint {
        let shape = match wire.shape {
            WireShape::Circle { radius } => Shape::Circle { radius },
            WireShape::Rect { width, height } => Shape::Rect { width, height },
            WireShape::Cone { angle, length } => Shape::Cone { angle, length },
            WireShape::Line { length, width } => Shape::Line { length, width },
        };
        Footprint {
            at: wire.at,
            facing: wire.facing,
            shape,
        }
    }
}
