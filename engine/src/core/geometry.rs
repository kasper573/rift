use std::f32::consts::PI;

use bevy::math::Vec2;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::math::{Offset, Pos, Rect};
use crate::core::units::{Degrees, MeterPos, Meters, Radians};

const OUTLINE_STEPS: usize = 40;

/// The corners of a rectangle, going round it.
pub fn corners(rect: Rect<Meters>) -> [Pos<Meters>; 4] {
    let (min, max) = (rect.min(), rect.max());
    [
        Pos::new(min.x, min.y),
        Pos::new(max.x, min.y),
        Pos::new(max.x, max.y),
        Pos::new(min.x, max.y),
    ]
}

/// An actor's body for hit tests: a capsule standing on its feet, `radius` wide around its
/// position and `height` tall.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capsule {
    pub radius: Meters,
    pub height: Meters,
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
            Shape::Cone { length, .. } => length,
            Shape::Line { length, width } => {
                Meters((length.0 * length.0 + width.0 * width.0 / 4.0).sqrt())
            }
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

impl JsonSchema for PlacedShape {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        WirePlacedShape::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        WirePlacedShape::json_schema(generator)
    }
}

impl PlacedShape {
    /// `count` points spread evenly over the shape, the same every time: a sunflower spiral over
    /// the shape's reach, kept where it falls inside, gives the candidates; picks start at even
    /// strides through them and relax to the middle of the candidates nearest each, so they end up
    /// evenly apart and well inside whatever the shape is.
    pub fn spread(&self, count: u32) -> Vec<Pos<Meters>> {
        let reach = self.reach().0.max(0.1);
        let golden = PI * (3.0 - 5f32.sqrt());
        let inside: Vec<Pos<Meters>> = (0..SPREAD_CANDIDATES)
            .map(|index| {
                let radius = reach * ((index as f32 + 0.5) / SPREAD_CANDIDATES as f32).sqrt();
                let angle = index as f32 * golden;
                Pos::new(
                    self.at.x + radius * angle.cos(),
                    self.at.y + radius * angle.sin(),
                )
            })
            .filter(|point| self.contains(*point))
            .collect();
        if inside.is_empty() || count == 0 {
            return vec![self.at; count as usize];
        }
        let mut picks: Vec<usize> = (0..count)
            .map(|k| {
                let stride = (k as f32 + 0.5) * inside.len() as f32 / count as f32;
                (stride as usize).min(inside.len() - 1)
            })
            .collect();
        for _ in 0..SPREAD_ROUNDS {
            let mut sums = vec![(Vec2::ZERO, 0usize); picks.len()];
            for point in &inside {
                let owner = nearest_index(picks.iter().map(|pick| inside[*pick]), *point)
                    .expect("count is not zero, so there are picks");
                sums[owner].0 += Vec2::new(point.x, point.y);
                sums[owner].1 += 1;
            }
            for (pick, (sum, members)) in picks.iter_mut().zip(sums) {
                if members > 0 {
                    let mean = sum / members as f32;
                    *pick = nearest_index(inside.iter().copied(), Pos::new(mean.x, mean.y))
                        .expect("inside is not empty");
                }
            }
        }
        picks.into_iter().map(|pick| inside[pick]).collect()
    }

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
        Radians::SOUTH
    } else {
        Radians::of(toward - from)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy)]
#[schemars(rename = "PlacedShape")]
struct WirePlacedShape {
    #[schemars(with = "crate::core::math::PosSchema")]
    at: Pos<Meters>,
    facing: Radians,
    shape: WireShape,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy)]
#[schemars(rename = "WireShape")]
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

/// How many candidate points a spread picks among, and how many rounds its picks relax over.
const SPREAD_CANDIDATES: usize = 4096;
const SPREAD_ROUNDS: usize = 12;

fn nearest_index(points: impl Iterator<Item = Pos<Meters>>, to: Pos<Meters>) -> Option<usize> {
    points
        .enumerate()
        .min_by(|a, b| (a.1 - to).length().total_cmp(&(b.1 - to).length()))
        .map(|(index, _)| index)
}
