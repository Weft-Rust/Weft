use weft_sdk::*;

use crate::items::{FLAMMABLE, FUEL};
use crate::worldgen;

pub static STONE: Block = Block::named("stone");
pub static COBBLESTONE: Block = Block::named("cobblestone");
pub static MOSSY_COBBLESTONE: Block = Block::named("mossy_cobblestone");
pub static DIRT: Block = Block::named("dirt");
pub static GRASS: Block = Block::named("grass");
pub static SNOWY_GRASS: Block = Block::named("snowy_grass");
pub static SAND: Block = Block::named("sand");
pub static GRAVEL: Block = Block::named("gravel");
pub static CLAY: Block = Block::named("clay");
pub static SANDSTONE: Block = Block::named("sandstone");
pub static SNOW: Block = Block::named("snow");
pub static ICE: Block = Block::named("ice");
pub static OBSIDIAN: Block = Block::named("obsidian");
pub static COAL_ORE: Block = Block::named("coal_ore");
pub static IRON_ORE: Block = Block::named("iron_ore");
pub static GOLD_ORE: Block = Block::named("gold_ore");
pub static DIAMOND_ORE: Block = Block::named("diamond_ore");
pub static OAK_LOG: Block = Block::named("oak_log");
pub static OAK_WOOD: Block = Block::named("oak_wood");
pub static BIRCH_LOG: Block = Block::named("birch_log");
pub static BIRCH_WOOD: Block = Block::named("birch_wood");
pub static SPRUCE_LOG: Block = Block::named("spruce_log");
pub static SPRUCE_WOOD: Block = Block::named("spruce_wood");
pub static OAK_PLANKS: Block = Block::named("oak_planks");
pub static BIRCH_PLANKS: Block = Block::named("birch_planks");
pub static SPRUCE_PLANKS: Block = Block::named("spruce_planks");
pub static OAK_LEAVES: Block = Block::named("oak_leaves");
pub static BIRCH_LEAVES: Block = Block::named("birch_leaves");
pub static SPRUCE_LEAVES: Block = Block::named("spruce_leaves");
/// What a cactus does to who touches it.
pub static PRICKED: DamageCause = DamageCause::named("cactus");
pub static WATER: Block = Block::named("water");
pub static LAVA: Block = Block::named("lava");
pub static TALL_GRASS: Block = Block::named("tall_grass");
pub static DANDELION: Block = Block::named("dandelion");
pub static ROSE: Block = Block::named("rose");
pub static SAPLING: Block = Block::named("sapling");
pub static SUGAR_CANE: Block = Block::named("sugar_cane");
pub static CACTUS: Block = Block::named("cactus");
pub static WHEAT: Block = Block::named("wheat");
pub static FARMLAND: Block = Block::named("farmland");
pub static GLASS: Block = Block::named("glass");
pub static CRAFTING_TABLE: Block = Block::named("crafting_table");
pub static FURNACE: Block = Block::named("furnace");
pub static CHEST: Block = Block::named("chest");
pub static TORCH: Block = Block::named("torch");
pub static LADDER: Block = Block::named("ladder");
pub static GLOWSTONE: Block = Block::named("glowstone");
pub static BRICKS: Block = Block::named("bricks");
pub static STONE_BRICKS: Block = Block::named("stone_bricks");
pub static IRON_BLOCK: Block = Block::named("iron_block");
pub static GOLD_BLOCK: Block = Block::named("gold_block");
pub static DIAMOND_BLOCK: Block = Block::named("diamond_block");
pub static COAL_BLOCK: Block = Block::named("coal_block");
pub static WOOL: Block = Block::named("wool");
pub static TALL_TALL_GRASS: Block = Block::named("tall_tallgrass");
pub static BUSH_STEM: Block = Block::named("bush_stem");
pub static RASPBERRY_LEAVES: Block = Block::named("raspberry_leaves");
pub static SNOWBERRY_LEAVES: Block = Block::named("snowberry_leaves");
pub static MOSS: Block = Block::named("moss");
pub static PORCINI_MUSHROOM: Block = Block::named("porcini_mushroom");
pub static CHANTERELLE_MUSHROOM: Block = Block::named("chanterelle_mushroom");
pub static TOADSTOOL_MUSHROOM: Block = Block::named("toadstool_mushroom");
pub static DRYADS_SADDLE_MUSHROOM: Block = Block::named("dryads_saddle_mushroom");
pub static BUSH_LEAVES: Block = Block::named("bush_leaves");
pub static FERN: Block = Block::named("fern");
pub static LAVENDER: Block = Block::named("lavender");
pub static BLUEGRASS: Block = Block::named("bluegrass");

