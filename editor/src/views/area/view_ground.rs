//! The ground an area view holds: the tiles around where its camera looks, as far out as it sees
//! up to a few tiles, so the meshes its overlays and previews read cover what is in view however
//! wide the area, and the ground left behind is let go.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use engine::core::ground::TILE_SIDE;
use engine::core::units::Meters;
use engine::systems::area::{Area, GroundHold};

use super::camera::OrbitCamera;

/// How far from where the camera looks a view holds ground at most.
const FARTHEST: Meters = Meters(TILE_SIDE.0 * 3.0);

/// The ground held, and the hold under way, which lands its ground in its slot once laid.
#[derive(Default)]
pub struct ViewGround {
    held: Option<GroundHold>,
    holding: Option<Arc<Mutex<Option<GroundHold>>>>,
}

impl ViewGround {
    /// Holds the ground of `area` around where `camera` looks, once the hold under way is done,
    /// letting go of what was held for a view elsewhere.
    pub fn follow(&mut self, area: &Arc<Area>, camera: &OrbitCamera) {
        if let Some(holding) = &self.holding {
            let Some(held) = holding.lock().expect("a view's ground hold").take() else {
                return;
            };
            self.held = Some(held);
            self.holding = None;
        }
        let reach = Meters(camera.distance.0.clamp(TILE_SIDE.0, FARTHEST.0));
        let wanted: BTreeSet<_> = area.tiles_around(camera.target, reach).collect();
        if self
            .held
            .as_ref()
            .is_some_and(|held| held.holds_on(area) && *held.tiles() == wanted)
        {
            return;
        }
        let previous = self.held.take();
        let area = area.clone();
        let landing = Arc::new(Mutex::new(None));
        self.holding = Some(landing.clone());
        crate::project::background().spawn(move || {
            let held = area.hold_instead(previous, wanted);
            *landing.lock().expect("a view's ground hold") = Some(held);
        });
    }
}
