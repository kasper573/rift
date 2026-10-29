//! Root-finds the highest probe count sustained within a per-tick budget. Shared by the in-process
//! benchmark (probing area counts) and the loadtest (probing player counts) so both converge with the
//! same handful of probes instead of crawling linearly.

pub const BUDGET_MS: f64 = 40.0;

pub struct Search {
    max: usize,
    under: Option<(usize, f64)>,
    over: Option<(usize, f64)>,
    previous: Option<(usize, f64)>,
}

impl Search {
    pub fn up_to(max: usize) -> Search {
        Search {
            max,
            under: None,
            over: None,
            previous: None,
        }
    }

    pub fn next_after(&mut self, count: usize, ms: f64) -> Option<usize> {
        let last = (count, ms);
        if ms <= BUDGET_MS {
            if self.under.is_none_or(|(highest, _)| count >= highest) {
                self.under = Some(last);
            }
        } else if self.over.is_none_or(|(lowest, _)| count <= lowest) {
            self.over = Some(last);
        }
        let next = project(self.max, self.under, self.over, self.previous, last);
        self.previous = Some(last);
        next
    }

    pub fn sustained(&self) -> Option<(usize, f64)> {
        self.under
    }
}

/// Stops once the projected next probe is only one count past the best sustained one — a finer answer
/// isn't worth a probe. With the budget bracketed it uses false position; before that it leaps straight
/// at the budget (secant of the last two probes, else one proportional guess), so a fast target is
/// bracketed at once.
fn project(
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
                ua as f64 + (BUDGET_MS - ut) * (oa - ua) as f64 / (ot - ut)
            } else {
                (ua + oa) as f64 / 2.0
            };
            (guess.round() as usize).clamp(ua + 1, oa - 1)
        }
        None => {
            let projected = match previous {
                Some((pa, pt)) if last.0 != pa && last.1 > pt => {
                    let slope = (last.1 - pt) / (last.0 - pa) as f64;
                    last.0 as f64 + (BUDGET_MS - last.1) / slope
                }
                _ => ua as f64 * BUDGET_MS / ut,
            };
            (projected.round() as usize).clamp(ua + 1, max)
        }
    };
    (next > ua + 1).then_some(next)
}
