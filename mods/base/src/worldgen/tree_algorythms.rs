// use crate::worldgen::tree_kinds;

use crate::blocks::{DRYADS_SADDLE_MUSHROOM, MUSHROOMS};
use crate::worldgen::util::sphere;
use crate::worldgen::{TreePlaceData, register_tree};

use weft_sdk::*;

use crate::blocks::*;

use weft_sdk::noise::*;

struct TreeSkeleton {
    height: i32,
    top: Option<(i32, i32, i32)>,
    bottom: (i32, i32, i32),
    branch_tips: Vec<(i32, i32, i32)>,
}
// TODO
// struct TreeAge {
//     trunk_height: &mut dyn FnMut(&mut Rng) -> u16,
//     allow_branching: bool,
//     thin_trunk: bool,
// }
// impl TreeAge {
//     const YOUNG = Config {
//         trunk_height: |rng| 4u16 + (rng.next() as u16) % 2u16,
//         allow_branching: false,
//         thin_trunk: true,
//     };
//     const MATURE = Config {
//         trunk_height: |rng| 6u16 + (rng.next() as u16) % 3u16,
//         allow_branching: false,
//         thin_trunk: false,
//     };
//     const OLD = Config {
//         trunk_height: |rng| 6u16 + (rng.next() as u16) % 3u16,
//         allow_branching: true,
//         thin_trunk: false,
//     };
//     const ALL = [&YOUNG, &MATURE, &OLD];
// }

