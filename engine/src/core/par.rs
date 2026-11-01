//! The grain of parallel work: the fewest items a job of a parallel iteration takes, so a pass over
//! a few thousand cheap items isn't cut into jobs that cost more to hand out than to run.

pub const GRAIN: usize = 128;
