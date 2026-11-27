//! Sound events: block groups (`block.<group>.break|place|step|hit`) blocks pick with
//! `.sound_group(group)`, and the background music; mobs' voices come with them (`mobs.rs`). Files are in `assets/sounds/`.

use crate::worldgen::statics::*;
use weft_sdk::*;

static SHEEP: Music = Music::named("sheep");

static CRICKETS: Music = Music::named("crickets");
static DAY_FOREST: Music = Music::named("day_forest");
static LIGHT_RAIN: Music = Music::named("light_rain");
static FOREST_RAIN: Music = Music::named("forest_rain");
static BEACH_WAVES: Music = Music::named("beach_waves");
static CAVE_DRONE: Sound = Sound::named("cave_drone");
static CAVE_AMBIENCE: Music = Music::named("cave_ambience");
static MOUNTAIN_WIND: Music = Music::named("mountain_wind");

pub fn register(r: &mut Registry) {
    for g in ["stone", "wood", "gravel", "grass", "sand", "glass", "cloth"] {
        let files = |kind: &str| {
            (1..=3)
                .filter_map(|i| {
                    let name = format!("{kind}/{g}{i}");
                    (asset_exists(&name, "sounds", "ogg") || asset_exists(&name, "sounds", "wav")).then_some(name)
                })
                .collect::<Vec<_>>()
        };

        let (dig, step, land) = (files("dig"), files("step"), files("land"));

        let dig: Vec<&str> = dig.iter().map(String::as_str).collect();
        let step: Vec<&str> = step.iter().map(String::as_str).collect();
        let land: Vec<&str> = land.iter().map(String::as_str).collect();

        let glass_breaks = files("random");
        let glass_breaks: Vec<&str> = glass_breaks.iter().map(String::as_str).collect();

        let breaks = if g == "glass" && !glass_breaks.is_empty() { &glass_breaks } else { &dig };

        let land_files = if land.is_empty() { &step } else { &land };

        r.sound(Sound::generated(format!("block.{g}.break")))
            .files(breaks)
            .pitch(0.75, 0.85);

        r.sound(Sound::generated(format!("block.{g}.place")))
            .files(&dig)
            .pitch(0.75, 0.85);

        r.sound(Sound::generated(format!("block.{g}.step")))
            .files(&step)
            .pitch(0.9, 1.1);

        r.sound(Sound::generated(format!("block.{g}.land")))
            .files(land_files)
            .volume(0.6)
            .pitch(0.9, 1.1);

        r.sound(Sound::generated(format!("block.{g}.hit")))
            .files(&step)
            .volume(0.6)
            .pitch(0.5, 0.55);
    }
    r.music(&SHEEP)
        .tracks(&["music/418DreamingSheep"])
        .when(&conditions::SURFACE);

    r.music(&CRICKETS)
        .tracks(&["ambient/crickets"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .when(&conditions::NIGHT)
        .biome(&PLAINS)
        .biome(&LAVENDER_FIELDS)
        .biome(&TALLGRASS_MEADOW)
        .biome(&CLOVER_MEADOW)
        .biome(&FOREST)
        .biome(&BIRCH_FOREST)
        .biome(&TAIGA)
        .biome(&HILLS)
        .rain(0.0, 0.05);

    r.music(&LIGHT_RAIN)
        .tracks(&["ambient/light_rain"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .biome(&PLAINS)
        .biome(&LAVENDER_FIELDS)
        .biome(&TALLGRASS_MEADOW)
        .biome(&CLOVER_MEADOW)
        .rain(0.05, 1.0);

    r.music(&MOUNTAIN_WIND)
        .tracks(&["ambient/mountain_wind"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .biome(&MOUNTAINS)
        .biome(&SNOWY_PEAKS)
        .rain(0.0, 0.05);

    r.music(&DAY_FOREST)
        .tracks(&["ambient/day_forest"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .when(&conditions::DAY)
        .biome(&FOREST)
        .biome(&BIRCH_FOREST)
        .biome(&TAIGA)
        .biome(&HILLS)
        .rain(0.0, 0.05);

    r.music(&FOREST_RAIN)
        .tracks(&["ambient/forest_rain"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .biome(&FOREST)
        .biome(&BIRCH_FOREST)
        .biome(&TAIGA)
        .biome(&HILLS)
        .rain(0.05, 1.0);

    r.music(&BEACH_WAVES)
        .tracks(&["ambient/beach_waves"])
        .ambient(0.1)
        .category(&sound_categories::AMBIENT)
        .when(&conditions::SURFACE)
        .biome(&BEACH);

    r.sound(&CAVE_DRONE)
        .files(&["ambient/cave_drone"])
        .category(&sound_categories::AMBIENT)
        .range(40.0);

    r.music(&CAVE_AMBIENCE)
        .ambient(1.0)
        .when(&conditions::UNDERGROUND)
        .biome(&DEEP_CAVES)
        .spot(&CAVE_DRONE)
        .spot_every(60.0, 120.0)
        .spot_distance(0.0, 0.0);
}