static TILLABLE: Tag = Tag::named("tillable");

/// A log that grows into a tree of `leaves` (a cone, like a spruce, when `cone`).
fn log(r: &mut Registry, handle: &'static Block, wood: &str, wood_block: bool) {
    let side = format!("log_{wood}");
    let top = format!("log_{wood}_top");
    let b = r
        .block(handle)
        .sound_group("wood")
        .hardness(2.0)
        .mined_by("axe", tool_tier::HAND)
        .set(&FLAMMABLE, 5)
        .set(&FUEL, 300)
        .tab(&crate::tabs::NATURAL);
    if wood_block {
        b.tag(&format!("wood/{wood}"))
            .tag("wood")
            .facing(Facings::HORIZONTAL, Toward::Player)
            .texture(side.as_str());
    } else {
        b.tag(&format!("logs/{wood}"))
            .tag("logs")
            .axis()
            .texture(&side)
            .ends(&top);
    }
}

/// Dyed blocks' colors: name, glass tint, wool tint, the light the glass takes away per channel.
const COLORS: &[(&str, u32, u32, [u8; 3])] =
    &[("red", 0xED3737, 0xB02E26, [0, 12, 12]), ("green", 0x37C837, 0x5E7C16, [12, 0, 12]), ("blue", 0x375BFF, 0x3C44AA, [12, 12, 0])];

fn leaves(r: &mut Registry, handle: &'static Block, tex: &str, tint: bool) {
    let mut b = r.block(handle)
        .sound_group("grass")
        .texture(tex)
        .hardness(0.2)
        .cutout()
        .light_opacity(1)
        .tag("leaves")
        .signal_blocker()
        .mined_by("shears", tool_tier::HAND)
        .tab(&crate::tabs::NATURAL);
    if tint {
        b = b.tint_kind(&tints::FOLIAGE);
    }
    std::mem::drop(b);
    // Vanilla: a sapling one time in 20, oak leaves an apple one time in 200.
    let mut table = r.loot(handle).pool(
        Pool::new()
            .when(Cond::tool("#tools/shears"))
            .item(handle.name()),
    );
    let no_shears = !Cond::tool("#tools/shears");
    table = table.pool(
        Pool::new()
            .when(no_shears.clone())
            .when(Cond::chance(0.05))
            .item("sapling"),
    );
    if std::ptr::eq(handle, &OAK_LEAVES) {
        table.pool(
            Pool::new()
                .when(no_shears)
                .when(Cond::chance(0.005))
                .item("apple"),
        );
    }
}
fn berry_leaves(r: &mut Registry, handle: &'static Block, texture: &'static str, texture_berried: &'static str, item: &'static Item) {
    r.block(handle)
        .sound_group("grass")
        .prop("berried", ["false", "true"])
        .hardness(0.2)
        .cutout()
        .light_opacity(1)
        .tag("leaves")
        .signal_blocker()
        .mined_by("shears", tool_tier::HAND)
        .tab(&crate::tabs::NATURAL)
        .texture(texture_berried)
        .states(|p, s| s.texture(if p.is("berried", "true") { texture_berried } else { texture }))
        .on_use(move |u| pick_berries(u, item));
    r.loot(handle)
        .pool(
            Pool::new()
                .when(Cond::tool("#tools/shears"))
                .item(handle.name()),
        )
        .pool(
            Pool::new()
                .when(Cond::state("berried=true"))
                .items(item.name(), 1, 1),
        );
}

fn plant(r: &mut Registry, handle: &'static Block, tex: &str) {
    r.block(handle)
        .sound_group("grass")
        .texture(tex)
        .plant()
        .replaceable()
        .hitbox([5.0, 0.0, 5.0], [11.0, 10.0, 11.0])
        .needs_below("#plant_soil")
        .tab(&crate::tabs::NATURAL);
}

