use weft_sdk::*;

pub static TREES: Component<f32> = Component::named("trees");
pub static GRASS_COVER: Component<f32> = Component::named("grass");
pub static TALL_GRASS_COVER: Component<f32> = Component::named("tall_grass");
pub static BOULDER_COVER: Component<f32> = Component::named("boulder");
pub static RASPBERRY_COVER: Component<f32> = Component::named("raspberry");
pub static SNOWBERRY_COVER: Component<f32> = Component::named("snowberry");
pub static BUSH_COVER: Component<f32> = Component::named("bush");
pub static FUNGI_COVER: Component<f32> = Component::named("fungi");
pub static FLOWER_COVER: Component<f32> = Component::named("flower");
pub static FERN_COVER: Component<f32> = Component::named("fern");
pub static BLUEGRASS_COVER: Component<f32> = Component::named("bluegrass");
pub static SHORT_GRASS_COVER: Component<f32> = Component::named("short_grass");
pub static CLOVER_COVER: Component<f32> = Component::named("clover");
pub static COARSE_DIRT_COVER: Component<f32> = Component::named("coarse_dirt");
pub static DRYGRASS_COVER: Component<f32> = Component::named("drygrass");
pub static SHORT_DRYGRASS_COVER: Component<f32> = Component::named("short_drygrass");
pub static GLOBULAR_CACTI_COVER: Component<f32> = Component::named("globular_cacti_cover");
pub static COLUMNAR_CACTI_COVER: Component<f32> = Component::named("columnar_cacti_cover");
pub static DRY_FERN_COVER: Component<f32> = Component::named("dry_fern");
/// Flowers a biome grows, picked evenly (`"base:dandelion base:rose"` where unset).
pub static FLOWERS: Component<BlockList> = Component::named("flowers");
pub static FROZEN: Tag = Tag::named("frozen");
pub static DRY: Tag = Tag::named("dry");

pub static PLAINS: Biome = Biome::named("plains");
pub static LAVENDER_FIELDS: Biome = Biome::named("lavender_fields");
pub static HILLS: Biome = Biome::named("hills");
pub static TALLGRASS_MEADOW: Biome = Biome::named("tallgrass_meadow");
pub static CLOVER_MEADOW: Biome = Biome::named("clover_meadow");
pub static FOREST: Biome = Biome::named("forest");
pub static BIRCH_FOREST: Biome = Biome::named("birch_forest");
pub static TAIGA: Biome = Biome::named("taiga");
pub static SNOWY: Biome = Biome::named("snowy_plains");
pub static DESERT: Biome = Biome::named("desert");
pub static MOUNTAINS: Biome = Biome::named("mountains");
pub static SNOWY_PEAKS: Biome = Biome::named("snowy_peaks");
pub static OCEAN: Biome = Biome::named("ocean");
pub static BEACH: Biome = Biome::named("beach");
pub static DEEP_CAVES: Biome = Biome::named("deep_caves");
pub static DUNES: Biome = Biome::named("dunes");
pub static GRASSY_DESERT: Biome = Biome::named("grassy_desert");
pub static SEMIDESERT: Biome = Biome::named("semidesert");

pub static PLANT_SOIL: Tag = Tag::named("plant_soil");
pub static CACTUS_SOIL: Tag = Tag::named("cactus_soil");
pub static SUGAR_CANE_SOIL: Tag = Tag::named("sugar_cane_soil");
pub static GENERAL_SOIL: Tag = Tag::named("general_soil");
