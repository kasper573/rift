use bevy::prelude::*;
use engine::core::units::{Meters, MetersPerSec, Seconds};
use engine::systems::area::shown_weather::TreadingLimb;
use engine::systems::water::ripples::{LimbsInWater, RippleField, Ripples};

const RIPPLES: Ripples = Ripples {
    speed: MetersPerSec(Meters(1.0)),
    lasts: Seconds(6.0),
    lasts_in_gale: Seconds(1.0),
    froth: Seconds(2.0),
    froths_at: MetersPerSec(Meters(4.0)),
};
const CALM: f32 = 0.0;
const GALE: f32 = 1.0;
const FRAME: Seconds = Seconds(1.0 / 60.0);
const LEVEL: f32 = 2.0;

fn pass(field: &mut RippleField, seconds: f32, wind: f32) {
    for _ in 0..(seconds / FRAME.0).round() as usize {
        field.pass(FRAME, &RIPPLES, wind);
    }
}

/// How far from `middle` the water stands highest along `way`, and how high.
fn crest(field: &RippleField, middle: Vec2, way: Vec2) -> (f32, f32) {
    (0..200)
        .map(|step| step as f32 * 0.04)
        .map(|out| (out, field.height_at(middle + way * out)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a ray of the field")
}

/// A leg standing through the surface at the ground point `at`.
fn leg(at: Vec2) -> TreadingLimb {
    TreadingLimb {
        from: Vec3::new(at.x, LEVEL + 0.5, at.y),
        to: Vec3::new(at.x, LEVEL - 0.5, at.y),
        girth: 0.1,
        tail_girth: 0.1,
    }
}

fn wading(limbs: &[TreadingLimb]) -> Option<LimbsInWater<'_>> {
    Some(LimbsInWater {
        limbs,
        level: LEVEL,
    })
}

#[test]
fn water_broken_at_a_place_sends_a_ring_outward_at_the_ripples_speed_that_fades() {
    let mut still = RippleField::default();
    pass(&mut still, 1.0, CALM);
    assert_eq!(crest(&still, Vec2::ZERO, Vec2::X).1, 0.0);

    let struck = Vec2::new(1.0, -2.0);
    let ways = [Vec2::X, Vec2::NEG_Y, Vec2::ONE.normalize()];
    let after = |seconds: f32, wind: f32| {
        let mut field = RippleField::default();
        field.follow(struck);
        field.strike(struck, Meters(0.5), 1.0);
        pass(&mut field, seconds, wind);
        ways.map(|way| crest(&field, struck, way))
    };
    for (seconds, out) in [(1.0, 1.0), (3.0, 3.0)] {
        for (reached, high) in after(seconds, CALM) {
            assert!(
                (reached - out).abs() < 0.35 && high > 0.0,
                "after {seconds} s the ring lies {reached} m out, {high} high",
            );
        }
    }
    let (calm, gale) = (after(1.0, CALM)[0].1, after(1.0, GALE)[0].1);
    assert!(gale < calm * 0.2, "a gale leaves {gale} of a calm's {calm}");
    assert_eq!(after(4.5, GALE)[0].1, 0.0, "spent, the water lies still");
}

#[test]
fn flesh_stirs_the_water_only_where_it_moves_through_the_surface() {
    let (stood, stepped) = (Vec2::new(3.0, 3.0), Vec2::new(3.1, 3.0));
    let mut field = RippleField::default();
    field.part(
        wading(&[leg(stood)]),
        wading(&[leg(stood)]),
        FRAME,
        &RIPPLES,
    );
    assert_eq!(crest(&field, stood, Vec2::X).1, 0.0, "a leg at rest");

    field.part(
        wading(&[leg(stood)]),
        wading(&[leg(stepped)]),
        FRAME,
        &RIPPLES,
    );
    assert!(
        field.height_at(stepped + Vec2::X * 0.1) > 0.0,
        "ahead of it"
    );
    assert!(field.height_at(stood - Vec2::X * 0.1) < 0.0, "behind it");
    let churned = field.froth_at(stepped + Vec2::X * 0.1);
    assert!(churned > 0.0, "froth where it passed");
    assert_eq!(field.froth_at(stood + Vec2::Y), 0.0, "none beside its way");
    let mut slow = RippleField::default();
    slow.part(
        wading(&[leg(stood)]),
        wading(&[leg(stepped)]),
        Seconds(1.0),
        &RIPPLES,
    );
    assert!(
        slow.froth_at(stepped + Vec2::X * 0.1) < churned * 0.1,
        "slower flesh churns less"
    );
    pass(&mut field, 1.0, CALM);
    let left = field.froth_at(stepped + Vec2::X * 0.1);
    assert!(left > 0.0 && left < churned * 0.5, "the froth settles");
    let (reached, high) = crest(&field, stepped, Vec2::X);
    assert!(
        reached > 0.3 && reached < 1.35 && high > 0.0,
        "the leg's ripple lies {reached} m out"
    );

    let under = |at: Vec2| TreadingLimb {
        from: Vec3::new(at.x, LEVEL - 0.5, at.y),
        to: Vec3::new(at.x, LEVEL - 1.0, at.y),
        ..leg(at)
    };
    let mut field = RippleField::default();
    field.part(
        wading(&[under(stood)]),
        wading(&[under(stepped)]),
        FRAME,
        &RIPPLES,
    );
    assert_eq!(crest(&field, stood, Vec2::X).1, 0.0, "a leg under water");

    field.part(None, wading(&[leg(stood)]), FRAME, &RIPPLES);
    assert!(field.height_at(stood) > 0.0, "a leg come into the water");
}

#[test]
fn ripples_stay_where_they_are_as_the_field_follows_its_middle() {
    let struck = Vec2::new(2.0, 1.0);
    let mut field = RippleField::default();
    field.follow(Vec2::ZERO);
    field.strike(struck, Meters(0.5), 1.0);
    let held = field.height_at(struck);
    field.follow(Vec2::new(-12.0, 9.0));
    assert_eq!(field.height_at(struck), held);
    field.follow(Vec2::new(500.0, 0.0));
    assert_eq!(field.height_at(struck), 0.0);
}
