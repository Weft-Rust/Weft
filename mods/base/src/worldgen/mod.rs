//! Base's worldgen as data: biomes, the overworld's terrain, ores, trees (a scatter the engine
//! places in every dimension whose biomes ask for them) and plants (a decoration).
pub mod consts;
mod decor;
pub mod statics;
pub mod tree_algorythms;
pub mod tree_kinds;
mod util;

use consts::*;
use decor::*;
use statics::*;
pub use tree_algorythms::grow_tree;
use tree_algorythms::register as register_trees;
use tree_kinds::*;
use util::gentle;

use weft_sdk::*;

use crate::blocks::*;

use weft_sdk::noise::*;
use weft_sdk::terrain::*;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Snow line in a temperate climate (temp = 0)
const SNOW_LINE: i32 = 190;
/// Air cools with height, so the warmer the climate the higher the snow line for this many
/// blocks for every step of temperature
const LAPSE: f64 = 100.0;
/// The lowest y the snow line can get in the coldest climates
const SNOW_FLOOR: i32 = 165;
/// Band size of snow line
const BAND: f64 = 0.1;
/// Afaik no mountain gets higher than this so a climate whose snow line is above it has none
const SNOW_TOP: i32 = SEA + 190;

fn snow_bands() -> Vec<(f64, f64, i32)> {
    let coldest = f64::from(SNOW_FLOOR - SNOW_LINE) / LAPSE;
    let edge = |i: usize| coldest + i as f64 * BAND;
    let mut bands = vec![(f64::NEG_INFINITY, coldest, SNOW_FLOOR)];
    for i in 0.. {
        let line = SNOW_LINE + (LAPSE * (edge(i) + BAND / 2.0)).round() as i32;
        if line > SNOW_TOP {
            break;
        }
        bands.push((edge(i), edge(i + 1), line));
    }
    bands
}

/// What a biome grows, as data any mod can set on any biome: trees per 5x5 cell (0..1), share of
/// grass on plantable soil, which trees and flowers (block lists the engine checks), and tags `frozen` (nothing grows, set by
/// `.frozen(..)`).

