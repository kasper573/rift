//! The game's spatial/math vocabulary, re-exported from the shared `math` crate so every domain
//! draws from one source. Game positions are `Pos<Tiles>`; map geometry crosses from `Pos<Pixels>`
//! through `Tiling`.
pub use math::{
    Direction, Millis, Pixels, PlaybackRate, Pos, Rect, Rng, Seconds, Size, Tiles, TilesPerSec,
    Tiling, Unit, next_rng, rng_unit,
};
