use engine::core::math::Rng;
use engine::core::time::Backoff;
use engine::core::units::Seconds;
use testkit::drawn_seed;

const BACKOFF: Backoff = Backoff {
    first: Seconds(1.0),
    longest: Seconds(30.0),
};

#[test]
fn each_failure_doubles_the_wait_up_to_the_longest_give_or_take_half() {
    let mut rng = Rng::new(drawn_seed());
    for (failures, base) in [
        (1, 1.0),
        (2, 2.0),
        (3, 4.0),
        (4, 8.0),
        (5, 16.0),
        (6, 30.0),
        (40, 30.0),
    ] {
        for _ in 0..100 {
            let wait = BACKOFF.after(failures, &mut rng).0;
            assert!(
                (base * 0.5..=base * 1.5).contains(&wait),
                "after {failures} failures the wait was {wait}s"
            );
        }
    }
}

#[test]
fn clients_that_fail_together_spread_out_their_next_tries() {
    let mut rng = Rng::new(drawn_seed());
    let waits: Vec<f32> = (0..20).map(|_| BACKOFF.after(3, &mut rng).0).collect();
    let soonest = waits.iter().copied().fold(f32::INFINITY, f32::min);
    let latest = waits.iter().copied().fold(0.0, f32::max);
    assert!(
        latest - soonest > 1.0,
        "twenty clients all retry within {}s",
        latest - soonest
    );
}