pub fn register(r: &mut Registry) {
    // Tags say what a biome is (spawning, decoration and mods ask these, not names).
    let (grass, foliage, water) = (&tints::GRASS, &tints::FOLIAGE, &tints::WATER);
    register_trees();
    r.biome(&PLAINS)
        .surface("grass", "dirt", 3)
        .tag("animals")
        .tag("spawn_biome")
        .set(&TREES, 0.025)
        .set(&GRASS_COVER, 0.7)
        .set(&TALL_GRASS_COVER, 0.075)
        .set(&BOULDER_COVER, 0.03)
        .set(&BUSH_COVER, 0.03)
        .set(&RASPBERRY_COVER, 0.005)
        .set(&SNOWBERRY_COVER, 0.002)
        .set_text(&FLOWERS, "base:rose base:dandelion base:tulip_orange base:tulip_red base:tulip_yellow base:tulip_white")
        .set(&FLOWER_COVER, 0.02);
    r.biome(&LAVENDER_FIELDS)
        .surface("grass", "dirt", 3)
        .tag("animals")
        .tag("spawn_biome")
        .set(&GRASS_COVER, 0.7)
        .set(&TALL_GRASS_COVER, 0.25)
        .set(&BOULDER_COVER, 0.03)
        .set(&BUSH_COVER, 0.02)
        .set(&RASPBERRY_COVER, 0.005)
        .set(&SNOWBERRY_COVER, 0.002)
        .set_text(&FLOWERS, "base:lavender")
        .set(&FLOWER_COVER, 0.1);
    r.biome(&HILLS)
        .surface("grass", "dirt", 3)
        .tag("animals")
        .tag("spawn_biome")
        .tint(grass, 0x88bb67)
        .tint(foliage, 0x6ba941)
        .set(&TREES, 0.025)
        .set(&TREE_KINDS, TreeKinds::from([("base:spruce", 1)]))
        .set(&GRASS_COVER, 0.5)
        .set(&TALL_GRASS_COVER, 0.05)
        .set(&BOULDER_COVER, 0.06)
        .set(&BLUEGRASS_COVER, 0.2)
        .set_text(&FLOWERS, "base:lavender base:dandelion base:tulip_pink base:tulip_purple base:tulip_white")
        .set(&FLOWER_COVER, 0.02);
    r.biome(&TALLGRASS_MEADOW)
        .surface("grass", "dirt", 3)
        .tag("animals")
        .tag("spawn_biome")
        .set(&TREES, 0.025)
        .set(&GRASS_COVER, 1.0)
        .set(&TALL_GRASS_COVER, 0.7)
        .set(&BOULDER_COVER, 0.03)
        .set(&BUSH_COVER, 0.03)
        .set(&RASPBERRY_COVER, 0.005)
        .set(&SNOWBERRY_COVER, 0.002)
        .set_text(&FLOWERS, "base:dandelion base:inula base:verbascum")
        .set(&FLOWER_COVER, 0.1);
    r.biome(&CLOVER_MEADOW)
        .surface("grass", "dirt", 3)
        .tag("animals")
        .tag("spawn_biome")
        .set(&TREES, 0.025)
        .set(&CLOVER_COVER, 0.3)
        .set(&SHORT_GRASS_COVER, 0.6)
        .set(&GRASS_COVER, 0.7)
        .set(&BOULDER_COVER, 0.03)
        .set(&FLOWER_COVER, 0.0);
    r.biome(&FOREST)
        .humidity(0.125..)
        .surface("grass", "dirt", 3)
        .tint(grass, 0x79c05a)
        .tint(foliage, 0x59ae30)
        .tag("forest")
        .tag("animals")
        .set(&TREES, 0.4)
        .set(&FERN_COVER, 0.2)
        .set(&GRASS_COVER, 0.4)
        .set(&BUSH_COVER, 0.06)
        .set(&RASPBERRY_COVER, 0.002)
        .set(&FUNGI_COVER, 0.001)
        .set(&COARSE_DIRT_COVER, 0.4)
        // .set_text(&FLOWERS, "base:rose base:dandelion base:fern base:fern")
        // .set(&FLOWER_COVER, 0.02)
        .set(&FLOWER_COVER, 0.02)
        .set(&TREE_KINDS, TreeKinds::from([("base:oak", 4), ("base:birch", 1)]));
    r.biome(&BIRCH_FOREST)
        .temperature(..0.1)
        .humidity(0.05..)
        .surface("grass", "dirt", 3)
        .tint(grass, 0x88bb67)
        .tint(foliage, 0x6ba941)
        .tag("forest")
        .tag("animals")
        .set(&TREES, 0.45)
        .set(&FERN_COVER, 0.2)
        .set(&GRASS_COVER, 0.4)
        .set(&BUSH_COVER, 0.06)
        .set(&RASPBERRY_COVER, 0.002)
        .set(&FUNGI_COVER, 0.001)
        // .set_text(&FLOWERS, "base:rose base:dandelion base:fern base:fern")
        // .set(&FLOWER_COVER, 0.02)
        .set(&TREE_KINDS, TreeKinds::from([("base:birch", 1)]));
    r.biome(&TAIGA)
        .temperature(..-0.15)
        .surface("grass", "dirt", 3)
        .tint(grass, 0x86b783)
        .tint(foliage, 0x68a464)
        .tint(water, 0x3d57d6)
        .tag("forest")
        .tag("cold")
        .tag("animals")
        .set(&TREES, 0.45)
        .set(&GRASS_COVER, 0.08)
        .set(&TREE_KINDS, TreeKinds::from([("base:spruce", 1)]));
    r.biome(&SNOWY)
        .snows()
        .temperature(..-0.25)
        .surface("snowy_grass", "dirt", 3)
        .frozen("ice")
        .tint(grass, 0x80b497)
        .tint(foliage, 0x60a17b)
        .tint(water, 0x3938c9)
        .tag("cold")
        .tag("animals")
        .set(&TREES, 0.03)
        .set(&TREE_KINDS, TreeKinds::from([("base:spruce", 1)]));
    r.biome(&DESERT)
        .temperature(0.15..)
        .humidity(..0.1)
        .surface("sand", "sand", 3)
        .under("sandstone", 1)
        .tint(grass, 0xbfb755)
        .tint(foliage, 0xaea42a)
        .set(&GLOBULAR_CACTI_COVER, 0.01)
        .set(&COLUMNAR_CACTI_COVER, 0.005)
        .set(&SHORT_GRASS_COVER, 0.05)
        .set(&SHORT_DRYGRASS_COVER, 0.2)
        .set(&DRY_FERN_COVER, 0.21)
        .tag("hot");
    r.biome(&GRASSY_DESERT)
        .temperature(0.1..)
        .humidity(..0.1)
        .surface("sand", "sand", 3)
        .under("sandstone", 1)
        .tint(grass, 0xbfb755)
        .tint(foliage, 0xaea42a)
        .set(&GLOBULAR_CACTI_COVER, 0.01)
        .set(&COLUMNAR_CACTI_COVER, 0.005)
        .set(&DRYGRASS_COVER, 0.3)
        .set(&TALL_GRASS_COVER, 0.375)
        .set(&DRY_FERN_COVER, 0.21)
        .tag("hot");
    r.biome(&DUNES)
        .temperature(0.33..)
        .humidity(..0.1)
        .surface("sand", "sand", 3)
        .under("sandstone", 1)
        .tint(grass, 0xbfb755)
        .tint(foliage, 0xaea42a)
        .tag("hot");
    r.biome(&SEMIDESERT)
        .temperature(0.1..)
        .humidity(..0.1)
        .surface("coarse_dirt", "coarse_dirt", 4)
        .under("dirt", 1)
        .tint(grass, 0xbfb755)
        .tint(foliage, 0xaea42a)
        .set(&GLOBULAR_CACTI_COVER, 0.01)
        .set(&COLUMNAR_CACTI_COVER, 0.005)
        .set(&TALL_GRASS_COVER, 0.375)
        .set(&TREES, 0.025)
        .set(&TREE_KINDS, TreeKinds::from([("base:oak", 1), ("base:acacia", 3)]))
        .tag("hot");
    r.biome(&MOUNTAINS)
        .surface("grass", "dirt", 3)
        .tint(grass, 0x8ab689)
        .tint(foliage, 0x6da36b)
        .tag("mountain")
        .tag("animals")
        .set(&TREES, 0.06)
        .set(&GRASS_COVER, 0.05)
        .set(&TREE_KINDS, TreeKinds::from([("base:spruce", 1)]));
    r.biome(&SNOWY_PEAKS)
        .snows()
        .surface("snow", "snow", 3)
        .tint(grass, 0x80b497)
        .tint(foliage, 0x60a17b)
        .tint(water, 0x3938c9)
        .tag("mountain")
        .tag("cold");
    r.biome(&OCEAN)
        .surface("sand", "sand", 3)
        .deep_top("gravel", SEA - 8)
        .tag("ocean");
    r.biome(&BEACH)
        .surface("sand", "sand", 3)
        .set(&SHORT_DRYGRASS_COVER, 0.2)
        .set(&TREES, 0.1)
        .set(&TREE_KINDS, TreeKinds::from([("base:palm", 1)]))
        .tag("beach");
    // Underground (3D): whole cubes deep under the surface; spiders like it there.
    r.biome(&DEEP_CAVES).tag("cave").tag("dark");

    // The last: share left out where it would touch a cave, vanilla's.
    for (ore, max_y, veins, size, exposed) in [
        (&COAL_ORE, 128, 8, 12, 0.0),
        (&IRON_ORE, 64, 6, 8, 0.0),
        (&GOLD_ORE, 32, 2, 7, 0.5),
        (&crate::redstone::REDSTONE_ORE, 16, 4, 8, 0.0),
        (&DIAMOND_ORE, 16, 1, 6, 0.5),
    ] {
        r.ore(ore)
            .y(i32::MIN, max_y)
            .per_cube(veins)
            .vein_size(size)
            .replaces("#stone_ore_replaceables")
            .exposed(exposed);
    }

    // The overworld's shape. Other mods add to it by these names (`base:desert`, `base:mountains`).
    let terrain = r
        .terrain(&crate::OVERWORLD)
        .stone("stone")
        .base_height(SEA as f64 - 8.0)
        .sea(SEA, "water")
        .field("temperature", Expr::fbm(0x22, 2200.0, 3) * Expr::value(1.4))
        .field("humidity", Expr::fbm(0x33, 1600.0, 3) * Expr::value(1.4))
        // Broad patches with smaller offshoots. Shifting the surface thresholds
        // with coherent noise mixes materials where two regions are close.
        .field("base:surface_patch", Expr::fbm(0x89, 24.0, 1) * Expr::value(0.76) + Expr::fbm(0x8A, 7.0, 1) * Expr::value(0.24))
        .field("base:biome_edge", Expr::field("base:surface_patch") * Expr::value(0.125))
        .field("base:surface_temperature", Expr::temperature() + Expr::field("base:biome_edge") * Expr::value(0.24))
        .field("base:surface_humidity", Expr::humidity() - Expr::field("base:biome_edge") * Expr::value(0.22))
        .field(
            "base:weirdness",
            (Expr::fbm(0x4A7, 1000.0, 3).at(
                Expr::x() + Expr::fbm(0x111, 600.0, 2) * Expr::blocks(80.0) + Expr::noise(40.0) * Expr::value(0.5),
                Expr::z() + Expr::fbm(0x222, 600.0, 2) * Expr::blocks(80.0) + Expr::noise(40.0) * Expr::value(0.5),
            ) + Expr::field("base:biome_edge") * Expr::value(0.08))
            .ramp(-0.2, 0.2),
        )
        .field("base:randomness", Expr::fbm(0x4A7, 700.0, 3).ramp(-0.2, 0.2))
        .field(
            "base:mountains",
            Expr::fbm(0x44, 700.0, 3)
                .ramp(-0.1, 0.3)
                .and(Expr::continent().ramp(0.0, 0.2)),
        )
        .field(
            "base:plains",
            Expr::fbm(0x53, 670.0, 3)
                .ramp(-0.19, 0.22)
                .and(Expr::continent().ramp(0.04, 0.19))
                .and(Expr::field("base:mountains").ramp(0.16, 0.42).not()),
        )
        .field(
            "base:shore",
            Expr::fbm(0x81, 210.0, 3) * Expr::value(0.78) + Expr::fbm(0x82, 39.0, 2) * Expr::value(0.22)
                - Expr::humidity() * Expr::value(0.18)
                + Expr::field("base:surface_patch") * Expr::value(0.18),
        )
        .field(
            "base:rock_line",
            Expr::fbm(0x85, 175.0, 3) * Expr::value(0.66)
                + Expr::fbm(0x86, 37.0, 2) * Expr::value(0.34)
                + Expr::field("base:surface_patch") * Expr::value(0.66),
        )
        .field("base:surface_mountains", Expr::field("base:mountains") + Expr::field("base:biome_edge") * Expr::value(0.18))
        .field("base:surface_plains", Expr::field("base:plains") + Expr::field("base:biome_edge") * Expr::value(0.18))
        .landform(preset::continents(18.0))
        // Plains have long, shallow rises and small swales. Ease out the
        // shorter hills as the plains take over.
        .landform(
            Landform::new(Expr::fbm(0x54, 310.0, 2) * Expr::blocks(9.0) + Expr::fbm(0x55, 105.0, 2) * Expr::blocks(3.0))
                .mask(Expr::field("base:plains"))
                .named("plains"),
        )
        .landform(
            preset::hills(16.0, 110.0)
                .mask(Expr::temperature().ramp(-0.35, -0.6).not() * (Expr::value(1.0) - Expr::field("base:plains") * Expr::value(0.86))),
        )
        .landform(
            preset::dunes(5.0, 30.0).mask(
                Expr::temperature()
                    .ramp(0.25, 0.55)
                    .and(Expr::humidity().ramp(0.1, -0.05)),
            ),
        )
        .landform(
            preset::mountains(150.0, 260.0)
                .mask(Expr::field("base:mountains"))
                .named("mountains"),
        )
        // Slight jittering so snow doesn't fall on visible contours
        .field(
            "base:alpine",
            Expr::field("base:surface_temperature") + Expr::noise(400.0) * Expr::value(0.2) + Expr::noise(70.0) * Expr::value(0.1)
                - Expr::humidity() * Expr::value(0.1),
        )
        // Biomes bring their climate and surface; rules only add where else they must hold.
        .rule(
            BiomeRule::new("ocean")
                .height(..=SEA - 8)
                .surface("gravel", "gravel", 3),
        )
        .rule(
            BiomeRule::new("snowy_plains")
                .named("snowy_coast")
                .height(SEA - 1..=SEA + 1)
                .temperature(..)
                .field("base:surface_temperature", ..-0.25)
                .continent(..0.1),
        )
        .rule(
            BiomeRule::new("beach")
                .named("rocky_shore")
                .surface("stone", "stone", 0)
                .height(SEA - 4..=SEA + 3)
                .continent(..0.28)
                .field("base:mountains", 0.24..)
                .field("base:shore", ..-0.08),
        )
        .rule(
            BiomeRule::new("beach")
                .named("gravel_shore")
                .surface("gravel", "gravel", 2)
                .height(SEA - 4..=SEA + 1)
                .continent(..0.28)
                .field("base:shore", ..-0.23),
        )
        .rule(
            BiomeRule::new("beach")
                .height(SEA - 4..=SEA + 3)
                .continent(..0.28)
                .field("base:shore", 0.23..),
        )
        .rule(
            BiomeRule::new("beach")
                .named("narrow_beach")
                .height(SEA - 3..=SEA)
                .continent(..0.28)
                .field("base:shore", 0.10..),
        )
        .rule(
            BiomeRule::new("ocean")
                .named("shallow_water")
                .surface("dirt", "dirt", 2)
                .height(..SEA),
        );
    let mut terrain = terrain;
    for (i, (cold, warm, line)) in snow_bands().into_iter().enumerate() {
        let alpine = BiomeRule::new("snowy_peaks")
            .field("base:surface_mountains", 0.45..)
            .field("base:alpine", cold..warm);
        terrain = terrain
            .rule(
                alpine
                    .clone()
                    .named(&format!("snow_line_{i}"))
                    .height(line..),
            )
            .rule(
                alpine
                    .height(line - 1..line)
                    .field("base:surface_patch", ..-0.08)
                    .surface("snowy_grass", "dirt", 2)
                    .named(&format!("snow_patches_{i}")),
            );
    }
    for i in 0..9 {
        let lo = if i == 0 { f64::NEG_INFINITY } else { (i - 5) as f64 * 0.13 };
        let hi = if i == 8 { f64::INFINITY } else { (i - 4) as f64 * 0.13 };
        let line = 106 + i * 6;
        let mountains = BiomeRule::new("mountains")
            .field("base:surface_mountains", 0.45..)
            .field("base:rock_line", lo..hi);
        terrain = terrain
            .rule(
                mountains
                    .clone()
                    .named(&format!("exposed_rock_{i}"))
                    .height(line..)
                    .surface("stone", "stone", 0),
            )
            .rule(
                mountains
                    .named(&format!("thin_soil_{i}"))
                    .height(line - 7..line)
                    .field("base:surface_patch", 0.02..)
                    .surface("coarse_dirt", "dirt", 1),
            );
    }
    terrain
        .rule(BiomeRule::new("mountains").field("base:surface_mountains", 0.45..))
        .rule(
            BiomeRule::new("snowy_plains")
                .temperature(..)
                .field("base:surface_temperature", ..-0.25),
        )
        .rule(
            BiomeRule::new("taiga")
                .temperature(..)
                .field("base:surface_temperature", ..-0.15),
        )
        .rule(
            BiomeRule::new("dunes")
                .temperature(..)
                .humidity(..)
                .field("base:surface_temperature", 0.28..)
                .field("base:surface_humidity", ..0.1),
        )
        .rule(
            BiomeRule::new("desert")
                .temperature(..)
                .humidity(..)
                .field("base:surface_temperature", 0.15..)
                .field("base:surface_humidity", ..0.1),
        )
        .rule(
            BiomeRule::new("grassy_desert")
                .temperature(..)
                .humidity(..)
                .field("base:surface_temperature", 0.15..)
                .field("base:surface_humidity", ..0.2)
                .field("base:weirdness", ..0.6),
        )
        .rule(
            BiomeRule::new("semidesert")
                .field("base:surface_temperature", 0.0..)
                .field("base:surface_humidity", ..0.3)
        )
        .rule(
            BiomeRule::new("plains")
                .named("open_plains")
                .field("base:surface_plains", 0.57..)
                .field("base:surface_humidity", ..0.22),
        )
        .rule(
            BiomeRule::new("forest")
                .humidity(..)
                .field("base:surface_humidity", 0.125..),
        )
        .rule(
            BiomeRule::new("birch_forest")
                .temperature(..)
                .humidity(..)
                .field("base:surface_temperature", ..0.1)
                .field("base:surface_humidity", 0.05..),
        )
        .rule(
            BiomeRule::new("hills")
                .field("base:randomness", 0.5..)
                .field("base:surface_mountains", 0.3..),
        )
        .rule(BiomeRule::new("lavender_fields").field("base:weirdness", 1.0..))
        .rule(BiomeRule::new("tallgrass_meadow").field("base:weirdness", 0.1..0.3))
        .rule(BiomeRule::new("clover_meadow").field("base:weirdness", 0.6..0.9))
        .rule(BiomeRule::new("plains"))
        .cave_biome("deep_caves", 24, 96.0, 0.25)
        .cave_biome("deep_caves", 48, 96.0, -1.0)
        .caves(Caves::new("lava", i32::MIN, 11));

    // Trees wherever a biome asks for them, in any dimension.
    r.scatter(&TREE_SITES)
        .impostor()
        .spacing(5)
        .reach(20)
        .density(&TREES)
        .ground("#tree_soil")
        .plan(|s| {
            // if COARSE_DIRT.is(s.read(s.pos().below())) {
            //     return;
            // }
            let kinds = TREE_KINDS.on_biome(s.biome());
            let mut rng = Rng(s.random());
            // Biomes that ask for trees without naming kinds grow oaks, as they always did.
            let name = match &kinds {
                Some(kinds) => kinds.pick(rng.next()),
                None => Some("base:oak"),
            };
            let Some(name) = name else { return };
            let Some(place_tree) = tree_place(name) else { return };
            place_tree(&mut TreePlaceData::new(s));
        });
    r.scatter(&BOULDER_SITES)
        .impostor()
        .spacing(5)
        .reach(3)
        .density(&BOULDER_COVER)
        .plan(|s| {
            let BlockPos { x, y, z } = s.pos();
            if y < 70 || !gentle(s, 2, 2) {
                return;
            }
            let mut rng = Rng(s.random());
            let mut cells = Vec::new();
            place_boulder(s, &mut rng, x, y, z, &mut cells);
            if rng.float() < 0.5 {
                let dx = x + (rng.float() * 2.0) as i32;
                let dy = y + (rng.float() * 2.0 - 0.5) as i32;
                let dz = z + (rng.float() * 2.0) as i32;
                place_boulder(s, &mut rng, dx, dy, dz, &mut cells);
            }
            let blocks: BTreeSet<[i32; 3]> = cells.into_iter().collect();
            let mut carpets: BTreeMap<[i32; 3], u8> = BTreeMap::new();
            for b in &blocks {
                for &d in &Dir::ALL {
                    if d == Dir::Down {
                        continue;
                    }
                    let o = d.offset();
                    let n = [b[0] + o.x, b[1] + o.y, b[2] + o.z];
                    if blocks.contains(&n) {
                        continue;
                    }
                    let face = d.opposite().face() as u8;
                    *carpets.entry(n).or_insert(0) |= 1u8 << face;
                }
            }
            for (p, mask) in carpets {
                if rng.float() > 0.2 {
                    continue;
                }
                let props: Vec<(&str, &str)> = Dir::ALL
                    .iter()
                    .map(|&d| {
                        let on = mask & (1u8 << d.face() as u8) != 0;
                        (d.name(), if on { "true" } else { "false" })
                    })
                    .collect();
                let state = MOSS.with(&props);
                if state != u32::MAX {
                    s.put(p, state);
                }
            }
        });
    r.scatter(&BUSH_SITES)
        .impostor()
        .spacing(5)
        .reach(3)
        .density(&BUSH_COVER)
        .plan(|s| place_bush(s, |_r| BUSH_LEAVES.id(), BUSH_STEM.id()));
    r.scatter(&RASPBERRY_SITES)
        .impostor()
        .spacing(5)
        .reach(3)
        .density(&RASPBERRY_COVER)
        .plan(|s| {
            place_bush(
                s,
                |r| if r.float() < 0.5 { RASPBERRY_LEAVES.id() } else { RASPBERRY_LEAVES.with(&[("berried", "true")]) },
                BUSH_STEM.id(),
            )
        });
    r.scatter(&SNOWBERRY_SITES)
        .impostor()
        .spacing(5)
        .reach(3)
        .density(&SNOWBERRY_COVER)
        .plan(|s| {
            place_bush(
                s,
                |r| if r.float() < 0.5 { SNOWBERRY_LEAVES.id() } else { SNOWBERRY_LEAVES.with(&[("berried", "true")]) },
                BUSH_STEM.id(),
            )
        });
    r.scatter(&FUNGI_SITES)
        .spacing(3)
        .reach(1)
        .density(&FUNGI_COVER)
        .plan(|s| {
            let fungi = [&PORCINI_MUSHROOM, &CHANTERELLE_MUSHROOM, &TOADSTOOL_MUSHROOM];
            let mut rng = Rng(s.random());
            s.put(s.pos(), fungi[rng.next() as usize % fungi.len()].id());
        });
    r.decoration(decorate).surface_cover(plant_at);
}

static FUNGI_SITES: Feature = Feature::named("fungis");
static BUSH_SITES: Feature = Feature::named("bushes");
static SNOWBERRY_SITES: Feature = Feature::named("snowberries");
static RASPBERRY_SITES: Feature = Feature::named("raspberries");
static BOULDER_SITES: Feature = Feature::named("boulders");
static TREE_SITES: Feature = Feature::named("trees");

/// The test dimension: flat, bounded in height, nothing else.
pub fn flat(g: &mut Gen) {
    let BlockPos { y: oy, .. } = g.origin();
    for y in 0..16usize {
        let wy = oy + y as i32;
        let s = match wy {
            ..=-1 => STONE.id(),
            0..=2 => DIRT.id(),
            3 => GRASS.id(),
            _ => 0,
        };
        if s != 0 {
            for z in 0..16 {
                for x in 0..16 {
                    g.set(x, y, z, s);
                }
            }
        }
    }
}