fn pick_berries(u: &mut Use, berry: &'static Item) -> bool {
    let w = u.world();
    let p = u.pos();
    if w.prop(p, "berried") != Some("true") {
        return false;
    }
    if !w.prop_set(p, "berried", "false") {
        return false;
    }
    w.spawn_item(block_center(p), berry.stack(1));
    true
}


/// Sand and gravel: with nothing under them they fall as an entity and land as a block.
pub fn register(r: &mut Registry) {
    // Any block of any mod tagged so comes in these too.
    r.tag(
        "has_shapes",
        &[
            "base:stone",
            "base:cobblestone",
            "base:mossy_cobblestone",
            "base:sandstone",
            "base:bricks",
            "base:stone_bricks",
            "base:oak_planks",
            "base:birch_planks",
            "base:spruce_planks",
        ],
    );
    r.stairs("#has_shapes").tab(&crate::tabs::BUILDING);
    r.slabs("#has_shapes").tab(&crate::tabs::BUILDING);
    r.vertical_slabs("#has_shapes").tab(&crate::tabs::BUILDING);
    r.beams("#has_shapes").tab(&crate::tabs::BUILDING);
    r.eighths("#has_shapes").tab(&crate::tabs::BUILDING);
    r.block(&STONE)
        .sound_group("stone")
        .hardness(1.5)
        .mined_by("pickaxe", 0)
        .drops("cobblestone", 1)
        .tag("stone")
        .tag("smooth_stone")
        .tag("stone_ore_replaceables")
        .tab(&crate::tabs::BUILDING);
    r.block(&COBBLESTONE)
        .sound_group("stone")
        .hardness(2.0)
        .mined_by("pickaxe", 0)
        .tag("stone")
        .tag("cobblestone")
        .tag("materials/stone")
        .tab(&crate::tabs::BUILDING);
    r.block(&MOSSY_COBBLESTONE)
        .sound_group("stone")
        .hardness(2.0)
        .mined_by("pickaxe", 0)
        .tab(&crate::tabs::BUILDING);
    r.block(&DIRT)
        .sound_group("gravel")
        .hardness(0.5)
        .mined_by("shovel", tool_tier::HAND)
        .tag("dirt")
        .tag("tillable")
        .tab(&crate::tabs::NATURAL);
    r.block(&GRASS)
        .sound_group("grass")
        .texture("dirt")
        .top("grass_top")
        .tint_kind(&tints::GRASS)
        .sides("dirt")
        .layer("grass_side_overlay")
        .tint_kind(&tints::GRASS)
        .hardness(0.6)
        .mined_by("shovel", tool_tier::HAND)
        .drops("dirt", 1)
        .tag("dirt")
        .tag("tillable")
        // Vanilla's pace: a minute or so under a block, many minutes to cover bare dirt.
        .spreads("#grass_spreads_to", 765.0, 9)
        .smothered_into("dirt", 68.0)
        .tab(&crate::tabs::NATURAL);
    r.block(&SNOWY_GRASS)
        .sound_group("grass")
        .texture("grass_snow_side")
        .top("snow")
        .bottom("dirt")
        .hardness(0.6)
        .mined_by("shovel", tool_tier::HAND)
        .drops("dirt", 1)
        .tag("dirt")
        .tag("tillable")
        .tab(&crate::tabs::NATURAL);
    r.block(&SAND)
        .sound_group("sand")
        .hardness(0.5)
        .mined_by("shovel", tool_tier::HAND)
        .tag("sand")
        .falling()
        .tab(&crate::tabs::NATURAL);
    r.block(&GRAVEL)
        .sound_group("gravel")
        .hardness(0.6)
        .mined_by("shovel", tool_tier::HAND)
        .falling()
        .tab(&crate::tabs::NATURAL);
    // Vanilla: flint one time in ten instead of the gravel.
    r.loot(&GRAVEL).pool(
        Pool::new()
            .when(Cond::harvested())
            .entry(Entry::item("flint"))
            .entry(Entry::item("gravel").weight(9)),
    );
    r.block(&CLAY)
        .sound_group("gravel")
        .hardness(0.6)
        .mined_by("shovel", tool_tier::HAND)
        .tag("clay")
        .drops("clay_ball", 4)
        .tab(&crate::tabs::NATURAL);
    r.block(&SANDSTONE)
        .sound_group("stone")
        .texture("sandstone_side")
        .top("sandstone_top")
        .bottom("sandstone_bottom")
        .hardness(0.8)
        .mined_by("pickaxe", 0)
        .tab(&crate::tabs::BUILDING);
    r.block(&SNOW)
        .sound_group("sand")
        .hardness(0.2)
        .mined_by("shovel", tool_tier::HAND)
        .tab(&crate::tabs::NATURAL);
    // Blue lamps and glass are made with it.
    r.block(&ICE)
        .signal_blocker()
        .sound_group("glass")
        .tag("ice")
        .tag("dyes/blue")
        .hardness(0.5)
        .translucent()
        .light_opacity(2)
        .friction(0.98)
        .mined_by("pickaxe", tool_tier::HAND)
        .drops_nothing()
        .tab(&crate::tabs::NATURAL);
    r.block(&OBSIDIAN)
        .immovable()
        .sound_group("stone")
        .hardness(50.0)
        .mined_by("pickaxe", tool_tier::DIAMOND)
        .tab(&crate::tabs::NATURAL);
    r.block(&COAL_ORE)
        .sound_group("stone")
        .hardness(3.0)
        .mined_by("pickaxe", 0)
        .drops("coal", 1)
        .tag("ores")
        .tag("ores/coal")
        .tab(&crate::tabs::NATURAL);
    r.block(&IRON_ORE)
        .sound_group("stone")
        .hardness(3.0)
        .mined_by("pickaxe", tool_tier::STONE)
        .drops("raw_iron", 1)
        .tag("ores")
        .tag("ores/iron")
        .tab(&crate::tabs::NATURAL);
    r.block(&GOLD_ORE)
        .sound_group("stone")
        .hardness(3.0)
        .mined_by("pickaxe", tool_tier::IRON)
        .drops("raw_gold", 1)
        .tag("ores")
        .tag("ores/gold")
        .tab(&crate::tabs::NATURAL);
    r.block(&DIAMOND_ORE)
        .sound_group("stone")
        .hardness(3.0)
        .mined_by("pickaxe", tool_tier::IRON)
        .drops("diamond", 1)
        .tag("ores")
        .tag("ores/diamond")
        .tab(&crate::tabs::NATURAL);

    log(r, &OAK_LOG, "oak", false);
    log(r, &BIRCH_LOG, "birch", false);
    log(r, &SPRUCE_LOG, "spruce", false);
    log(r, &OAK_WOOD, "oak", true);
    log(r, &BIRCH_WOOD, "birch", true);
    log(r, &SPRUCE_WOOD, "spruce", true);
    r.block(&BUSH_STEM)
        .sound_group("grass")
        .hardness(0.2)
        .cutout()
        .light_opacity(1)
        .signal_blocker()
        .mined_by("shears", tool_tier::HAND)
        .tab(&crate::tabs::NATURAL);
    for (h, wood) in [(&OAK_PLANKS, "oak"), (&BIRCH_PLANKS, "birch"), (&SPRUCE_PLANKS, "spruce")] {
        r.block(h)
            .sound_group("wood")
            .texture(&format!("planks_{wood}"))
            .hardness(2.0)
            .mined_by("axe", tool_tier::HAND)
            .tag("planks")
            .tag(&format!("planks/{wood}"))
            .tag("materials/wood")
            .set(&FLAMMABLE, 5)
            .set(&FUEL, 300)
            .tab(&crate::tabs::BUILDING);
    }
    leaves(r, &OAK_LEAVES, "leaves_oak", true);
    leaves(r, &BIRCH_LEAVES, "leaves_birch", true);
    leaves(r, &SPRUCE_LEAVES, "leaves_spruce", true);
    leaves(r, &BUSH_LEAVES, "raspberry_leaves", false);
    berry_leaves(r, &RASPBERRY_LEAVES, "raspberry_leaves", "raspberry_leaves_berried", &crate::items::RASPBERRY);
    berry_leaves(r, &SNOWBERRY_LEAVES, "raspberry_leaves", "snowberry_leaves_berried", &crate::items::SNOWBERRY);
    // Vanilla numbers: water steps every 5 ticks and reaches 7 blocks, lava every 30 and 3.
    // Players swim in water; what they wear may add more (`SWIMS_IN` on an item).
    r.edit(&PLAYER).set_text(&SWIMS_IN, "#water");
    r.liquid(&WATER)
        .block(|b| {
            b.tint_kind(&tints::WATER)
                .translucent()
                .set_text(&DRIP, abi::particle::DRIP)
                .set_text(&SPLASH, abi::particle::SPLASH)
                .set_text(&BUBBLE, abi::particle::BUBBLE)
                .tag("water")
        })
        .renewable(2)
        .fog(0x050533, 96.0)
        .bucket(Item::generated("water_bucket".into()), &crate::items::BUCKET, &crate::tabs::TOOLS);
    r.liquid(&LAVA)
        .block(|b| {
            b.light_rgb(15, 10, 4)
                .drag(0.4)
                .tag("lava")
                .tag("cools_into_stone")
        })
        .flow_delay(30)
        .drop_off(2)
        .slope_find(2)
        .heat(1300)
        .fog(0x991A00, 1.0)
        .no_overlay()
        .bucket(Item::generated("lava_bucket".into()), &crate::items::BUCKET, &crate::tabs::TOOLS);
    r.liquid_reaction("#cools_into_stone", "#water")
        .source_turns_into("obsidian")
        .flowing_turns_into("cobblestone")
        .below_turns_into("stone");

    r.block(&TALL_GRASS)
        .sound_group("grass")
        .texture("tallgrass")
        .plant()
        .replaceable()
        .needs_below("#plant_soil")
        .tint_kind(&tints::GRASS)
        .tab(&crate::tabs::NATURAL);
    r.loot(&TALL_GRASS)
        .pool(Pool::new().when(Cond::chance(0.125)).item("wheat_seeds"));
    r.block(&TALL_TALL_GRASS)
        .sound_group("grass")
        .texture("tall_tallgrass_bottom")
        .tall_plant("tall_tallgrass_bottom", "tall_tallgrass_top")
        .replaceable()
        // .two_part("half", ["lower", "upper"], |_| Dir::Up)
        // .state_where("half=upper", |s| s.texture("tall_tallgrass_top"))
        // .state_where("half=lower", |s| s.needs_below("#plant_soil"))
        .tint_kind(&tints::GRASS)
        .tab(&crate::tabs::NATURAL);
    r.loot(&TALL_TALL_GRASS)
        .pool(Pool::new().when(Cond::chance(0.25)).item("wheat_seeds"));
    r.block(&BLUEGRASS)
        .sound_group("grass")
        .plant()
        .replaceable()
        .needs_below("#plant_soil")
        .tint_kind(&tints::GRASS)
        .tab(&crate::tabs::NATURAL);
    plant(r, &DANDELION, "flower_dandelion");
    plant(r, &ROSE, "flower_rose");
    r.block(&LAVENDER)
        .sound_group("grass")
        .tall_plant("lavender_bottom", "lavender_top")
        .replaceable()
        .needs_below("#plant_soil")
        .tab(&crate::tabs::NATURAL);

    r.block(&SAPLING)
        .sound_group("grass")
        .texture("sapling_oak")
        .plant()
        .needs_below("#plant_soil")
        .grows(470.0, 9)
        .on_grow(|e| worldgen::grow_tree(e.world(), e.pos().to_array()))
        .tab(&crate::tabs::NATURAL);
    r.block(&SUGAR_CANE)
        .sound_group("grass")
        .texture("reeds")
        .item_texture("reeds")
        .plant()
        .needs_below("#sugar_cane_soil")
        .needs_near("#water")
        .grows(270.0, 0)
        .on_grow(|e| grow_column(e, &SUGAR_CANE))
        .tab(&crate::tabs::NATURAL);
    r.damage_cause(&PRICKED).death_message("death.base.cactus");
    r.block(&CACTUS)
        .sound_group("cloth")
        .texture("cactus_side")
        .ends("cactus_top")
        .model_box([1.0, 0.0, 1.0], [15.0, 16.0, 15.0])
        .hitbox([1.0, 0.0, 1.0], [15.0, 16.0, 15.0])
        .hardness(0.4)
        .needs_below("#cactus_soil")
        .hazard(Hazard::touch(1.0, &PRICKED))
        .grows(270.0, 0)
        .on_grow(|e| grow_column(e, &CACTUS))
        .tab(&crate::tabs::NATURAL);
    r.block(&WHEAT)
        .sound_group("grass")
        .prop("age", 0..=7)
        .states(|p, s| s.texture(&format!("wheat_stage_{}", p.get("age"))))
        .plant()
        .needs_below("#crop_soil")
        .no_item()
        .grows(205.0, 9);
    // Modern's: unripe gives its seed back, ripe gives the wheat and up to 3 more seeds.
    r.loot(&WHEAT)
        .pool(Pool::new().when(Cond::state("age=7")).item("wheat"))
        .pool(Pool::new().entry(Entry::item("wheat_seeds")))
        .pool(
            Pool::new()
                .when(Cond::state("age=7"))
                .entry(Entry::item("wheat_seeds").count(0, 3)),
        );
    r.block(&FARMLAND)
        .sound_group("gravel")
        .texture("dirt")
        .top("farmland")
        .model_box([0.0, 0.0, 0.0], [16.0, 15.0, 16.0])
        .hardness(0.6)
        .mined_by("shovel", tool_tier::HAND)
        .drops("dirt", 1)
        .tab(&crate::tabs::NATURAL);

    r.block(&GLASS)
        .signal_blocker()
        .sound_group("glass")
        .tag("glass")
        .cutout()
        .cull_same()
        .hardness(0.3)
        .drops_nothing()
        .tab(&crate::tabs::BUILDING);
    // Every color of glass and wool is one white picture tinted: a color more is a line here.
    for &(name, glass, wool, filter) in COLORS {
        r.block(Block::generated(format!("{name}_stained_glass")))
            .signal_blocker()
            .sound_group("glass")
            .tag(&format!("stained_glass/{name}"))
            .texture("stained_glass")
            .tint(glass)
            .translucent()
            .light_filter(filter[0], filter[1], filter[2])
            .hardness(0.3)
            .tab(&crate::tabs::COLORED);
        r.block(Block::generated(format!("{name}_wool")))
            .sound_group("cloth")
            .tag(&format!("wool/{name}"))
            .texture("wool_white")
            .tint(wool)
            .hardness(0.8)
            .set(&FLAMMABLE, 30)
            .tab(&crate::tabs::COLORED);
    }
    for (name, rgb) in [("red", [15u8, 3, 2]), ("green", [2, 15, 4]), ("blue", [3, 6, 15])] {
        r.block(Block::generated(format!("{name}_lamp")))
            .sound_group("glass")
            .tag(&format!("lamps/{name}"))
            .texture(&format!("lamp_{name}"))
            .light_rgb(rgb[0], rgb[1], rgb[2])
            .hardness(0.3)
            .tab(&crate::tabs::COLORED);
    }
    r.block(&CRAFTING_TABLE)
        .sound_group("wood")
        .texture("crafting_table_side")
        .bottom("crafting_table_bottom")
        .top("crafting_table_top")
        .north("crafting_table_front")
        .south("crafting_table_front")
        .hardness(2.5)
        .mined_by("axe", tool_tier::HAND)
        .set(&FUEL, 300)
        .opens(&crate::player::CRAFTING)
        .tab(&crate::tabs::FUNCTIONAL);
    r.block(&TORCH)
        .sound_group("wood")
        .ambient_particles(&["core:flame", "core:smoke"])
        .facing(TORCH_FACINGS, Toward::Face)
        .states(|p, s| torch_state(p.facing(), s, "torch"))
        .item_texture("torch")
        .not_solid()
        .light_rgb(15, 13, 10)
        .hardness(0.0)
        .tab(&crate::tabs::FUNCTIONAL);
    // On walls only, up to four in one block.
    r.block(&LADDER)
        .sound_group("wood")
        .on_sides(Facings::HORIZONTAL)
        .climbable()
        .not_solid()
        .hardness(0.4)
        .tab(&crate::tabs::FUNCTIONAL);
    r.block(&GLOWSTONE)
        .sound_group("glass")
        .tag("glowstone")
        .light(15)
        .hardness(0.3)
        .tab(&crate::tabs::NATURAL);
    r.block(&BRICKS)
        .west("bricks_side_1")
        .east("bricks_side_4")
        .bottom("bricks_bottom")
        .top("bricks_top")
        .north("bricks_side_3")
        .south("bricks_side_2")
        .sound_group("stone")
        .hardness(2.0)
        .mined_by("pickaxe", 0)
        .tab(&crate::tabs::BUILDING);
    r.block(&STONE_BRICKS)
        .sound_group("stone")
        .west("stonebrick_side_1")
        .east("stonebrick_side_4")
        .bottom("stonebrick_bottom")
        .top("stonebrick_top")
        .north("stonebrick_side_3")
        .south("stonebrick_side_2")
        .hardness(1.5)
        .mined_by("pickaxe", 0)
        .tab(&crate::tabs::BUILDING);
    r.block(&IRON_BLOCK)
        .sound_group("stone")
        .hardness(5.0)
        .mined_by("pickaxe", tool_tier::STONE)
        .tag("ore_blocks/iron")
        .tab(&crate::tabs::BUILDING);
    r.block(&GOLD_BLOCK)
        .sound_group("stone")
        .hardness(3.0)
        .mined_by("pickaxe", tool_tier::IRON)
        .tag("ore_blocks/gold")
        .tab(&crate::tabs::BUILDING);
    r.block(&DIAMOND_BLOCK)
        .sound_group("stone")
        .hardness(5.0)
        .mined_by("pickaxe", tool_tier::IRON)
        .tag("ore_blocks/diamond")
        .tab(&crate::tabs::BUILDING);
    r.block(&COAL_BLOCK)
        .sound_group("stone")
        .hardness(5.0)
        .mined_by("pickaxe", 0)
        .tag("ore_blocks/coal")
        .set(&FUEL, 16000)
        .tab(&crate::tabs::BUILDING);
    r.block(&WOOL)
        .sound_group("cloth")
        .texture("wool_white")
        .hardness(0.8)
        .set(&FLAMMABLE, 30)
        .tab(&crate::tabs::COLORED);
    r.block(&MOSS)
        .on_sides(Facings::ALL)
        .sound_group("grass")
        .shares_cell_with("#water")
        .hardness(0.1)
        .signal_blocker()
        .tab(&crate::tabs::NATURAL);
    r.block(&PORCINI_MUSHROOM)
        .sound_group("grass")
        .shares_cell_with("#water")
        .hardness(0.1)
        .needs_below("")
        .plant()
        .texture("porcini_mushroom")
        .drops("porcini", 1)
        .tab(&crate::tabs::NATURAL);
    r.block(&CHANTERELLE_MUSHROOM)
        .sound_group("grass")
        .shares_cell_with("#water")
        .hardness(0.1)
        .needs_below("")
        .plant()
        .texture("chanterelle_mushroom")
        .drops("chanterelle", 1)
        .tab(&crate::tabs::NATURAL);
    r.block(&TOADSTOOL_MUSHROOM)
        .sound_group("grass")
        .shares_cell_with("#water")
        .hardness(0.1)
        .needs_below("")
        .plant()
        .texture("toadstool_mushroom")
        .drops("toadstool", 1)
        .tab(&crate::tabs::NATURAL);
    let uv = [0.1, 0.1, 15.9, 7.9];
    let top_uv = [0.1, 0.1, 15.9, 7.9];
    let bottom_uv = [0.1, 8.1, 15.9, 15.9];
    fn flip_v(uv: [f32; 4]) -> [f32; 4] {
        [uv[0], uv[3], uv[2], uv[1]]
    }
    fn shelf_faces(top: [f32; 4], bottom: [f32; 4]) -> [(&'static str, [f32; 4]); 6] {
        let tex = "dryads_saddle_mushroom";
        let none = (tex, UV_NO_FACE);
        [none, none, (tex, bottom), (tex, top), none, none]
    }
    r.block(&DRYADS_SADDLE_MUSHROOM)
        .sound_group("wood")
        .shares_cell_with("#water")
        .hardness(0.1)
        .texture("dryads_saddle_mushroom")
        .facing(Facings::HORIZONTAL, Toward::Face)
        .states(|_p, s| s.needs(Dir::South, ""))
        .model_box_faces(
            [0.0, 12.0, 8.0],
            [16.0, 12.2, 16.0],
            shelf_faces(top_uv, flip_v(top_uv)), // upper: top half both sides
        )
        .tilted([8.0, 12.0, 16.0], Dir::East, -18.0)
        .model_box_faces(
            [0.0, 7.0, 8.0],
            [16.0, 7.2, 16.0],
            shelf_faces(bottom_uv, flip_v(bottom_uv)), // lower: bottom half both sides
        )
        .tilted([8.0, 7.0, 16.0], Dir::East, -24.0)
        .not_solid()
        .hitbox([0.0, 6.0, 8.0], [16.0, 13.0, 16.0])
        .drops("dryads_saddle", 1)
        .tab(&crate::tabs::NATURAL);
    r.block(&FERN)
        .sound_group("grass")
        .shares_cell_with("#water")
        .hardness(0.1)
        .needs_below("#plant_soil")
        .tint_kind(&tints::GRASS)
        .tab(&crate::tabs::NATURAL)
        .not_solid()
        .hitbox([2.0, 0.0, 2.0], [14.0, 16.0, 14.0]);
    r.shapes_of("hollow_log", "#logs", "_hollow")
        .sound_group("wood")
        .model_file("hollow_log")
        .model_texture("inside", "{source.mod}:block/{source.name}_inner")
        .axis()
        .signal_blocker()
        .shares_cell_with("#water")
        .liquid_walls(&[Dir::North, Dir::South, Dir::East, Dir::West])
        .outline([0.0; 3], [16.0; 3])
        .outline([2.0, 0.0, 2.0], [14.0, 16.0, 14.0])
        .tag("hollow_log")
        .tab(&crate::tabs::NATURAL);
    r.loot(&FERN).pool(
        Pool::new()
            .when(Cond::tool("#tools/shears"))
            .item(FERN.name())
    );

    // What may grow or stand where, from the common `dirt` and `sand` tags like modern vanilla:
    // a mod's sand tagged `sand` gets cactus and sugar cane, in the world and from players.
    r.tag("plant_soil", &["#dirt"]);
    r.tag("dyes/red", &["rose"]);
    r.tag("dyes/green", &["cactus"]);
    r.tag("torches", &["torch"]);
    r.tag("furnaces", &["furnace"]);
    r.tag("crafting_tables", &["crafting_table"]);
    r.tag("cobblestone", &["cobblestone"]);
    r.tag("plant_soil", &["farmland"]);
    r.tag("sugar_cane_soil", &["#dirt"]);
    r.tag("sugar_cane_soil", &["#sand"]);
    r.tag("sugar_cane_soil", &["sugar_cane"]);
    r.tag("cactus_soil", &["#sand"]);
    r.tag("cactus_soil", &["cactus"]);
    r.tag("crop_soil", &["farmland"]);
    r.tag("grass_spreads_to", &["dirt"]);
    r.tag("animal_spawn", &["grass"]);

    // A hoe's own use, as vanilla's, any mod's hoe: the soil stays a block nothing happens to
    // when clicked, so a block placed on it shows at once.
    r.edit_pattern::<kind::Item>("#tools/hoe").on_use(till_soil);
}

fn till_soil(u: &mut ItemUse) -> bool {
    let w = u.world();
    let Some((pos, _)) = u.target_block() else { return false };
    if TILLABLE.on_block(w.block(pos)) {
        w.set_block(pos, FARMLAND.id());
        return true;
    }
    false
}

pub use weft_sdk::shapes::{TORCH_FACINGS, stick_faces, torch_state};

/// Sugar cane and cacti grow a block on top, up to three high.
fn grow_column(e: &mut BlockEvent, me: &'static Block) {
    let w = e.world();
    let p = e.pos();
    let above = [p.x, p.y + 1, p.z];
    if w.block(above) != 0 {
        return;
    }
    let mut height = 1;
    while height < 3 && me.is(w.block([p.x, p.y - height, p.z])) {
        height += 1;
    }
    if height < 3 {
        w.set_block(above, me.id());
    }
}
