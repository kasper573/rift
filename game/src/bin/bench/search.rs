//! Root-finds the highest probe count sustained within a per-tick budget. Shared by the in-process
//! benchmark (probing area counts) and the loadtest (probing player counts) so both converge with the
//! same handful of probes instead of crawling linearly.

/// The next count to probe, or `None` to stop. Stops once the projected next probe is only one count
/// past the best sustained one — a finer answer isn't worth a probe. With the budget bracketed it uses
/// false position; before that it leaps straight at the budget (secant of the last two probes, else one
/// proportional guess), so a fast target is bracketed at once. Generic over what the count means.
pub fn project(
    budget_ms: f64,
    max: usize,
    under: Option<(usize, f64)>,
    over: Option<(usize, f64)>,
    previous: Option<(usize, f64)>,
    last: (usize, f64),
) -> Option<usize> {
    let (ua, ut) = under?;
    if ua >= max {
        return None;
    }
    let next = match over {
        Some((oa, ot)) => {
            if oa <= ua + 1 {
                return None;
            }
            let guess = if ot > ut {
                ua as f64 + (budget_ms - ut) * (oa - ua) as f64 / (ot - ut)
            } else {
                (ua + oa) as f64 / 2.0
            };
            (guess.round() as usize).clamp(ua + 1, oa - 1)
        }
        None => {
            let projected = match previous {
                Some((pa, pt)) if last.0 != pa && last.1 > pt => {
                    let slope = (last.1 - pt) / (last.0 - pa) as f64;
                    last.0 as f64 + (budget_ms - last.1) / slope
                }
                _ => ua as f64 * budget_ms / ut,
            };
            (projected.round() as usize).clamp(ua + 1, max)
        }
    };
    (next > ua + 1).then_some(next)
}
