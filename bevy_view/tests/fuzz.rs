//! Property test: under any sequence of list edits (insert, remove, swap), the reconciler must
//! always leave the host's children exactly equal to the declared data, in order, and must keep a
//! stable entity for every key that survives an edit. Driven by a deterministic PRNG so failures
//! reproduce.

mod harness;

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use bevy_view::{each, text};
use harness::Ui;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() as usize) % bound
    }
}

fn run_sequence(seed: u64) {
    let mut ui = Ui::new();
    let mut rng = Lcg(seed);
    let mut model: Vec<u64> = Vec::new();
    let mut next_key = 1u64;
    let mut previous: HashMap<u64, Entity> = HashMap::new();

    for _ in 0..60 {
        match rng.next() % 3 {
            0 => {
                let position = rng.below(model.len() + 1);
                model.insert(position, next_key);
                next_key += 1;
            }
            1 if !model.is_empty() => {
                let position = rng.below(model.len());
                model.remove(position);
            }
            2 if model.len() >= 2 => {
                let a = rng.below(model.len());
                let b = rng.below(model.len());
                model.swap(a, b);
            }
            _ => {}
        }

        let data = model.clone();
        ui.render(each(
            move |_| data.clone(),
            |&key| key,
            |&key| text(key.to_string()),
        ));

        assert_eq!(
            ui.child_count(),
            model.len(),
            "seed {seed}: count must equal the data"
        );
        assert_eq!(
            ui.texts(),
            model.iter().map(|key| key.to_string()).collect::<Vec<_>>(),
            "seed {seed}: children must match the data in order",
        );

        let children = ui.children();
        let mut current = HashMap::new();
        for (index, &key) in model.iter().enumerate() {
            current.insert(key, children[index]);
        }
        for (key, entity) in &previous {
            if let Some(now) = current.get(key) {
                assert_eq!(
                    now, entity,
                    "seed {seed}: surviving key {key} must keep its entity"
                );
            }
        }
        previous = current;
    }
}

#[test]
fn list_edits_always_reconcile_to_the_declared_set_with_stable_identity() {
    for seed in [1, 2, 3, 7, 42, 1234, 99999, 0xDEAD_BEEF] {
        run_sequence(seed);
    }
}
