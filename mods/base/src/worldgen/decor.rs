use crate::blocks::*;
use weft_sdk::noise::*;
use weft_sdk::*;

use super::consts::*;
use super::statics::*;
use super::util::sphere;

static COVERED: Block = Block::named("core:covered");
pub(super) fn plant_at(seed: u64, surface: BlockPos, biome: u32, ground: u32) -> u32 {
    if surface.y < SEA || FROZEN.on_biome(biome) || !PLANT_SOIL.on_block(ground) {
        return 0;
    }
    let h = hash(seed ^ 0xF10, surface.x as i64, 0, surface.z as i64);
    soil_plant(seed, surface.x, surface.z, biome, h, unit(h))
}

fn soil_plant(seed: u64, x: i32, z: i32, b: u32, h: u64, roll: f64) -> u32 {
    if roll < FLOWER_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        FLOWERS
            .on_biome(b)
            .map_or_else(|| [DANDELION.id(), ROSE.id()][(h >> 32) as usize % 2], |l| l.pick(h >> 32).unwrap_or(0))
    } else if roll < FERN_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        FERN.id()
    } else if roll < CLOVER_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        CLOVER.id()
    } else if roll < BLUEGRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        let wave = fixed::fbm(fixed::seed32(seed, 0x7A11) + 1, x * 16, z * 16, 48 * 16, 3);
        if wave < 0 { BLUEGRASS.id() } else { 0 }
    } else if roll < SHORT_GRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        SHORT_GRASS.id()
    } else if roll < SHORT_DRYGRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        SHORT_DRYGRASS.id()
    } else if roll < DRYGRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        DRYGRASS.id()
    } else if roll < TALL_GRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        TALL_TALL_GRASS.id()
    } else if roll < GRASS_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        TALL_GRASS.id()
    } else if roll < DRY_FERN_COVER.on_biome(b).unwrap_or(0.0) as f64 {
        BUSH_STEM.id()
    } else {
        0
    }
}

