//! The grain of parallel work: how finely a pass over many cheap items is cut into jobs. Too few
//! jobs leave cores idle behind the one slow item (a route to plan, a crowd to scan); too many cost
//! more to hand out than to run. A few jobs per core balances the two.

use std::sync::OnceLock;

/// Starts the pool every parallel pass runs on, with a worker per physical core: hyperthreads
/// share a core's execution units, and waking twice the workers costs a pass more than they
/// bring. `RAYON_NUM_THREADS` chooses otherwise. A pool started already stays.
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let mut pool = rayon::ThreadPoolBuilder::new().thread_name(|index| format!("par-{index}"));
        if std::env::var_os("RAYON_NUM_THREADS").is_none() {
            pool = pool.num_threads(num_cpus::get_physical());
        }
        let _ = pool.build_global();
    });
}

/// Whether a pass over `len` items is too small to hand to the pool: handing it out would cost
/// more than the pass itself, so it runs where it's called.
pub fn few(len: usize) -> bool {
    len < FEW
}

/// The fewest items a job of a pass over `len` items takes.
pub fn grain(len: usize) -> usize {
    let jobs = JOBS_PER_THREAD * rayon::current_num_threads();
    len.div_ceil(jobs).max(1)
}

const JOBS_PER_THREAD: usize = 2;
const FEW: usize = 32;
