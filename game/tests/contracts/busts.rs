use std::collections::HashSet;
use std::path::Path;

use game::systems::actor::bust::ModelDef;

use crate::support::content;

const BUST_CAP_BYTES: u64 = 120 * 1024;
const CAST_BUDGET_BYTES: u64 = 3 * 1024 * 1024;

#[test]
fn every_bust_fits_the_download_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
    let arts: HashSet<&str> = content()
        .table::<ModelDef>()
        .rows()
        .iter()
        .flat_map(|def| {
            def.busts()
                .into_iter()
                .flat_map(|busts| busts.all(content()))
        })
        .map(|bust| bust.0)
        .collect();
    let mut total = 0;
    for art in arts {
        let size = std::fs::metadata(root.join(art))
            .unwrap_or_else(|error| panic!("{art}: {error}"))
            .len();
        assert!(size <= BUST_CAP_BYTES, "{art} weighs {size} bytes");
        total += size;
    }
    assert!(total <= CAST_BUDGET_BYTES, "the busts weigh {total} bytes");
}