/// Plants and Clay after the terrain and trees filled the cube.
pub fn decorate(g: &mut Gen) {
    let place_tall_grass = |g: &mut Gen, x, y, z| {
        let upper = COVERED.with(&[("to", "down")]);
        if y + 1 < 16 && g.get(x, y + 1, z) == 0 && upper != u32::MAX {
            g.set(x, y, z, TALL_TALL_GRASS.id());
            g.set(x, y + 1, z, upper);
        } else {
            g.set(x, y, z, TALL_GRASS.id());
        }
    };
    let Some(&cols) = g.columns() else { return };
    let seed = g.seed();
    let BlockPos { x: ox, y: oy, z: oz } = g.origin();
    let max_h = cols.iter().map(|c| c.height).max().unwrap_or(0);
    let min_h = cols.iter().map(|c| c.height).min().unwrap_or(0);

    // Surface decoration only near the surface.
    if oy > max_h + 16 || oy + 15 < min_h - 1 {
        return;
    }
    // Plant decorations
    for z in 0..16usize {
        for x in 0..16usize {
            let col = &cols[z * 16 + x];
            let y = col.height + 1 - oy;
            if !(0..16).contains(&y) || col.height < SEA {
                continue;
            }
            let y = y as usize;
            if g.get(x, y, z) != 0 {
                continue;
            }
            let (wx, wz) = (ox + x as i32, oz + z as i32);
            let h = hash(seed ^ 0xF10, wx as i64, 0, wz as i64);
            let roll = unit(h);
            // Plants go where they could be planted (soil tags); the biome decides how many.
            let b = col.biome;
            if FROZEN.on_biome(b) {
                continue;
            }
            // What is under the lowest layer takes a look at the terrain: only if anything would grow.
            if y == 0
                && roll >= GRASS_COVER.on_biome(b).unwrap_or(0.05).max(0.08) as f64
                && roll >= TALL_GRASS_COVER.on_biome(b).unwrap_or(0.0).max(0.08) as f64
            {
                continue;
            }
            let below = g.read([wx, oy + y as i32 - 1, wz]);
            let coarse_cover = COARSE_DIRT_COVER.on_biome(b).unwrap_or(0.0) as f64;
            if coarse_cover > 0.0 && y > 0 && GRASS.is(below) {
                let patch = fixed::fbm(fixed::seed32(seed, 0xC0A5E), wx * 16, wz * 16, 20 * 16, 3);
                let value = (patch as f64 / fixed::ONE as f64 + 1.0) * 0.5;
                if value < coarse_cover {
                    g.set(x, y - 1, z, COARSE_DIRT.id());
                }
            }
            if CACTUS_SOIL.on_block(below) {
                let cactus_cover = GLOBULAR_CACTI_COVER.on_biome(b).unwrap_or(0.0) as f64;
                let columnar_cactus_cover = COLUMNAR_CACTI_COVER.on_biome(b).unwrap_or(0.0) as f64;
                if roll < columnar_cactus_cover && y + 3 < 16 {
                    let n = 2 + (h >> 40) % 3;
                    let mut dy = 0;
                    for _ in 0..n as usize {
                        g.set(x, y + dy, z, CACTUS.id());
                        dy += 1;
                    }
                    if dy >= 4 && (h >> 50) % 6 == 1 && y + dy < 16 {
                        g.set(x, y + dy, z, PRICKLY_PEAR.id());
                    }
                    continue;
                } else if roll < cactus_cover {
                    g.set(x, y, z, CACTUS.id());
                    if y + 1 < 16 && (h >> 50) % 4 == 1 {
                        g.set(x, y + 1, z, CACTUS_FLOWER.id());
                    }
                    continue;
                }
            }
            if GENERAL_SOIL.on_block(below) {
                let plant = soil_plant(seed, wx, wz, b, h, roll);
                if TALL_TALL_GRASS.is(plant) {
                    place_tall_grass(g, x, y, z);
                } else if plant != 0 {
                    g.set(x, y, z, plant);
                }
            } else if SUGAR_CANE_SOIL.on_block(below)
                && col.height == SEA
                && roll < 0.08
                && y + 2 < 16
                && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|&(dx, dz)| g.column(wx + dx, wz + dz).height < SEA)
            {
                let n = 1 + (h >> 40) % 3;
                for k in 0..n as usize {
                    g.set(x, y + k, z, SUGAR_CANE.id());
                }
            }
        }
    }
    // Clay patches in shallow water (rivers, lakes, ocean shores), replacing the top blocks in
    // bodies of water
    for z in 0..16usize {
        for x in 0..16usize {
            let col = &cols[z * 16 + x];
            // Make sure we only generate clay patches in shallower waters
            if col.height >= SEA || col.height < SEA - 6 {
                continue;
            }
            let y = col.height - oy;
            if !(0..16).contains(&y) {
                continue;
            }
            let y = y as usize;
            let (wx, wz) = (ox + x as i32, oz + z as i32);
            // Clay patch size based on noise scale
            let clay = fixed::fbm(fixed::seed32(seed, 0xC1A7), wx * 16, wz * 16, 18 * 16, 3);
            // Threshold for clay generation
            if clay > fixed::ONE / 4 {
                let surface = g.get(x, y, z);
                if SAND.is(surface) || GRAVEL.is(surface) {
                    g.set(x, y, z, CLAY.id());
                }
            }
        }
    }
}
pub fn place_boulder(s: &mut Plan, rng: &mut Rng, ox: i32, oy: i32, oz: i32, cells: &mut Vec<[i32; 3]>) {
    for (dx, dy, dz) in sphere(1.5) {
        let pos = [ox + dx, oy + dy, oz + dz];
        let block = if rng.float() < 0.5 { COBBLESTONE.id() } else { MOSSY_COBBLESTONE.id() };
        s.put(pos, block);
        cells.push(pos);
    }
}
pub fn place_bush(s: &mut Plan, leaf: impl Fn(&mut Rng) -> u32, stem: u32) {
    let BlockPos { x, y, z } = s.pos();
    let mut rng = Rng(s.random());
    let bush_size = {
        let mut result = rng.float() + 0.7;
        if result < 1.0 {
            result = 1.0;
        }
        result
    };
    s.put([x, y, z], stem);
    let mut place_ball = |mut rng: &mut Rng, ox, oy, oz, size| {
        for (dx, dy, dz) in sphere(size) {
            s.put([ox + dx, oy + dy, oz + dz], leaf(&mut rng));
        }
    };
    place_ball(&mut rng, x, y, z, bush_size);
    if bush_size < 1.5 {
        return;
    }
    let dx = x + (rng.float() * 2.0) as i32;
    let dz = z + (rng.float() * 2.0) as i32;
    place_ball(&mut rng, dx, y, dz, bush_size - 0.5);
}
