use bevy::prelude::*;
use engine::core::ground::ShareMap;
use engine::core::math::Pos;
use engine::core::render::world::clouds::{CLOUD_TILE, CloudSheet};
use engine::core::time::{TimeCourse, UnixMillis};
use engine::core::units::Seconds;
use engine::systems::area::rain::rain_at;
use engine::systems::area::{self, AreaDef, Climate, RAIN, Weather};
use serde_json::json;
use testkit::{Game, Map, path};

fn climate() -> Climate {
    serde_json::from_value(json!({
        "seed": 7,
        "turns": 30.0,
        "spells": [
            {
                "name": "fair",
                "weather": { "clouds": 0.2 },
                "lasts": { "shortest": 300.0, "longest": 900.0 },
                "next": [{ "spell": "showers", "weight": 2.0 }, { "spell": "storm", "weight": 1.0 }],
            },
            {
                "name": "showers",
                "weather": { "clouds": 0.7, "water": 0.6, "rain": 0.4 },
                "lasts": { "shortest": 120.0, "longest": 600.0 },
                "next": [{ "spell": "fair", "weight": 1.0 }],
            },
            {
                "name": "storm",
                "weather": { "clouds": 1.0, "water": 1.0, "rain": 1.0, "lightning": 6.0 },
                "lasts": { "shortest": 120.0, "longest": 300.0 },
                "next": [{ "spell": "showers", "weight": 1.0 }],
            },
        ],
    }))
    .expect("a climate")
}

#[test]
fn a_climate_turns_through_its_spells_alike_for_everyone_and_only_as_each_may_follow_the_last() {
    let climate = climate();
    let origin = UnixMillis(1_800_000_000_000);
    let course = TimeCourse::Since(origin);
    let mut walked = None;
    let mut spells: Vec<usize> = Vec::new();
    let mut last_clouds = None;
    for step in 0..10_800 {
        let now = origin.after(Seconds(step as f32 * 2.0));
        walked = climate.course_at(course, now, walked);
        let joined = climate.course_at(course, now, None);
        assert_eq!(
            walked, joined,
            "a client that just joined stands where one that watched all along does"
        );
        let spell = walked.expect("a turning course stands somewhere").spell();
        if spells.last() != Some(&spell) {
            spells.push(spell);
        }
        let clouds = climate.weather_at(walked, now).clouds;
        if let Some(last) = last_clouds {
            let turned: f32 = clouds - last;
            assert!(
                turned.abs() < 0.1,
                "the weather turns from spell to spell over a while, not at once: {last} to {clouds} in 2 s"
            );
        }
        last_clouds = Some(clouds);
    }
    let names: Vec<&str> = spells
        .iter()
        .map(|&spell| climate.spells[spell].name.as_str())
        .collect();
    assert_eq!(
        names.first(),
        Some(&"fair"),
        "the course starts at the first spell"
    );
    for pair in spells.windows(2) {
        let (from, to) = (&climate.spells[pair[0]], &climate.spells[pair[1]]);
        assert!(
            from.next.iter().any(|next| next.spell == to.name),
            "{} follows {}, which names only {:?}",
            to.name,
            from.name,
            from.next.iter().map(|next| &next.spell).collect::<Vec<_>>()
        );
    }
    for name in ["fair", "showers", "storm"] {
        assert!(
            names.contains(&name),
            "in six hours {name} comes: {names:?}"
        );
    }

    let later = origin.after(Seconds(5_000.0));
    let held = climate.course_at(TimeCourse::Held(0), later, None);
    assert_eq!(
        climate.weather_at(held, later),
        climate.spells[0].weather,
        "a server holding the weather still keeps the first spell"
    );
    assert_eq!(
        climate.held(climate.spell_named("storm").expect("the storm")),
        climate.spells[2].weather,
        "a server holding the weather at a named spell keeps that spell"
    );
}

#[test]
fn rain_falls_only_from_cloud_that_holds_water_carried_downwind_as_hard_as_the_spell_pours_where_the_rain_map_lets_it()
 {
    let content = Game::new()
        .area("vale", &Map::open(256, 256), json!([]))
        .hero("vale", 8.0, 8.0)
        .part(
            "area",
            "vale",
            &RAIN.named("", "png"),
            ShareMap::from_fn(2, 1, |across, _| if across < 0.5 { 1.0 } else { 0.0 }).encode(),
        )
        .load();
    let vale = area::load(
        &content,
        content
            .by_path::<AreaDef>(&path("area", "vale"))
            .expect("the vale"),
    );
    let mut world = World::new();
    world.init_resource::<Assets<Image>>();
    let mut sheet = CloudSheet::new(&mut world, testkit::look().clouds);
    let (wet, dry) = (Pos::new(64.0, 128.0), Pos::new(192.0, 128.0));
    let still = Vec2::ZERO;
    let windy = Vec2::new(0.5, -0.25);
    for (pour, lean) in [(1.0, still), (0.4, still), (0.0, still), (1.0, windy)] {
        let weather = Weather {
            clouds: 0.8,
            water: 0.7,
            rain: pour,
            ..Weather::default()
        };
        sheet.cover(weather.clouds, weather.water);
        let (mut rained, mut cloud_without_rain) = (false, false);
        for x in 0..16 {
            for y in 0..16 {
                let drift = Vec2::new(x as f32, y as f32) * CLOUD_TILE.0 / 16.0;
                let fallen = sheet.height().0 - vale.heights.height_at(wet).0;
                let upwind = Vec2::new(wet.x, wet.y) - lean * fallen;
                let under = (upwind - drift) / CLOUD_TILE.0;
                let rain = rain_at(vale, &weather, &sheet, drift, lean, wet);
                let water = sheet.water(under);
                assert!(
                    (rain - pour * water).abs() < 1e-4,
                    "where the rain map lets all of it fall, it rains as hard as the spell pours under the water of the cloud it falls from, upwind by as far as the wind carries it down from the clouds: {rain} under {water} of a {pour} downpour leaning {lean}"
                );
                assert_eq!(
                    rain_at(vale, &weather, &sheet, drift, lean, dry),
                    0.0,
                    "where the rain map lets none fall, it never rains"
                );
                rained |= rain > 0.2 * pour && pour > 0.0;
                cloud_without_rain |= sheet.cloudiness(under) > 0.5 && rain == 0.0;
            }
        }
        assert_eq!(
            rained,
            pour > 0.0,
            "a {pour} downpour rains where cloud holds water"
        );
        assert!(
            cloud_without_rain,
            "under a {pour} downpour some cloud holds no water and drops none"
        );
    }
}