const DIRECTIONS: [(i32, i32, &str); 4] = [(1, 0, "east"), (-1, 0, "west"), (0, 1, "south"), (0, -1, "north")];
fn random_direction(rng: &mut Rng) -> (i32, i32, &str) {
    DIRECTIONS[rng.next() as usize % 4usize]
}
fn place_roots(d: &mut TreePlaceData, log_block: &Block, wood_block: &Block) {
    let (x, y, z) = d.pos;
    let mut grow_root = |dx: i32, dz: i32, tall: bool| {
        let wood = wood_block.with(&[(
            "facing",
            match (dx, dz) {
                (1, 0) => "east",
                (-1, 0) => "west",
                (0, 1) => "south",
                (0, -1) => "north",
                _ => return,
            },
        )]);
        let x = x + dx;
        let z = z + dz;
        if tall {
            (d.place)(x, y + 1, z, wood, true);
        }
        (d.place)(x, y, z, wood, true);
        (d.place)(x, y - 1, z, log_block.id(), true);
    };
    let root_positions = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut root_count = 0;
    for dpos in root_positions {
        if d.rng.float() < 0.666 {
            grow_root(dpos.0, dpos.1, d.rng.float() < 0.5);
            root_count += 1;
        }
    }
    if root_count == 0 {
        let root_pos = root_positions[d.rng.next() as usize % root_positions.len()];
        grow_root(root_pos.0, root_pos.1, d.rng.float() < 0.5);
    }
    (d.place)(x, y - 1, z, log_block.id(), true);
    (d.place)(x, y - 2, z, DIRT.id(), true);
}
/// ts stops working when angle > 45
struct TrunkLean {
    /// DEGREES
    angle: f32,
    direction: (i32, i32),
}
impl TrunkLean {
    fn no_lurch() -> Self {
        Self { angle: 0.0, direction: (0, 0) }
    }
    fn random(rng: &mut Rng, max_angle: f32) -> Self {
        let angle = rng.float() * max_angle;
        let dir = random_direction(rng);
        let direction = (dir.0, dir.1);
        Self { angle, direction }
    }
    fn offset_at(&self, dy: i32) -> (f32, f32) {
        let (dx, dz) = self.direction;
        let len = ((dx * dx + dz * dz) as f32).sqrt();
        if len == 0.0 || self.angle == 0.0 {
            return (0.0, 0.0);
        }
        let horiz = dy as f32 * self.angle.to_radians().tan();
        (horiz * dx as f32 / len, horiz * dz as f32 / len)
    }
}
fn place_trunk(
    d: &mut TreePlaceData,
    height: u16,
    log_block: &Block,
    wood_block: &Block,
    allow_branches: bool,
    allow_roots: bool,
    lean: TrunkLean,
) -> TreeSkeleton {
    let (x, y, z) = d.pos;
    let top = y + height as i32;
    let log = log_block.id();
    let mut branch_tips: Vec<(i32, i32, i32)> = Vec::new();
    for dy in 0..(height as i32) {
        let y = y + dy;
        let offset = lean.offset_at(dy);
        let x = x + offset.0 as i32;
        let z = z + offset.1 as i32;
        (d.place)(x, y, z, log, true);
        if !allow_branches {
            continue;
        }
        let left = top - y;
        if left < 5 && left > 1 && (d.rng.next() % (left as u64 + 8u64)) <= 2 {
            let roll = d.rng.float();
            if roll < 0.25 {
                (d.place)(x + 1, y, z, log_block.with(&[("axis", "x")]), true);
                (d.place)(x + 2, y, z, log_block.with(&[("axis", "x")]), true);
                branch_tips.push((x + 2, y, z));
            } else if roll < 0.5 {
                (d.place)(x - 1, y, z, log_block.with(&[("axis", "x")]), true);
                (d.place)(x - 2, y, z, log_block.with(&[("axis", "x")]), true);
                branch_tips.push((x - 2, y, z));
            } else if roll < 0.75 {
                (d.place)(x, y, z + 1, log_block.with(&[("axis", "z")]), true);
                (d.place)(x, y, z + 2, log_block.with(&[("axis", "z")]), true);
                branch_tips.push((x, y, z + 2));
            } else {
                (d.place)(x, y, z - 1, log_block.with(&[("axis", "z")]), true);
                (d.place)(x, y, z - 2, log_block.with(&[("axis", "z")]), true);
                branch_tips.push((x, y, z - 2));
            }
        }
    }
    if allow_roots {
        place_roots(d, log_block, wood_block);
    }

    let offset = lean.offset_at(height as i32 - 1);
    TreeSkeleton { height: top - y, top: Some((x + offset.0 as i32, top, z + offset.1 as i32)), bottom: (x, y, z), branch_tips }
}
fn place_cone_foliage(d: &mut TreePlaceData, height: u16, leaves: u32) {
    let (x, y, z) = d.pos;
    for dy in 2..=(height as i32) + 1 {
        let y = y + dy;
        let r = if dy == (height as i32) + 1 { 0 } else { 1 + (((height as i32) + 1 - dy) % 3).min(2) };
        for dz in -r..=r {
            for dx in -r..=r {
                if dx * dx + dz * dz <= r * r + 1 {
                    (d.place)(x + dx, y, z + dz, leaves, false);
                }
            }
        }
    }
}
fn place_foliage(skeleton: &TreeSkeleton, d: &mut TreePlaceData, _height: u16, leaves: u32, branch_leaves_base: (f32, f32)) {
    fn leaf_kept(rx: f32, ry: f32, rz: f32, dx: i32, dy: i32, dz: i32, wx: i32, wy: i32, wz: i32, salt: u64) -> bool {
        let q = (dx as f32 / rx).powi(2) + (dy as f32 / ry).powi(2) + (dz as f32 / rz).powi(2);
        if q > 1.0 {
            return false;
        }
        let d = q.sqrt();
        let shell = 1.0 / rx.min(ry).min(rz).max(1.0);
        let core = (1.0 - shell).max(0.0);
        if d <= core {
            return true;
        }
        let t = ((d - core) / (1.0 - core).max(0.001)).clamp(0.0, 1.0);
        let keep = 0.95 - 0.55 * t;
        let fine = unit(hash(salt, wx as i64, wy as i64, wz as i64)) as f32;
        let coarse = unit(hash(salt ^ 0xBEEF, (wx / 2) as i64, (wy / 2) as i64, (wz / 2) as i64)) as f32;
        fine < keep || coarse < 0.20
    }
    for (x, y, z) in &skeleton.branch_tips {
        let salt = hash(0x1EAF, *x as i64, *y as i64, *z as i64);
        let rx = branch_leaves_base.0 + d.rng.float();
        let ry = rx * branch_leaves_base.1;
        let rz = rx;
        for (dx, dy, dz) in sphere(rx) {
            let bx = x + dx;
            let by = y + dy;
            let bz = z + dz;
            if leaf_kept(rx, ry, rz, dx, dy, dz, bx, by, bz, salt) {
                (d.place)(bx, by, bz, leaves, false);
            }
        }
    }
    let Some((x, y, z)) = skeleton.top else { return };
    let salt = hash(0x1EAF, x as i64, y as i64, z as i64);
    let rx = skeleton.height as f32 * 0.66 + d.rng.float() - 0.5;
    let ry = rx * 0.75;
    let rz = rx;
    for (dx, dy, dz) in sphere(rx) {
        let bx = x + dx;
        let by = y + dy;
        let bz = z + dz;
        if leaf_kept(rx, ry, rz, dx, dy, dz, bx, by, bz, salt) {
            (d.place)(bx, by, bz, leaves, false);
        }
    }
}
fn moss_on_side(side: Dir) -> u32 {
    let props: Vec<(&str, &str)> = Dir::ALL
        .iter()
        .map(|&d| (d.name(), if d == side { "true" } else { "false" }))
        .collect();
    MOSS.with(&props)
}
/// Doesnt work with leaned trunks
fn decorate_trunk(skeleton: &TreeSkeleton, d: &mut TreePlaceData) {
    let mossy = d.rng.float() < 0.25;
    let Some((_, y, _)) = skeleton.top else { return };
    for y in skeleton.bottom.1..y {
        if d.rng.float() >= 0.5 {
            continue;
        }
        if mossy {
            let bz = skeleton.bottom.2 - 1;
            let state = moss_on_side(Dir::South);
            if state != u32::MAX {
                (d.place)(skeleton.bottom.0, y, bz, state, false);
            }
            continue;
        }
        if d.rng.float() >= 0.1 {
            continue;
        }
        let (dx, dz, facing) = random_direction(&mut d.rng);
        let bx = skeleton.bottom.0 + dx;
        let bz = skeleton.bottom.2 + dz;
        let state = DRYADS_SADDLE_MUSHROOM.with(&[("facing", facing)]);
        (d.place)(bx, y, bz, state, false);
    }
}
fn gentle(d: &mut TreePlaceData, r: i32, max_range: i32) -> bool {
    let (x, _, z) = d.pos;
    let mut lo = i32::MAX;
    let mut hi = i32::MIN;
    for dz in -r..=r {
        for dx in -r..=r {
            let h = (d.ground)(x + dx, z + dz);
            if h == i32::MIN {
                return false;
            }
            lo = lo.min(h);
            hi = hi.max(h);
        }
    }
    hi - lo <= max_range
}
fn check_stump(data: &mut TreePlaceData, log_length: u16) -> bool {
    data.rng.float() < 0.1 && gentle(data, log_length as i32 + 3, 3)
}
fn calculate_mean_height_diff(d: &mut TreePlaceData, direction: (i32, i32), log_length: u16) -> (i32, i32) {
    let mut heights = Vec::new();
    for i in 3..(log_length + 3) {
        let log_x = d.pos.0 + direction.0 * (i as i32);
        let log_z = d.pos.2 + direction.1 * (i as i32);
        heights.push((d.ground)(log_x, log_z));
    }
    let mut diffs = Vec::new();
    let mut last: Option<i32> = None;
    for &height in &heights {
        if let Some(previous) = last {
            diffs.push((height - previous).abs());
        }
        last = Some(height);
    }
    (diffs.iter().sum::<i32>() / diffs.len() as i32, *heights.iter().max().unwrap())
}
fn place_log(d: &mut TreePlaceData, log_block: &Block, length: u16) {
    let (dir_index, y) = {
        let mut result = [(0, 0, 0); DIRECTIONS.len()];
        for (i, dir) in DIRECTIONS.iter().enumerate() {
            let (mean, max_y) = calculate_mean_height_diff(d, (dir.0, dir.1), length);
            result[i] = (i, mean, max_y)
        }
        result.sort_by(|a, b| a.1.cmp(&b.1));
        (result[0].0, result[0].2 + 1)
    };
    let direction = DIRECTIONS[dir_index];
    let block = log_block.with(&[(
        "axis",
        match direction.2 {
            "east" | "west" => "x",
            _ => "z",
        },
    )]);
    for i in 3..(length + 3) {
        let dx = direction.0 * i as i32;
        let dz = direction.1 * i as i32;
        let x = d.pos.0 + dx;
        let z = d.pos.2 + dz;
        (d.place)(x, y, z, block, false);
        let roll = d.rng.float();
        let deco = if roll < 0.15 {
            MUSHROOMS[d.rng.next() as usize % MUSHROOMS.len()].id()
        } else if roll < 0.25 {
            moss_on_side(Dir::Down)
        } else {
            0
        };
        if deco != 0 {
            (d.place)(x, y + 1, z, deco, false);
        } else if roll < 0.35 {
            let mut facing = "east";
            let x = x + if dx == 0 {
                let mut d = d.rng.next() as i32 % 2;
                if d == 0 {
                    d = -1
                }
                facing = if d < 0 { "east" } else { "west" };
                d
            } else {
                0
            };
            let z = z + if dz == 0 {
                let mut d = d.rng.next() as i32 % 2;
                if d == 0 {
                    d = -1
                }
                facing = if d < 0 { "north" } else { "south" };
                d
            } else {
                0
            };
            let block = DRYADS_SADDLE_MUSHROOM.with(&[("facing", facing)]);
            (d.place)(x, y, z, block, false);
        }
    }
}
fn place_stump(d: &mut TreePlaceData, log_block: &Block, hollow_log_block: &Block, wood_block: &Block, log_length: u16, allow_roots: bool) {
    if allow_roots {
        place_roots(d, log_block, wood_block);
    }
    let (x, y, z) = d.pos;
    (d.place)(x, y, z, log_block.id(), true);
    (d.place)(x, y + 1, z, log_block.id(), true);
    if d.rng.float() > 0.5 {
        (d.place)(x, y + 2, z, log_block.id(), true);
    }
    let log = if d.rng.float() > 0.5 { log_block } else { hollow_log_block };
    place_log(d, log, log_length);
}
fn place_oak_impl(data: &mut TreePlaceData, allow_stump: bool) {
    static LOG: &Block = &OAK_LOG;
    static HOLLOW_LOG: Block = Block::named("oak_log_hollow");
    static WOOD: &Block = &OAK_WOOD;
    static ALLOW_ROOTS: bool = true;
    let height = 6u16 + (data.rng.next() as u16) % 3u16;
    if allow_stump && check_stump(data, height) {
        place_stump(data, LOG, &HOLLOW_LOG, WOOD, height, ALLOW_ROOTS);
        return;
    }
    let skeleton = place_trunk(data, height, LOG, WOOD, true, ALLOW_ROOTS, TrunkLean::no_lurch());
    place_foliage(&skeleton, data, height, OAK_LEAVES.id(), (2.0, 0.75));
    decorate_trunk(&skeleton, data);
}
fn place_oak(data: &mut TreePlaceData) {
    place_oak_impl(data, true)
}
fn place_birch(data: &mut TreePlaceData) {
    static LOG: &Block = &BIRCH_LOG;
    static HOLLOW_LOG: Block = Block::named("birch_log_hollow");
    static WOOD: &Block = &BIRCH_WOOD;
    static ALLOW_ROOTS: bool = false;
    let height = 6u16 + (data.rng.next() as u16) % 4u16;
    if check_stump(data, height) {
        place_stump(data, LOG, &HOLLOW_LOG, WOOD, height, ALLOW_ROOTS);
        return;
    }
    let skeleton = place_trunk(data, height, LOG, WOOD, true, ALLOW_ROOTS, TrunkLean::no_lurch());
    place_foliage(&skeleton, data, height, BIRCH_LEAVES.id(), (2.0, 0.75));
    decorate_trunk(&skeleton, data);
}
fn place_spruce(data: &mut TreePlaceData) {
    let height = 6u16 + (data.rng.next() as u16) % 4u16;
    place_trunk(data, height, &SPRUCE_LOG, &SPRUCE_WOOD, false, false, TrunkLean::no_lurch());
    place_cone_foliage(data, height, SPRUCE_LEAVES.id());
}
fn place_palm_foliage(d: &mut TreePlaceData, skeleton: &TreeSkeleton) {
    const FRONDS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
    let Some((x, mut y, z)) = skeleton.top else { return };
    y -= 1;
    let leaves = PALM_LEAVES.id();
    (d.place)(x, y + 1, z, leaves, false);
    (d.place)(x, y + 2, z, leaves, false);
    for (dx, dz) in FRONDS {
        let len = 3 + (d.rng.next() % 2) as i32;
        for i in 1..=len {
            let px = x + dx * i;
            let pz = z + dz * i;
            let py = if i >= len - 1 { y } else { y + 1 };
            (d.place)(px, py, pz, leaves, false);
            if i > 1 && i < len {
                (d.place)(px, py + 1, pz, leaves, false);
            }
        }
    }
}
fn place_palm_coconuts(d: &mut TreePlaceData, skeleton: &TreeSkeleton) {
    let Some((x, mut y, z)) = skeleton.top else { return };
    y -= 1;
    for (dx, dz, dir) in DIRECTIONS {
        if d.rng.float() >= 0.333 { continue }
        (d.place)(x + dx, y, z + dz, COCONUT.with(&[("facing", dir)]), false);
    }
}
fn place_palm(data: &mut TreePlaceData) {
    let height = 6u16 + (data.rng.next() as u16) % 4u16;
    let lean = TrunkLean::random(&mut data.rng, 30.0);
    let skeleton = place_trunk(data, height, &PALM_LOG, &PALM_WOOD, false, false, lean);
    place_palm_foliage(data, &skeleton);
    place_palm_coconuts(data, &skeleton);
}
fn place_trunk_branchy(d: &mut TreePlaceData, height: u16, log_block: &Block, wood_block: &Block, allow_roots: bool) -> TreeSkeleton {
    let (x, y, z) = d.pos;
    let log = log_block.id();
    let mut branch_tips: Vec<(i32, i32, i32)> = Vec::new();
    let (mut dx, mut dz) = (0, 0);
    let place_branch = |d: &mut TreePlaceData, branch_tips: &mut Vec<(i32, i32, i32)>, x: i32, y: i32, z: i32, length: i32| {
        let (dx, dz, _) = random_direction(&mut d.rng);
        for dy in 0..length {
            (d.place)(x + dx * dy, y + dy, z + dz * dy, log, true);
        }
        branch_tips.push((x + dx * length, y + length, z + dz * length));
    };
    let mut moved = false;
    for dy in 0..(height as i32 - 3) {
        let (bx, by, bz) = (x + dx, y + dy, z + dz);
        (d.place)(bx, by, bz, log, true);
        if dy >= height as i32 / 3 {
            if d.rng.float() < 0.3 {
                place_branch(d, &mut branch_tips, bx, by, bz, height as i32 - dy - 1);
            } else if !moved && d.rng.float() < 0.2 {
                let mv = random_direction(&mut d.rng);
                dx += mv.0;
                dz += mv.1;
                moved = true;
            }
        }
    }
    place_branch(d, &mut branch_tips, x, y + height as i32 - 3, z, 3);
    if allow_roots {
        place_roots(d, log_block, wood_block);
    }
    TreeSkeleton { height: height as i32, top: None, bottom: d.pos, branch_tips }
}
fn place_acacia(data: &mut TreePlaceData) {
    let height = 6u16 + (data.rng.next() as u16) % 4u16;
    let skeleton = place_trunk_branchy(data, height, &ACACIA_LOG, &ACACIA_WOOD, true);
    place_foliage(&skeleton, data, height, ACACIA_LEAVES.id(), (4.0, 0.4));
}
pub fn register() {
    register_tree("base:oak", place_oak);
    register_tree("base:birch", place_birch);
    register_tree("base:spruce", place_spruce);
    register_tree("base:palm", place_palm);
    register_tree("base:acacia", place_acacia);
}
/// Places a tree into a live world (saplings). Checks for room first.
pub fn grow_tree(w: World, p: [i32; 3]) {
    // A tree needs a clear trunk column before we start replacing the sapling.
    for y in p[1] + 1..p[1] + 12 {
        match w.block_loaded([p[0], y, p[2]]) {
            Some(s) if s == 0 || TALL_GRASS.is(s) || TALL_TALL_GRASS.is(s) => {}
            _ => return,
        }
    }

    let mut data = TreePlaceData {
        pos: (p[0], p[1], p[2]),
        rng: Rng(w.random()),
        place: Box::new(move |x, y, z, state, force| {
            let Some(cur) = w.block_loaded([x, y, z]) else { return };
            if force || cur == 0 || TALL_GRASS.is(cur) || TALL_TALL_GRASS.is(cur) {
                w.set_block([x, y, z], state);
            }
        }),
        ground: Box::new(move |x, z| {
            // The live world has no column-height query, so find the top solid block near the
            // sapling. Stop at the loaded edge rather than pretending air is ground.
            for y in (p[1].saturating_sub(64)..=p[1] + 32).rev() {
                let Some(state) = w.block_loaded([x, y, z]) else { continue };
                if block_info(state).is_some_and(|i| i.solid()) {
                    return y;
                }
            }
            p[1] - 1
        }),
    };

    place_oak_impl(&mut data, false)
}
