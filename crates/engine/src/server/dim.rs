use crate::world::placement::{CellLayout, Part};
use std::cmp::Reverse;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tag_bits::damage as df;
use crate::tag_bits::entity as ef;
use glam::{DVec3, IVec3};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use weft_abi::{self as abi, ItemStack, block_flags as bf, damage_cause as damage, entry_kind as ek};

use super::entity::{Completion, Entities, Entity, Motion, Told, step};
use super::space::{BlockTimer, Space};
use crate::components as comps;
use crate::content::{AIR, Content, Then, When, content};
use crate::host;
use crate::world::cube::VOLUME;
use crate::world::light::{self, CubeMap, Lighter};
use crate::world::{Cube, Rng, cube_of, local_index};

const MAX_NEIGHBOR_UPDATES: usize = 1 << 16;
/// Missed ticks are made up in at most this many parts, all in one tick (`run_systems`).
const CATCH_UP_PARTS: u64 = 20;
const ITEM_LIFETIME: u64 = crate::world::TPS as u64 * 60 * 5;
/// The removal of what nobody was told came and so isn't told went: a refused spawn, and what is
/// blocks (moving blocks set down, a block entity whose block went), whose comings and goings are
/// the blocks'.
pub const UNTOLD: u32 = u32::MAX;

pub enum DimEvent {
    GrabReleased(u64),
    Controller {
        player: u64,
        controller: u64,
        face: u8,
        data: Vec<u8>,
    },
    HelmProfile {
        player: u64,
        mount: u64,
        controller: u64,
        data: Vec<u8>,
    },
    /// An entity started using an item that takes time (ticks, off hand), or stopped.
    Using {
        entity: u64,
        using: Option<(u32, bool)>,
    },
    /// An entity's use of an item did something.
    Used {
        entity: u64,
        off: bool,
    },
    /// `on`: the inventory it opened on, `None` for a menu of its own (a crafting table).
    /// `block`: the state it was opened on, 0 when opened from code.
    OpenMenu {
        player: u64,
        menu: u32,
        on: Option<crate::server::holder::Inv>,
        block: u32,
    },
    /// An entity died: a player (who respawns) or one with a name, which everyone hears of.
    Died {
        entity: u64,
        cause: u32,
        /// A player's nickname or the localized name of a mob.
        killer: Option<crate::ui::text::Arg>,
        /// Its own name; a player's comes from its session.
        name: Option<String>,
    },
    SetPermission {
        player: u64,
        level: u32,
    },
    SetWeather {
        dim: u32,
        weather: u16,
        ticks: u32,
    },

    /// Its arm swings for those who see it.
    EntitySwing {
        off: bool,
        id: u64,
    },
    /// A sprinting hit landed: the client stops sprinting.
    StopSprint {
        player: u64,
    },
    /// `Host::force_load`.
    ForceLoad {
        dim: u32,
        key: u64,
        min: IVec3,
        max: IVec3,
        mode: u32,
        ticks: u32,
    },
    /// `ToClient::Camera` to it.
    Camera {
        player: u64,
        mode: u32,
        locked: bool,
        transition: f32,
        easing: u32,
    },
    Packet {
        dim: u32,
        who: abi::Who,
        channel: u32,
        data: Vec<u8>,
    },
    /// `dim` u32::MAX = the region's own dimension.
    Teleport {
        from: u32,
        id: u64,
        dim: u32,
        pos: DVec3,
    },
    SetTime {
        dim: u32,
        time: u64,
    },
    SkipTicks {
        dim: u32,
        ticks: u64,
    },
    /// `server::area`, done at the end of the tick across every region of `dim`.
    AreaFill {
        dim: u32,
        fill: super::area::AreaFill,
    },
    SetMode {
        player: u64,
        mode: crate::world::GameMode,
    },
    Particles {
        dim: u32,
        who: abi::Who,
        burst: abi::ParticleBurst,
    },
    Sound {
        dim: u32,
        who: abi::Who,
        sound: u32,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
    },
    PlayAction {
        id: u64,
        name: String,
        looping: bool,
    },
    Show {
        dim: u32,
        who: abi::Who,
        shown: crate::net::Shown,
    },
}

/// Path searches a region does per tick at most (in parallel); more wait for the next tick (a
/// spawned crowd all starting to walk at once).
const PATHS_PER_TICK: usize = 128;

pub struct Dim {
    pub id: u32,
    /// The world seed.
    pub seed: u64,
    /// This region's fragment of the dimension's root space.
    pub space: Box<Space>,
    /// The blocks of bodies of this region, by entity.
    pub bodies: FxHashMap<u64, Box<Space>>,
    pub(super) brain_writes: FxHashMap<(u64, u32), Box<[u8]>>,
    pub(super) observations: super::sense::Observations,
    pub(super) gaze: super::gaze::Watches,
    pub(super) senses: super::sense::Environment,
    pub time: u64,
    /// What `/time` moved the time of day by, from `time`.
    pub day_offset: i64,
    pub(crate) weather: crate::weather::WeatherField,
    rng: Rng,
    /// Cubes whose copy on disk holds entities or ticks, written again even if nothing else
    /// changed: what left them must not come back from there.
    pub on_disk: FxHashSet<IVec3>,
    /// Cubes only machines tickets keep (`server::forced`): mobs stand still, nothing grows.
    pub idle: FxHashSet<IVec3>,
    /// Players mods see this tick (not spectators), listed once on first use.
    players: Option<Vec<u64>>,
    pub entities: Entities,
    pub events: Vec<DimEvent>,
    pub(super) next_id: Arc<AtomicU64>,
    pub player_identities: super::PlayerIdentities,
    /// Cubes whose sky light is final: open sky above, or the cube above is settled. Only these
    /// are sent, so clients never see light being filled in afterwards.
    pub settled: FxHashSet<IVec3>,
    /// Bumped whenever a cube becomes settled, so senders know when to look again.
    pub settled_gen: u64,
    /// Cubes settled since the server last looked (to send them to players).
    pub newly_settled: Vec<IVec3>,
    /// Loaded cubes per section, kept as cubes come and go (`sections` for regionizing).
    section_cubes: FxHashMap<IVec3, u32>,
    /// Gizmo shapes mods drew this tick, and the groups some player here shows (set before the
    /// tick, so mods can skip drawing nobody sees).
    pub gizmos: Vec<(abi::Who, crate::net::GizmoShape)>,
    pub gizmo_groups: Arc<[String]>,
    /// Timers set on the dimension this tick (`Host::timer` with no holder), for `DimSet` to keep.
    pub dim_timers: Vec<DimTimer>,
}

fn physics_body<'a>(
    c: &'a Content,
    e: &Entity,
    built: Option<&'a Box<Space>>,
    before: crate::physics::Transform,
) -> Option<crate::physics::Body<'a>> {
    let crate::physics::Dims { half_width: hw, height: h, .. } = e.dims(c);
    let built = built.map(|s| &**s as &dyn crate::physics::Blocks);
    crate::physics::Body::of(e.id, &c.game.entities[e.kind as usize], e.body_place(), (hw, h), before, built)
}

pub(super) struct Cubes<'a>(pub(super) &'a mut FxHashMap<IVec3, Arc<Cube>>);

impl CubeMap for Cubes<'_> {
    fn cube(&self, c: IVec3) -> Option<&Cube> {
        self.0.get(&c).map(|a| &**a)
    }
    fn cube_mut(&mut self, c: IVec3) -> Option<&mut Cube> {
        self.0.get_mut(&c).map(Arc::make_mut)
    }
}

/// What every cube goes through once made (generated, loaded, or on the GPU) before it joins a
/// world, on the server or a client: the area edits made since, then its light on its own as if
/// open sky were above (`light::prelight`; joining the world fixes that up).
pub fn prepare(c: &Content, dim: u32, edits: &[super::area::AreaFill], pos: IVec3, cube: &mut Cube) {
    super::area::catch_up(edits, pos, cube);
    light::prelight(c, cube, c.dims[dim as usize].sky_light);
}

pub fn generate(c: &Content, dim: u32, seed: u64, pos: IVec3) -> Cube {
    let def = &c.dims[dim as usize];
    if pos.y < def.min_cube_y || pos.y > def.max_cube_y {
        return Cube::uniform(AIR, 0);
    }
    let base = crate::generation::base(c, dim, seed, pos);
    finish_generated(c, dim, seed, pos, base.blocks, base.biomes, base.columns, def.builtin.is_some())
}

/// Everything generation does after the terrain (which the GPU may have made): the
/// dimension's generator, then its features (`features::place`). `builtin`: the blocks came from a
/// built-in dimension, which has no generator.
#[allow(clippy::too_many_arguments)]
pub fn finish_generated(
    c: &Content,
    dim: u32,
    seed: u64,
    pos: IVec3,
    mut blocks: Vec<u32>,
    mut biomes: [u16; 256],
    cols: Option<Arc<[abi::TerrainColumn; 256]>>,
    builtin: bool,
) -> Cube {
    let columns = cols.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    if !builtin {
        crate::generation::run_generator(c, dim, seed, pos, &mut blocks, &mut biomes, cols.as_deref());
    }
    crate::generation::validate(c, dim, &mut blocks, "generator");
    let (changed, mut generated) = crate::features::place(c, dim, seed, pos, &mut blocks, &mut biomes, columns);
    if changed {
        crate::generation::validate(c, dim, &mut blocks, "feature");
    }
    let mut cube = if blocks.iter().all(|&b| b == blocks[0]) {
        Cube::uniform(blocks[0], 0)
    } else {
        Cube::from_states(blocks.as_slice().try_into().unwrap())
    };
    if biomes.iter().any(|&b| b != 0) {
        cube.biomes = Some(Box::new(biomes));
    }
    for (i, layout) in generated.layouts.drain() {
        cube.set_layout(i, layout);
    }
    if !generated.extras.is_empty() || !generated.placed.is_empty() {
        cube.generated = Some(Box::new(generated));
    }
    cube
}

use weft_noise::hash;

/// A timer on a dimension: when, the event and its value.
pub type DimTimer = (u64, u32, Box<[u8]>);

/// Appends a timer: fire time, event name and value, as the `TIMERS` component keeps them.
pub fn push_timer(out: &mut Vec<u8>, until: u64, event: &str, data: &[u8]) {
    out.extend_from_slice(&until.to_le_bytes());
    out.extend_from_slice(&(event.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&event.as_bytes()[..event.len().min(u16::MAX as usize)]);
    out.extend_from_slice(&(data.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&data[..data.len().min(u16::MAX as usize)]);
}

pub fn read_timers(mut b: &[u8]) -> Vec<(u64, String, Vec<u8>)> {
    let mut out = Vec::new();
    while let Some((len, t)) = read_timer(b) {
        out.push(t);
        b = &b[len..];
    }
    out
}

/// One timer off the front of `b`, with the bytes it took.
pub fn read_timer(b: &[u8]) -> Option<(usize, (u64, String, Vec<u8>))> {
    let until = u64::from_le_bytes(b.get(..8)?.try_into().ok()?);
    let n = u16::from_le_bytes(b.get(8..10)?.try_into().ok()?) as usize;
    let name = String::from_utf8_lossy(b.get(10..10 + n)?).into_owned();
    let at = 10 + n;
    let m = u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize;
    let data = b.get(at + 2..at + 2 + m)?.to_vec();
    Some((at + 2 + m, (until, name, data)))
}

/// Face connectivity through blocks that do not hide what is behind them.
pub fn see_through(c: &Content, cube: &Cube) -> u16 {
    cube.connectivity(|s| c.states[s as usize].flags & bf::OPAQUE == 0)
}

/// What an entity's move brought on it, applied after the parallel part.
struct TickHit {
    id: u64,
    /// How far it fell before landing, and whether it fell into a liquid: for `fall_effects`.
    fell: Option<f32>,
    splash: Option<DVec3>,
    burn: bool,
    /// Hazards it meets (`content::Hazard`), and the block state they are of (0: none).
    met: Vec<(u32, crate::content::Hazard)>,
}

/// The hazards entity `e` meets, by state and which of its: those in the cells its box is in or
/// a sixteenth from, at its eyes, under its feet. What isn't a body meets only those it is in.
fn hazards_met(c: &Content, block: &impl crate::physics::Cells, e: &super::entity::Entity) -> Vec<(u32, crate::content::Hazard)> {
    let b = e.aabb(c);
    let reach = crate::physics::Aabb { min: b.min - 1.0 / 16.0, max: b.max + 1.0 / 16.0 };
    let mut met = Vec::new();
    block.visit_spaces(reach, |cells, place| {
        hazards_in_space(c, cells, e, place, b, &mut met);
        false
    });
    met.into_iter()
        .map(|(s, i)| (s, c.states[s as usize].hazards[i]))
        .collect()
}

fn hazards_in_space(
    c: &Content,
    block: &dyn crate::physics::Cells,
    e: &super::entity::Entity,
    place: crate::physics::Transform,
    b: crate::physics::Aabb,
    out: &mut Vec<(u32, usize)>,
) {
    use abi::hazard_at as at;
    const GROW: f64 = 1.0 / 16.0;
    let body = e.is_body();
    let look = |p: IVec3, out: &mut Vec<(u32, usize)>, f: &dyn Fn(&crate::content::Hazard, Part) -> bool| {
        let Some((main, extras)) = block.cell(p) else { return };
        for part in extras.parts(main) {
            let s = part.state;
            for (i, h) in c.states[s as usize].hazards.iter().enumerate() {
                if f(h, part) && !out.contains(&(s, i)) {
                    out.push((s, i));
                }
            }
        }
    };
    let overlaps = |part: Part, p: IVec3, grow: f64| {
        part.bounds(c).any(|a| {
            let local = crate::physics::Aabb { min: a.min + p.as_dvec3() - grow, max: a.max + p.as_dvec3() + grow };
            if !crate::physics::contact_box(b, local, place) {
                return false;
            }
            if c.states[part.state as usize].liquid.is_none() {
                return true;
            }
            let world = place.box_to_world(local);
            let contact = (world.min.max(b.min) + world.max.min(b.max)) * 0.5;
            !block.masks_liquid_at(place.to_local(contact))
        })
    };
    let local = place.box_to_local(crate::physics::Aabb { min: b.min - GROW, max: b.max + GROW });
    let (lo, hi) = (local.min.floor().as_ivec3(), local.max.floor().as_ivec3());
    for y in lo.y..=hi.y {
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                let p = IVec3::new(x, y, z);
                look(p, out, &|h, s| match h.at {
                    at::INSIDE => overlaps(s, p, 0.0),
                    at::TOUCH => body && overlaps(s, p, GROW),
                    _ => false,
                });
            }
        }
    }
    if body {
        let eye = place.to_local(e.pos + DVec3::Y * e.eye(c));
        let liquid = block.liquid_block_at(c, eye);
        look(eye.floor().as_ivec3(), out, &|h, s| {
            h.at == at::HEAD && (c.states[s.state as usize].liquid.is_none() || liquid == Some(s.state))
        });
        if e.on_ground {
            look(place.to_local(e.pos - DVec3::Y * 0.05).floor().as_ivec3(), out, &|h, _| h.at == at::ON);
        }
    }
}

impl Dim {
    /// Writes the fields clients show (`registry::HEALTH` ...) into their synced components when
    /// they differ, so they travel like mods' components. Absent ones stay absent while default.
    pub fn mirror_fields(&mut self, c: &'static Content) {
        use crate::net::{HealthField, RidingField, ShapeField, StuckField, UsingField, codec::Wire};
        let crate::registry::FieldComps {
            health,
            burning,
            worn,
            shape,
            held,
            off_held,
            using,
            riding,
            stuck,
            game_mode,
            permission,
            attributes,
            shown,
            ground,
            ..
        } = c.game.fields;
        // What lives has attributes; a dropped item or a block's entity none worth sending. A
        // player's go every tick (its client moves by them), the rest each at its tenth tick.
        let now = self.time;
        let attrs: FxHashMap<u64, Vec<f32>> = self
            .entities
            .list
            .iter()
            .filter(|e| e.player || (e.is_body() && ((now + e.id) % 10 == 0 || !self.entities.comps.contains_key(&(e.id, attributes)))))
            .map(|e| e.id)
            .collect::<Vec<_>>()
            .into_iter()
            .map(|id| {
                (
                    id,
                    (0..c.game.attributes.len() as u32)
                        .map(|a| self.attribute(0, c, id, a, false) as f32)
                        .collect(),
                )
            })
            .collect();
        use crate::registry::attr;
        for (&id, a) in &attrs {
            if let Some(e) = self.entities.get_mut(id) {
                let max = a[attr::MAX_HEALTH as usize];
                if e.health > max {
                    e.health = max;
                }
                if a[attr::BURNING_TIME as usize] == 0.0 {
                    e.fire = 0;
                }
            }
        }
        let Entities { list, comps, inventories, blocks, public_blocks, dirty_inv, .. } = &mut self.entities;
        for &id in public_blocks.iter().filter(|id| dirty_inv.contains(id)) {
            let b = &blocks[&id];
            let slots = &c.game.block_data[&b.kind].shown_slots;
            if !slots.is_empty() {
                let inv = inventories.get(&id);
                let items: Vec<_> = slots
                    .iter()
                    .map(|&s| {
                        inv.and_then(|i| i.get(s as usize))
                            .copied()
                            .unwrap_or_default()
                    })
                    .collect();
                comps.insert((id, shown), crate::stack::shown_bytes(&items).into());
            }
        }
        for e in list.iter().filter(|e| e.in_world()) {
            let put = |comps: &mut FxHashMap<(u64, u32), Box<[u8]>>, comp: u32, bytes: &[u8]| match comps.get_mut(&(e.id, comp)) {
                Some(v) if **v != *bytes => *v = bytes.into(),
                Some(_) => {}
                None if *c.game.components[comp as usize].default != *bytes => {
                    comps.insert((e.id, comp), bytes.into());
                }
                None => {}
            };
            put(comps, ground, &crate::stack::client_bytes(e.item));
            let slots = &c.game.entities[e.kind as usize].shown_slots;
            if !slots.is_empty() {
                let inv = inventories.get(&e.id);
                let items: Vec<ItemStack> = slots
                    .iter()
                    .map(|&s| {
                        inv.and_then(|i| i.get(s as usize))
                            .copied()
                            .unwrap_or_default()
                    })
                    .collect();
                put(comps, shown, &crate::stack::shown_bytes(&items));
            }
            let attrs = attrs.get(&e.id);
            // Between its turns, the maximum it was sent.
            let max_at = crate::registry::attr::MAX_HEALTH as usize;
            let max = attrs.map(|a| a[max_at]).unwrap_or_else(|| {
                comps
                    .get(&(e.id, health))
                    .and_then(|b| HealthField::from_bytes(b))
                    .map_or(0.0, |hp| hp.maximum)
            });
            put(comps, health, &HealthField { current: e.health.max(0.0), maximum: max }.to_bytes());
            put(comps, burning, &[(e.fire > 0) as u8]);
            let inv = inventories.get(&e.id).map_or(&[][..], Vec::as_slice);
            put(comps, worn, &crate::stack::shown_bytes(crate::content::equipped(c, e.kind, inv)));
            let size = e.size.unwrap_or_default();
            put(
                comps,
                shape,
                &ShapeField { width: size.width, height: size.height, eye: size.eye, look: e.look.unwrap_or(u32::MAX) }.to_bytes(),
            );
            put(
                comps,
                riding,
                &e.vehicle
                    .map_or_else(Vec::new, |(mount, seat)| RidingField { mount, seat: seat as u32 }.to_bytes()),
            );
            let held_by = match e.stuck {
                Some(super::entity::Stuck { into: super::entity::StuckIn::Entity(host), at, yaw, .. }) => {
                    StuckField { host, at: at.as_vec3().to_array(), yaw }.to_bytes()
                }
                _ => Vec::new(),
            };
            put(comps, stuck, &held_by);
            if let Some(a) = attrs {
                let bytes = a.to_bytes();
                put(comps, attributes, &bytes);
            }
            for (comp, item) in
                [(held, crate::content::in_main_hand(c, e.kind, inv, e.selected)), (off_held, crate::content::in_off_hand(c, e.kind, inv))]
            {
                put(comps, comp, &crate::stack::client_bytes(item));
            }
            put(comps, using, &UsingField { ticks: e.using.0, offhand: e.using.1 }.to_bytes());
            if e.player {
                put(comps, game_mode, &(e.mode.0 as u32).to_le_bytes());
                put(comps, permission, &e.permission.to_le_bytes());
            }
        }
    }

    pub fn new(id: u32, seed: u64, next_id: Arc<AtomicU64>, player_identities: super::PlayerIdentities) -> Dim {
        Dim {
            id,
            seed,
            space: Box::default(),
            bodies: FxHashMap::default(),
            brain_writes: FxHashMap::default(),
            observations: Default::default(),
            gaze: Default::default(),
            senses: Default::default(),
            time: 1000,
            day_offset: 0,
            weather: Default::default(),
            on_disk: Default::default(),
            idle: Default::default(),
            rng: Rng(weft_noise::mix(seed ^ id as u64)),
            players: None,
            entities: Entities::default(),
            events: Vec::new(),
            next_id,
            player_identities,
            settled: FxHashSet::default(),
            settled_gen: 0,
            newly_settled: Vec::new(),
            section_cubes: FxHashMap::default(),
            gizmos: Vec::new(),
            gizmo_groups: Arc::from([]),
            dim_timers: Vec::new(),
        }
    }

    pub fn has_space(&self, space_id: u64) -> bool {
        space_id == 0 || self.bodies.contains_key(&space_id)
    }

    pub(crate) fn space(&self, space_id: u64) -> &Space {
        if space_id == 0 { &self.space } else { &self.bodies[&space_id] }
    }

    pub(crate) fn space_mut(&mut self, space_id: u64) -> &mut Space {
        self.space_entities(space_id).0
    }

    pub(super) fn space_entities(&mut self, space_id: u64) -> (&mut Space, &mut Entities) {
        let space = if space_id == 0 { &mut self.space } else { self.bodies.get_mut(&space_id).unwrap() };
        (space, &mut self.entities)
    }

    pub fn space_pose(&self, space_id: u64) -> crate::physics::Transform {
        self.entities
            .get(space_id)
            .map_or_else(crate::physics::Transform::default, Entity::body_place)
    }

    /// The block instance in the cell at `p`: inside a callback of a part of it, that part's (as
    /// `seen`); else of its block, else of another in it.
    pub fn anchored(&self, space_id: u64, p: IVec3) -> Option<u64> {
        match self.space(space_id).acting {
            Some((at, part)) if at == p => self.anchored_part(space_id, crate::content::content(), p, part.state),
            _ => self
                .get(space_id, p)
                .and_then(|state| {
                    let c = crate::content::content();
                    self.space(space_id)
                        .layout(p)
                        .residents(state)
                        .find_map(|part| {
                            let kind = c.states[part.state as usize].block;
                            self.entities.anchored_kind(space_id, p, kind, part.offset)
                        })
                })
                .or_else(|| {
                    self.space(space_id)
                        .layout(p)
                        .covers
                        .iter()
                        .find_map(|cover| {
                            let (owner, part) = cover.owner(p);
                            let c = crate::content::content();
                            let kind = c.states[part.state as usize].block;
                            self.entities
                                .anchored_kind(space_id, owner, kind, part.offset)
                        })
                }),
        }
    }

    /// The instance of the block of `state` in the cell at `p`.
    pub fn anchored_part(&self, space_id: u64, c: &Content, p: IVec3, state: u32) -> Option<u64> {
        let kind = c.states[state as usize].block;
        if !c.game.has_block_data(kind) {
            return None;
        }
        self.entities
            .anchored_kind(space_id, p, kind, self.resident(space_id, p, state)?.offset)
    }

    pub fn new_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// The solid body right under feet at `pos` (a step at most), and where that body is.
    pub fn body_under(&self, c: &Content, pos: DVec3, half_width: f64) -> Option<(u64, crate::physics::Transform)> {
        let bodies: Vec<crate::physics::Body> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed && !e.player && c.game.entities[e.kind as usize].is(ef::SOLID))
            .filter_map(|e| physics_body(c, e, self.bodies.get(&e.id), e.body_place()))
            .collect();
        let id = crate::physics::body_below(c, pos, half_width, &bodies, 0.6)?;
        self.entities.get(id).map(|e| (id, e.body_place()))
    }

    fn dry_volume_iter(&self) -> impl Iterator<Item = crate::physics::DryVolume<'_>> {
        self.bodies
            .iter()
            .filter(|(_, b)| !b.dry_boxes.is_empty())
            .filter_map(|(id, b)| {
                let place = self.entities.get(*id)?.body_place();
                Some((place, b.dry_boxes.as_slice()))
            })
    }

    pub(super) fn dry_volumes(&self) -> Vec<crate::physics::DryVolume<'_>> {
        self.dry_volume_iter().collect()
    }

    pub(super) fn contact_bodies<'a>(&'a self, c: &'a Content) -> Vec<crate::physics::Body<'a>> {
        self.bodies
            .iter()
            .filter_map(|(id, built)| {
                let entity = self.entities.get(*id)?;
                physics_body(c, entity, Some(built), entity.body_place())
            })
            .collect()
    }

    pub(crate) fn observed_liquid_at(&self, space_id: u64, c: &Content, pos: DVec3) -> Option<u16> {
        let outside = crate::physics::liquid_at(c, self.space(space_id), pos);
        if space_id != 0 {
            return outside;
        }
        outside
            .filter(|_| {
                !self
                    .dry_volume_iter()
                    .any(|v| crate::physics::inside_dry_volume(v, pos))
            })
            .or_else(|| {
                self.bodies.iter().find_map(|(id, built)| {
                    let entity = self.entities.get(*id)?;
                    crate::physics::body_liquid_at(c, &**built, entity.body_place(), pos)
                })
            })
    }

    pub fn has(&self, pos: IVec3) -> bool {
        self.space.cubes.contains_key(&pos)
    }

    pub fn snapshot(&self, pos: IVec3) -> Option<Arc<Cube>> {
        self.space.cubes.get(&pos).cloned()
    }

    /// Generated terrain and old save files contain the owners. Rebuild their spatial index
    /// as cubes arrive, in either order. A current save already carries matching entries.
    pub(super) fn index_loaded_parts(&mut self, c: &'static Content, cp: IVec3, cube: &mut Cube) {
        let mut pending = self.space.pending_covers.remove(&cp).unwrap_or_default();
        if cube
            .palette()
            .iter()
            .any(|&s| !c.states[s as usize].cells.is_empty())
            || cube.layouts().any(|(_, l)| {
                !l.offsets.is_empty()
                    || l.extras
                        .iter()
                        .any(|&s| !c.states[s as usize].cells.is_empty())
            })
        {
            for i in 0..VOLUME {
                let p = cp * 16 + crate::world::cube::offset(i);
                for part in cube.layout(i).residents(cube.get(i)) {
                    for &delta in part.footprint(c).iter() {
                        let at = p + delta;
                        let cover = part.in_cell(delta);
                        if cube_of(at) == cp {
                            pending.push((at, cover));
                        } else if let Some(neighbor) = self.space.cubes.get(&cube_of(at)) {
                            let mut layout = neighbor.layout(local_index(at)).clone();
                            if !layout.covers.contains(&cover) {
                                layout.covers.push(cover);
                                self.set_layout(0, c, at, layout);
                            }
                        } else {
                            let waiting = self.space.pending_covers.entry(cube_of(at)).or_default();
                            if !waiting.contains(&(at, cover)) {
                                waiting.push((at, cover));
                            }
                        }
                    }
                }
            }
        }
        let valid = |at: IVec3, cover: Part| {
            let (owner, part) = cover.owner(at);
            let source = if cube_of(owner) == cp { Some(&*cube) } else { self.space.cubes.get(&cube_of(owner)).map(Arc::as_ref) };
            source.is_none_or(|source| {
                let i = local_index(owner);
                source.layout(i).residents(source.get(i)).any(|v| v == part)
            })
        };
        pending.retain(|&(at, cover)| valid(at, cover));
        let stale: Vec<_> = cube
            .layouts()
            .filter_map(|(i, layout)| {
                let at = cp * 16 + crate::world::cube::offset(i);
                if layout.covers.iter().all(|&part| valid(at, part)) {
                    return None;
                }
                let mut fixed = layout.clone();
                fixed.covers.retain(|&part| valid(at, part));
                Some((i, fixed))
            })
            .collect();
        for (i, fixed) in stale {
            cube.set_layout(i, fixed);
            self.space.modified.insert(cp);
        }
        for (at, part) in pending {
            let i = local_index(at);
            let mut layout = cube.layout(i).clone();
            if !layout.covers.contains(&part) {
                layout.covers.push(part);
                cube.set_layout(i, layout);
                self.space.modified.insert(cp);
            }
        }
    }

    /// `prelit`: the cube carries light from `light::prelight`.
    pub fn insert(&mut self, c: &'static Content, pos: IVec3, mut cube: Cube, prelit: bool) {
        if self.space.cubes.contains_key(&pos) {
            return;
        }
        self.index_loaded_parts(c, pos, &mut cube);
        self.space.grid.cube_loaded(c, pos, &cube);
        self.space.signals.cube_loaded(c, pos, &cube);
        let def = &c.dims[self.id as usize];
        let open_above = light::open_above(def, pos.y);
        // Clear cubes have a cheaper path of their own in `light_new`.
        let clear = cube
            .palette()
            .iter()
            .all(|&s| c.tight[s as usize].opacity == 0);
        // The worker's light assumed open sky above.
        let prelit = prelit
            && cube.layouts().next().is_none()
            && !clear
            && (!def.sky_light
                || match light::sky_below(self.space.cubes.get(&(pos + IVec3::Y)).map(|a| &**a), open_above) {
                    light::SkyBelow::Full => true,
                    light::SkyBelow::Dark => {
                        cube.mask_light(!0xFF);
                        true
                    }
                    light::SkyBelow::Partial => false,
                });
        // What time acts on continues from when the cube was last simulated: steps due while it
        // was away run in the next tick, in order. Spreading and covered blocks only in cubes
        // that were saved with some: freshly generated terrain stays as generated until
        // something changes near it (a cliff of dirt does not start greening everywhere at once,
        // which kept every new area remeshing).
        let mut lives = Vec::new();
        let saved = cube.ticked != 0;
        let since = if saved { cube.ticked.min(self.time) } else { self.time };
        let wanted = |s: u32| if saved { c.lives(s) } else { c.grows(s) };
        if cube.palette().iter().any(|&s| wanted(s)) {
            lives.extend(
                (0..VOLUME)
                    .filter(|&i| wanted(cube.get(i)))
                    .map(|i| pos * 16 + crate::world::cube::offset(i)),
            );
        }
        for (i, parts) in cube.all_extras() {
            if parts.iter().any(|&s| wanted(s)) {
                lives.push(pos * 16 + crate::world::cube::offset(i));
            }
        }
        self.add_cube(pos, Arc::new(cube));
        for p in lives {
            self.consider(0, c, p, since);
        }
        let mut map = Cubes(&mut self.space.cubes);
        let mut lighter = Lighter::new(&mut map, c, def.sky_light);
        if prelit {
            lighter.light_prelit(pos);
        } else {
            lighter.light_new(pos, open_above);
        }
        let changed = std::mem::take(&mut lighter.changed);
        changed.into_iter().filter(|&p| p != pos).for_each(|p| {
            self.space.light_dirty.insert(p);
        });
        if open_above || !def.sky_light || self.settled.contains(&(pos + IVec3::Y)) {
            self.settle_down(pos);
        }
    }

    /// Drops cubes `keep` rejects; returns the ones that must be saved (modified or holding
    /// entities) together with the entities that lived in them, which leave the world.
    /// The cubes beside the one at `pos` for its distant light (`lod_tops::Around`): loaded, or
    /// as `gone` has them.
    pub fn around(&self, pos: IVec3, gone: impl Fn(IVec3) -> Option<Arc<Cube>>) -> crate::server::lod_tops::Around {
        [IVec3::NEG_X, IVec3::X, IVec3::NEG_Z, IVec3::Z].map(|d| {
            self.space
                .cubes
                .get(&(pos + d))
                .cloned()
                .or_else(|| gone(pos + d))
        })
    }

    pub fn retain_cubes(&mut self, c: &Content, keep: impl Fn(IVec3) -> bool) -> Vec<(IVec3, Arc<Cube>, Vec<u64>)> {
        let mut out = Vec::new();
        let gone: FxHashSet<IVec3> = self
            .space
            .cubes
            .keys()
            .copied()
            .filter(|&p| !keep(p))
            .collect();
        if gone.is_empty() {
            return out;
        }
        // What is on its way lands first, to be with the cube it lands in.
        self.finish_moving(crate::content::content());
        // One pass over entities and growth for all cubes leaving, not one per cube (a crowd of
        // mobs times thousands of cubes).
        let mut inside: FxHashMap<IVec3, Vec<u64>> = FxHashMap::default();
        for e in self.entities.list.iter().filter(|e| e.saved_with_cube()) {
            let cp = cube_of(e.pos.floor().as_ivec3());
            if gone.contains(&cp) {
                inside.entry(cp).or_default().push(e.id);
            }
        }
        let mut grew = false;
        let mut gone: Vec<IVec3> = gone.into_iter().collect();
        gone.sort_unstable_by_key(|p| (p.x, p.y, p.z));
        for pos in gone {
            let mut inside = inside.remove(&pos).unwrap_or_default();
            inside.extend(
                self.entities
                    .blocks_in_cube
                    .get(&pos)
                    .into_iter()
                    .flatten()
                    .copied(),
            );
            let mut cube = self.space.cubes.remove(&pos).unwrap();
            self.cube_gone(pos);
            self.space.grid.cube_unloaded(pos);
            self.space.signals.cube_unloaded(c, pos);
            self.settled.remove(&pos);
            self.space.conn.remove(&pos);
            // Only cubes where something happened are saved; the rest generate again the same.
            // Saved ones keep the time their growing blocks were simulated to, to catch up later
            // (untouched ones start growing again when they come back, like vanilla's).
            let grows = cube.palette().iter().any(|&s| c.lives(s));
            grew |= grows;
            let modified = self.space.modified.remove(&pos);
            let on_disk = self.on_disk.remove(&pos);
            if modified
                || on_disk
                || !inside.is_empty()
                || self
                    .space
                    .scheduled
                    .iter()
                    .any(|t| cube_of(IVec3::from_array(t.0.1)) == pos)
            {
                if grows {
                    Arc::make_mut(&mut cube).ticked = self.time;
                }
                out.push((pos, cube, inside));
            }
        }
        if grew {
            let cubes = &self.space.cubes;
            self.space
                .growth
                .retain(|p, _| cubes.contains_key(&cube_of(*p)));
        }
        out
    }

    /// `None` for what isn't loaded; a body has nothing that isn't, only air where it has no cube.
    pub fn get(&self, space_id: u64, p: IVec3) -> Option<u32> {
        self.space(space_id).get(p)
    }

    pub fn light(&self, space_id: u64, p: IVec3) -> u32 {
        self.space(space_id)
            .cubes
            .get(&cube_of(p))
            .map_or(0, |cube| cube.light(local_index(p)))
    }

    /// 0..1 strength of the sun.
    pub fn daylight(&self, c: &Content) -> f32 {
        self.daylight_at(c, self.time)
    }

    /// `time` by the tick counter: the sky is that plus what `/time` moved it by.
    pub fn daylight_at(&self, c: &Content, time: u64) -> f32 {
        c.dims[self.id as usize].daylight(time.saturating_add_signed(self.day_offset))
    }

    /// Writes a block and updates its light, neighbors, client state and instance data.
    pub fn set(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32) -> Option<u32> {
        self.write_cell(space_id, c, p, state, None, false)
    }

    /// The cell at `p` holds `state` and `extras`; `None`: what else is in the cell stays with the
    /// same block, a property changing, and goes with another block. A carried block keeps its instance data.
    pub(super) fn write_cell(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        state: u32,
        extras: Option<Vec<u32>>,
        carried: bool,
    ) -> Option<u32> {
        let old = self.get(space_id, p)?;
        let layout = self.space(space_id).layout(p);
        let mut parts = if extras.is_none() && state != AIR && c.states[old as usize].block == c.states[state as usize].block {
            let mut parts: Vec<_> = layout.residents(old).collect();
            parts[0].state = state;
            parts
        } else {
            std::iter::once(state)
                .chain(extras.into_iter().flatten())
                .filter(|&s| s != AIR)
                .map(Part::aligned)
                .collect()
        };
        self.write_residents(space_id, c, p, &mut parts, carried)
    }

    /// Checks the complete new footprint before publishing any of its cell records.
    pub(super) fn write_residents(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        parts: &mut [Part],
        carried: bool,
    ) -> Option<u32> {
        if !c.dims[self.id as usize].editable {
            return None;
        }
        let old = self.get(space_id, p)?;
        let mut layout = self.space(space_id).layout(p).clone();
        let before: Vec<_> = layout.residents(old).collect();
        let changes: Vec<_> = before
            .iter()
            .filter(|v| !parts.contains(v))
            .map(|&v| (false, v, v.footprint(c)))
            .chain(
                parts
                    .iter()
                    .filter(|v| !before.contains(v))
                    .map(|&v| (true, v, v.footprint(c))),
            )
            .collect();
        for (add, part, footprint) in &changes {
            for cell in std::iter::once(IVec3::ZERO).chain(footprint.iter().copied()) {
                let at = p + cell;
                let main = self.get(space_id, at)?;
                if *add
                    && self.space(space_id).layout(at).parts(main).any(|other| {
                        other.owner(at).0 != p && !c.may_share(part.state, other.state) && part.in_cell(cell).overlaps(other, c)
                    })
                {
                    return None;
                }
            }
        }
        for (add, part, footprint) in changes {
            for &o in footprint.iter() {
                let at = p + o;
                let mut other = self.space(space_id).layout(at).clone();
                let placed = part.in_cell(o);
                if add {
                    other.covers.push(placed);
                } else {
                    other.covers.retain(|v| *v != placed);
                }
                self.set_layout(space_id, c, at, other);
            }
        }
        let state = layout.set_residents(parts);
        let cp = cube_of(p);
        let def = &c.dims[self.id as usize];
        let space = self.space_mut(space_id);
        if space.finite && state != AIR {
            space.structure_cube(c, cp);
        }
        let cube = Arc::make_mut(space.cubes.get_mut(&cp)?);
        let i = local_index(p);
        let had = cube.layout(i).clone();
        let same = state != AIR && c.states[old as usize].block == c.states[state as usize].block;
        cube.set(i, state);
        cube.set_layout(i, layout.clone());
        if old != state || had != layout {
            space.not_shown.remove(&p);
        }
        if old != state && !same && !space.landings.is_empty() {
            self.invalidate_arrival(space_id, p);
        }
        // Clients lose what else is in a cell with its block, so it comes again after it.
        if layout != had || (old != state && layout != CellLayout::EMPTY) {
            self.layout_changed(space_id, c, p, &had, &layout);
        }
        if !carried && (old != state || layout != had) {
            self.block_data_follow(space_id, c, p, &before, parts);
        }
        if def.block_updates {
            let signals = &mut self.space_mut(space_id).signals;
            if had.offsets.is_empty() && layout.offsets.is_empty() && had.extras.is_empty() && layout.extras.is_empty() {
                if old != state {
                    signals.block_changed(c, p.into(), old, state);
                }
            } else {
                let sites: rustc_hash::FxHashSet<_> = before
                    .iter()
                    .map(|&v| crate::wire::Site::of(c, p, v, old, &had))
                    .chain(
                        parts
                            .iter()
                            .map(|&v| crate::wire::Site::of(c, p, v, state, &layout)),
                    )
                    .collect();
                for site in sites {
                    let at = |main, layout: &CellLayout| {
                        let state = site.state(c, main, layout);
                        if state != AIR && crate::wire::Site::of(c, p, site.part(state), main, layout) == site { state } else { AIR }
                    };
                    let was = at(old, &had);
                    let now = at(state, &layout);
                    if was != now {
                        signals.block_changed(c, site, was, now);
                    }
                }
            }
        }
        if old == state {
            return Some(old);
        }
        let now = self.time;
        let space = self.space_mut(space_id);
        space.changes.push((p, state));
        space.modified.insert(cp);
        space.conn.remove(&cp);
        space.edited.insert(cp);
        if space.finite {
            space.grow_bounds(p, state);
            space.emitters_changed |= c.states[old as usize].light != c.states[state as usize].light;
        }
        space.relight_at(c, def.sky_light, p, Some(old));
        // The new block hears about itself too (like vanilla's onBlockAdded): a liquid starts
        // flowing, a plant without support drops, whoever set it.
        space.queue_cell(p);
        space.grid.block_changed(c, p, old, state);
        let from = space.growing_at.unwrap_or(now);
        // What may now happen with time: here, from now on, to the block it covers or uncovers,
        // and around a block that spreads.
        space.growth.remove(&p);
        self.consider(space_id, c, p, from);
        self.consider(space_id, c, p - IVec3::Y, from);
        if let Some(sp) = c.states[state as usize].spread {
            for dy in sp.min.y..=sp.max.y {
                for dz in sp.min.z..=sp.max.z {
                    for dx in sp.min.x..=sp.max.x {
                        self.consider(space_id, c, p + IVec3::new(dx, dy, dz), from);
                    }
                }
            }
        }
        Some(old)
    }

    /// Resolves a cell address for callers without a selected piece.
    pub fn block_at(&self, space_id: u64, _c: &Content, p: IVec3) -> Option<(IVec3, u32)> {
        let state = self.get(space_id, p)?;
        let layout = self.space(space_id).layout(p);
        Some(
            if state == AIR
                && let Some(part) = layout.covers.first()
            {
                (part.owner(p).0, part.state)
            } else {
                (p, state)
            },
        )
    }

    fn set_layout(&mut self, space_id: u64, c: &'static Content, p: IVec3, layout: CellLayout) {
        let space = self.space_mut(space_id);
        let cp = cube_of(p);
        if space.finite {
            space.structure_cube(c, cp);
        }
        let Some(cube) = space.cubes.get_mut(&cp) else { return };
        let i = local_index(p);
        let old = cube.layout(i).clone();
        Arc::make_mut(cube).set_layout(i, layout.clone());
        if space.finite {
            space.cover_bounds(p, !layout.covers.is_empty() || space.get(p).is_some_and(|s| s != AIR));
        }
        self.layout_changed(space_id, c, p, &old, &layout);
    }

    /// Removes data and drops contents for parts that left the cell. Initializes new parts.
    fn block_data_follow(&mut self, space_id: u64, c: &'static Content, p: IVec3, before: &[Part], after: &[Part]) {
        let same = |a: Part, b: Part| a.offset == b.offset && c.states[a.state as usize].block == c.states[b.state as usize].block;
        for &part in before
            .iter()
            .filter(|&&part| !after.iter().any(|&v| same(part, v)))
        {
            let kind = c.states[part.state as usize].block;
            let Some(id) = self.entities.anchored_kind(space_id, p, kind, part.offset) else {
                continue;
            };
            if let Some(items) = self.entities.inventories.get(&id).cloned() {
                for s in items.into_iter().filter(|s| !s.is_empty()) {
                    self.spawn_item(space_id, c, p.as_dvec3() + DVec3::splat(0.5), s);
                }
            }
            self.entities.remove(id, UNTOLD);
        }
        for &part in after
            .iter()
            .filter(|&&part| !before.iter().any(|&v| same(part, v)))
        {
            let kind = c.states[part.state as usize].block;
            if c.game.has_block_data(kind) {
                self.add_block_data(space_id, c, p, kind, part);
            }
        }
    }

    /// All that is in the cell at `p`, the block `get` gives first. Air has none.
    pub fn parts(&self, space_id: u64, p: IVec3) -> Vec<u32> {
        let Some(main) = self.get(space_id, p).filter(|&s| s != AIR) else { return Vec::new() };
        std::iter::once(main)
            .chain(self.space(space_id).extras(p).iter().copied())
            .collect()
    }

    /// Whether `part` is in the cell at `p`.
    pub fn has_part(&self, space_id: u64, p: IVec3, part: u32) -> bool {
        self.get(space_id, p) == Some(part) || self.space(space_id).extras(p).contains(&part)
    }

    /// What else is in the cell at `p` went from `had` to `extras`: it is saved and sent, lit, and
    /// those around hear of it.
    fn layout_changed(&mut self, space_id: u64, c: &Content, p: IVec3, had: &CellLayout, layout: &CellLayout) {
        let cp = cube_of(p);
        let sky = c.dims[self.id as usize].sky_light;
        let space = self.space_mut(space_id);
        space.extra_changes.push((p, layout.clone()));
        space.modified.insert(cp);
        space.edited.insert(cp);
        if space.finite {
            space.geometry_changed();
        }
        let main = space.get(p).unwrap_or(AIR);
        let old_light = had.light(c, main);
        let new_light = layout.light(c, main);
        if !old_light.same_light(&new_light) {
            space.emitters_changed |= space.finite && old_light.light != new_light.light;
            space.relight_at(c, sky, p, None);
        }
        space.queue_cell(p);
    }

    pub fn resident(&self, space_id: u64, p: IVec3, state: u32) -> Option<Part> {
        let space = self.space(space_id);
        let active = space
            .acting
            .filter(|(at, part)| *at == p && part.state == state);
        active.map(|(_, part)| part).or_else(|| {
            space
                .layout(p)
                .residents(space.get(p)?)
                .find(|part| part.state == state)
        })
    }

    pub fn add_part(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32) -> bool {
        self.add_placed(space_id, c, p, Part::aligned(state))
    }

    pub(super) fn add_placed(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: Part) -> bool {
        if part.state == AIR {
            return false;
        }
        let Some(main) = self.get(space_id, p) else { return false };
        let mut parts: Vec<_> = self.space(space_id).layout(p).residents(main).collect();
        if parts.contains(&part) {
            return false;
        }
        if (c.states[part.state as usize].signal.kind != abi::signal_kind::NONE
            && c.states[main as usize].signal.kind == abi::signal_kind::NONE)
            || c.states[main as usize].liquid.is_some()
        {
            parts.insert(0, part);
        } else {
            parts.push(part);
        }
        self.write_residents(space_id, c, p, &mut parts, false)
            .is_some()
    }

    pub fn replace_part(&mut self, space_id: u64, c: &'static Content, p: IVec3, old: u32, new: u32) -> bool {
        let Some((owner, part)) = self.resident(space_id, p, old).map(|v| (p, v)).or_else(|| {
            self.space(space_id)
                .layout(p)
                .covers
                .iter()
                .find(|v| v.state == old)
                .map(|v| v.owner(p))
        }) else {
            return false;
        };
        self.replace_placed(space_id, c, owner, part, new)
    }

    pub(crate) fn replace_placed(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: Part, state: u32) -> bool {
        let Some(main) = self.get(space_id, p) else { return false };
        let mut parts: Vec<_> = self.space(space_id).layout(p).residents(main).collect();
        let Some(i) = parts.iter().position(|&v| v == part) else { return false };
        let next = Part { state, ..part };
        if state != AIR
            && parts.iter().any(|&other| {
                other != part
                    && (other == next || (!c.may_share(state, other.state) && next.overlaps(other, c) && !part.overlaps(other, c)))
            })
        {
            return false;
        }
        if state == AIR {
            parts.remove(i);
        } else {
            parts[i].state = state;
        }
        if i == 0
            && let Some(n) = parts
                .iter()
                .position(|v| c.states[v.state as usize].liquid.is_none())
        {
            parts.swap(0, n);
        }
        self.write_residents(space_id, c, p, &mut parts, false)
            .is_some()
    }

    /// A callback of `part` of the cell at `p` is running: what it gets and sets there is that
    /// part, as if it were alone. When it is, there is nothing to keep apart.
    fn as_part<R>(&mut self, space_id: u64, p: IVec3, part: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(part) = self.resident(space_id, p, part) else { return f(self) };
        self.as_placed(space_id, p, part, f)
    }

    pub(super) fn as_placed<R>(&mut self, space_id: u64, p: IVec3, part: Part, f: impl FnOnce(&mut Self) -> R) -> R {
        let was = self.space_mut(space_id).acting.replace((p, part));
        let r = f(self);
        self.space_mut(space_id).acting = was;
        r
    }

    /// `get` for a mod: inside a callback of a part of a cell, that part.
    pub fn seen(&self, space_id: u64, p: IVec3) -> Option<u32> {
        match self.space(space_id).acting {
            Some((at, part)) if at == p => Some(part.state),
            // A big block's other cell is the block, to a mod as to a player.
            _ => self
                .block_at(space_id, crate::content::content(), p)
                .map(|b| b.1),
        }
    }

    /// `set` for a mod: inside a callback of a part of a cell, of that part.
    pub fn set_seen(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32) -> Option<u32> {
        match self.space(space_id).acting {
            Some((at, part)) if at == p => {
                self.replace_placed(space_id, c, p, part, state)
                    .then_some(())?;
                self.space_mut(space_id).acting = Some((p, Part { state, ..part }));
                Some(part.state)
            }
            _ if self.get(space_id, p)? == AIR
                && let Some(cover) = self.space(space_id).layout(p).covers.first().copied() =>
            {
                let (owner, part) = cover.owner(p);
                self.replace_placed(space_id, c, owner, part, state)
                    .then_some(part.state)
            }
            _ => self.set(space_id, c, p, state),
        }
    }

    /// The share of the cell's room its shapes take.
    fn fill_of(c: &Content, state: u32) -> f32 {
        c.states[state as usize]
            .selection
            .iter()
            .map(|b| (b[3] - b[0]) * (b[4] - b[1]) * (b[5] - b[2]))
            .sum()
    }

    /// Whether the source `liquid` may be in the cell at `p` along with what is in it: all of it
    /// may share a cell with that liquid (`shares_cell`), and leaves it room.
    pub(super) fn can_hold_liquid(&self, space_id: u64, c: &Content, p: IVec3, liquid: u32) -> bool {
        let parts = self.parts(space_id, p);
        let solids = parts
            .iter()
            .filter(|&&s| c.states[s as usize].liquid.is_none());
        solids.clone().all(|&s| c.may_share(s, liquid)) && solids.map(|&s| Self::fill_of(c, s)).sum::<f32>() < 0.999
    }

    /// A liquid goes from a cell that can't hold it (any more), or that it just flowed through.
    fn drain_if_no_room(&mut self, space_id: u64, c: &'static Content, p: IVec3) {
        let Some(liquid) = self
            .parts(space_id, p)
            .into_iter()
            .find(|&s| c.states[s as usize].liquid.is_some())
        else {
            return;
        };
        let source = c.liquids[c.states[liquid as usize].liquid.unwrap_or(0) as usize].first_state == liquid;
        if self.parts(space_id, p).len() > 1 && !(source && self.can_hold_liquid(space_id, c, p, liquid)) {
            self.replace_part(space_id, c, p, liquid, AIR);
        }
    }

    fn add_block_data(&mut self, frame: u64, c: &Content, cell: IVec3, kind: u32, part: Part) {
        let id = self.new_id();
        self.entities
            .insert_block(c, id, super::entity::BlockData { kind, frame, cell, offset: part.offset }, None, Vec::new());
    }

    pub(crate) fn block_kind(&self, id: u64) -> Option<u32> {
        self.entities.blocks.get(&id).map(|block| block.kind)
    }

    pub(crate) fn block_part(&self, id: u64, c: &Content) -> Option<(super::entity::BlockData, Part)> {
        let block = *self.entities.blocks.get(&id)?;
        if !self.has_space(block.frame) {
            return None;
        }
        let part = self
            .space(block.frame)
            .layout(block.cell)
            .residents(self.get(block.frame, block.cell)?)
            .find(|part| part.offset == block.offset && c.states[part.state as usize].block == block.kind)?;
        Some((block, part))
    }

    pub(super) fn block_position(&self, id: u64) -> Option<DVec3> {
        let block = self.entities.blocks.get(&id)?;
        Some(self.space_pose(block.frame).to_world(block.center()))
    }

    pub fn block_data_fits(&self, c: &Content, block: super::entity::BlockData) -> bool {
        self.has_space(block.frame)
            && self
                .entities
                .anchored_kind(block.frame, block.cell, block.kind, block.offset)
                .is_none()
            && self
                .space(block.frame)
                .layout(block.cell)
                .residents(self.get(block.frame, block.cell).unwrap_or(AIR))
                .any(|part| part.offset == block.offset && c.states[part.state as usize].block == block.kind)
    }

    /// The same for all of a body, once the ones saved with it are in.
    pub fn add_missing_on_body(&mut self, c: &Content, id: u64) {
        let cubes: Vec<IVec3> = self
            .bodies
            .get(&id)
            .map_or(Vec::new(), |s| s.cubes.keys().copied().collect());
        for cp in cubes {
            self.add_missing_block_data(id, c, cp);
        }
    }

    /// Initializes missing instance data in generated cubes or cubes saved before the block declared it.
    pub fn add_missing_block_data(&mut self, space_id: u64, c: &Content, pos: IVec3) {
        let Some(cube) = self.space(space_id).cubes.get(&pos).cloned() else { return };
        let has = |s: u32| c.game.has_block_data(c.states[s as usize].block);
        let mains = cube.palette().iter().any(|&s| has(s));
        let parts = cube
            .all_extras()
            .filter(|(_, parts)| parts.iter().any(|&s| has(s)));
        let cells = (0..if mains { VOLUME } else { 0 }).chain(parts.filter(|_| !mains).map(|(i, _)| i));
        for i in cells {
            for part in cube.layout(i).residents(cube.get(i)) {
                let kind = c.states[part.state as usize].block;
                if !c.game.has_block_data(kind) {
                    continue;
                }
                let p = pos * 16 + crate::world::cube::offset(i);
                if self
                    .entities
                    .anchored_kind(space_id, p, kind, part.offset)
                    .is_none()
                {
                    self.add_block_data(space_id, c, p, kind, part);
                }
            }
        }
    }

    pub fn spawn(&mut self, space_id: u64, c: &'static Content, kind: u32, pos: DVec3) -> u64 {
        let id = self.new_id();
        let k = &c.game.entities[kind as usize];
        if let Some(blocks) = super::structure::template(c, kind) {
            self.bodies
                .insert(id, Box::new(Space::with_blocks(c, blocks)));
        }
        let mut e = Entity::new(id, kind, self.space_pose(space_id).to_world(pos), 0.0);
        // Solid bodies collide as axis-aligned boxes, so they start square to the world.
        if !k.is(ef::SOLID) {
            e.yaw = (self.rng.float() * std::f32::consts::TAU) - std::f32::consts::PI;
        }
        self.entities.insert(c, e);
        self.add_missing_on_body(c, id);
        let max = self.max_health(space_id, c, id);
        if let Some(e) = self.entities.get_mut(id) {
            e.health = max;
        }
        id
    }

    /// The block at `p` starts falling (sand, gravel, anvils): a moving structure of that one
    /// block, drawn and colliding like any. It is placed again where it lands, or drops what it
    /// drops if it cannot be.
    pub fn fall_block(&mut self, space_id: u64, c: &'static Content, p: IVec3) -> u64 {
        let Some(part) = self
            .space(space_id)
            .acting
            .filter(|(at, _)| *at == p)
            .map(|(_, part)| part)
            .or_else(|| {
                self.space(space_id)
                    .layout(p)
                    .residents(self.get(space_id, p)?)
                    .next()
            })
        else {
            return 0;
        };
        let block =
            super::structure::Lifted { from: p, local: IVec3::ZERO, to: p, part: Some(Part::aligned(part.state)), only: Some(part) };
        // The falling entity's collision box is centered on its block, including gridless placements.
        let at = p.as_dvec3() + crate::physics::Transform::FEET + part.shift();
        self.lift(space_id, c, c.game.falling_kind, at, [block], &[], super::structure::Liveness::Free)
            .0
    }

    /// Body `id` falls under its weight from now on and lands as blocks (`land_falling`).
    pub fn body_fall(&mut self, id: u64) {
        if !self.bodies.contains_key(&id) {
            return;
        }
        if let Some(e) = self
            .entities
            .get_mut(id)
            .filter(|e| e.motion != Motion::Parked)
        {
            e.completion = Completion::LandOnSupport;
            e.motion = Motion::Free;
        }
    }

    /// Falling bodies that came down: each block placed where it landed, on what it landed on,
    /// else dropped as loot. One resting on another that is still falling (layers of sand) rides
    /// it down and lands once that one has; what it stood on being gone, it stands on the world.
    fn land_falling(&mut self, c: &'static Content) {
        let (bodies, entities) = (&self.bodies, &self.entities);
        let falling = |id: u64| {
            entities
                .get(id)
                .is_some_and(|e| e.completion == Completion::LandOnSupport && !e.removed)
        };
        let landed: Vec<(u64, DVec3, Vec<(IVec3, u32)>, bool, Option<u64>)> = self
            .entities
            .list
            .iter()
            .filter(|e| e.completion == Completion::LandOnSupport && !e.removed && e.motion != Motion::Parked)
            .filter(|e| {
                (e.on_ground && !e.frame.is_some_and(falling)) || e.age > 600 || bodies.get(&e.id).is_none_or(|s| s.all_blocks().is_empty())
            })
            .map(|e| {
                let on = e.frame.filter(|f| bodies.contains_key(f));
                (e.id, e.pos, bodies.get(&e.id).map_or(Vec::new(), |s| s.all_blocks()), e.on_ground, on)
            })
            .collect();
        for (id, pos, blocks, ground, on) in landed {
            // One without its blocks (loaded from a save mid-fall) just goes.
            if blocks.is_empty() {
                self.entities.remove(id, UNTOLD);
                continue;
            }
            let space_id = on.filter(|&frame| self.active_space(frame)).unwrap_or(0);
            let cell = crate::physics::Transform::of(self.space_pose(space_id).to_local(pos)).cell(None);
            let mut down = Vec::new();
            let mut broke = Vec::new();
            for (local, state) in blocks {
                let p = cell + local;
                if ground {
                    down.push(super::structure::Carried { local, to: p, state });
                } else {
                    broke.extend(self.space(id).layout(local).residents(state).map(|part| {
                        let entity = self
                            .entities
                            .anchored_kind(id, local, c.states[part.state as usize].block, part.offset);
                        (p, part.state, entity)
                    }));
                }
            }
            if down.is_empty() {
                self.entities.remove(id, UNTOLD);
            } else {
                self.put_down(space_id, c, id, down, Vec::new());
            }
            for (p, state, entity) in broke {
                self.broke_landing(space_id, c, p, state, entity);
            }
        }
    }

    /// How far a player's actions on blocks reach, with room for what the client saw last.
    pub fn block_reach(&mut self, space_id: u64, c: &'static Content, player: u64) -> f64 {
        self.attribute(space_id, c, player, crate::registry::attr::BLOCK_REACH, false) + super::BLOCK_SLACK
    }

    /// Velocity in the world, including the body carrying it. Seated riders already have their
    /// mount's velocity in `vel`; its frame is only the platform underneath that mount.
    pub fn world_velocity(&self, id: u64) -> DVec3 {
        let Some(e) = self.entities.get(id) else { return DVec3::ZERO };
        e.vel
            + e.frame
                .and_then(|f| self.entities.get(f))
                .map_or(DVec3::ZERO, |b| b.vel)
    }

    /// `stack` thrown by `player`: from just below its eyes where it looks, not to be picked up
    /// for two seconds.
    pub fn throw(&mut self, space_id: u64, c: &'static Content, player: u64, stack: ItemStack) -> u64 {
        let Some(e) = self.entities.get(player) else { return 0 };
        let (from, look) = (e.drop_point(c), crate::physics::look_dir(e.yaw, e.pitch));
        let id = self.spawn_item(space_id, c, from, stack);
        if let Some(item) = self.entities.get_mut(id) {
            item.vel = (look + DVec3::Y * 0.2) * 6.0;
            item.pickup_delay = 40;
        }
        id
    }

    /// A dropped stack, asked about and told of as any new entity (`spawn_cause::DROP`): its id,
    /// 0 if refused.
    pub fn spawn_item(&mut self, space_id: u64, c: &'static Content, pos: DVec3, stack: ItemStack) -> u64 {
        // A liquid by the millibucket spills away instead: it only lives in liquid slots.
        if stack.is_empty() || c.item_fluid.contains_key(&stack.item) {
            return 0;
        }
        let id = self.new_id();
        let mut e = Entity::new(id, c.game.item_kind, self.space_pose(space_id).to_world(pos), 1.0);
        e.item = stack;
        e.pickup_delay = 10;
        let (a, b) = (self.rng.float() - 0.5, self.rng.float() - 0.5);
        e.vel = DVec3::new(a as f64 * 2.0, 3.0, b as f64 * 2.0);
        self.entities.insert(c, e);
        self.announce(space_id, c, id, abi::spawn_cause::DROP)
    }

    /// Calls a mod callback with this dimension as its world.
    pub fn call<A>(&mut self, space_id: u64, owner: abi::ModId, cb: abi::Callback, make: impl FnOnce(*mut abi::WorldCtx) -> A) -> A {
        let mut ctx = abi::WorldCtx { region: (self as *mut Dim).cast(), frame: space_id };
        let mut a = make(&mut ctx);
        host::invoke(owner, cb, &mut a);
        a
    }

    fn call_block<A>(
        &mut self,
        space_id: u64,
        c: &'static Content,
        state: u32,
        pick: impl Fn(&crate::content::BlockCallbacks) -> Option<(abi::ModId, abi::Callback)>,
        args: impl FnOnce(*mut abi::WorldCtx) -> A,
    ) -> Option<A> {
        let (owner, cb) = pick(&c.block_of(state).cb)?;
        Some(self.call(space_id, owner, cb, args))
    }

    /// A melee hit of `attacker` on `target`, the same for players and mobs: the `ATTACK`
    /// question, its `ATTACK_DAMAGE` against the target through `chain::MELEE` (how charged its
    /// swing is, whether it crits), the damage, extra knockback (`ATTACK_KNOCKBACK`), a sweep.
    /// The part of `target`'s hitbox `who` looks at (`abi::hitbox_part`), 0 if none: where a blow
    /// lands.
    fn part_looked_at(&self, c: &Content, who: u64, target: u64) -> u32 {
        let (Some(me), Some(t)) = (self.entities.get(who), self.entities.get(target)) else { return 0 };
        let eye = me.pos + DVec3::Y * me.dims(c).eye;
        let ((sy, cy), (sp, cp)) = ((me.yaw as f64).sin_cos(), (me.pitch as f64).sin_cos());
        let look = DVec3::new(-sy * cp, sp, -cy * cp);
        let parts = c.game.entities[t.kind as usize].hitboxes.as_ref();
        crate::hitbox::ray_entity(parts, &t.hit_body(c, t.pos), eye, look, 16.0, &mut Default::default()).map_or(0, |h| h.0)
    }

    pub fn attack(&mut self, space_id: u64, c: &'static Content, attacker: u64, target: u64, held: ItemStack) -> bool {
        let (Some(_), Some(t)) = (self.entities.get(attacker), self.entities.get(target)) else { return false };
        if target == attacker || !t.is_body() {
            return false;
        }
        let mut q = abi::Query { entity: attacker, other: target, item: held.item, ..abi::Query::DEFAULT };
        if !self.ask(space_id, c, abi::question::ATTACK, &q) {
            return false;
        }
        // It may have changed what it holds since the last tick looked.
        if let Some(me) = self.entities.get_mut(attacker) {
            me.notice_hand(held);
        }
        q.charge = self.swing_strength(space_id, c, attacker, held);
        q.part = self.part_looked_at(c, attacker, target);
        q.crit = self.ask_or(space_id, c, abi::question::CRIT, &q, false) as u32;
        let base = self.attribute_for(space_id, c, crate::registry::attr::ATTACK_DAMAGE, &q, false);
        let dmg = (self.compute(space_id, c, abi::chain::MELEE, base, &q, false) as f32).max(0.0);
        self.events
            .push(DimEvent::EntitySwing { id: attacker, off: false });
        let speed = self.attribute(space_id, c, attacker, crate::registry::attr::ATTACK_SPEED, false);
        let Some(me) = self.entities.get_mut(attacker) else { return false };
        me.melee_recovery = crate::content::melee_recovery(c, me.kind, &held, speed as f32);
        me.mining = None;
        me.swing_ticks = 0;
        let pos = me.pos;
        let hit = self.damage(
            space_id,
            c,
            target,
            &abi::Damage {
                amount: dmg,
                cause: damage::ATTACK,
                source: attacker,
                item: held,
                crit: q.crit,
                part: q.part,
                ..abi::Damage::DEFAULT
            },
        );
        if !hit {
            return false;
        }
        // Asked before the knock below ends a sprint.
        let sweeps = self.ask_or(space_id, c, abi::question::SWEEP, &q, false);
        let extra = self.attribute_for(space_id, c, crate::registry::attr::ATTACK_KNOCKBACK, &q, false);
        if extra > 0.0 {
            let away = (self.entities.get(target).map_or(pos, |t| t.pos) - pos)
                .with_y(0.0)
                .normalize_or_zero();
            self.knock(space_id, c, target, away * extra * 0.5);
            // A knocking hit ends a sprint, as vanilla's.
            if let Some(me) = self.entities.get_mut(attacker).filter(|e| e.sprinting) {
                me.sprinting = false;
                self.events.push(DimEvent::StopSprint { player: attacker });
            }
        }
        if sweeps {
            let sweep = self.compute(space_id, c, abi::chain::SWEEP_DAMAGE, dmg as f64, &q, false) as f32;
            let center = self.entities.get(target).map_or(pos, |t| t.pos);
            let near: Vec<u64> = self
                .entities
                .list
                .iter()
                .filter(|e| {
                    e.is_body()
                        && e.id != attacker
                        && e.id != target
                        && (e.pos - center).length_squared() <= 2.0
                        && (e.pos - pos).length_squared() <= 9.0
                })
                .map(|e| e.id)
                .collect();
            for id in near {
                self.damage(
                    space_id,
                    c,
                    id,
                    &abi::Damage { amount: sweep, cause: damage::ATTACK, source: attacker, item: held, ..abi::Damage::DEFAULT },
                );
            }
        }
        true
    }

    /// Vanilla's knockback by `push` (flat; its length is the strength), less the target's
    /// `KNOCKBACK_RESISTANCE`; a player's client applies its own.
    pub fn knock(&mut self, space_id: u64, c: &'static Content, id: u64, push: DVec3) {
        let resist = self.attribute(space_id, c, id, crate::registry::attr::KNOCKBACK_RESISTANCE, false);
        let Some(e) = self.entities.get_mut(id) else { return };
        if e.player {
            self.entities
                .told
                .push((id, Told::Knock(push.as_vec3().to_array())));
        } else {
            e.vel = crate::physics::knock(e.vel * crate::world::TICK, push, e.on_ground, resist) * crate::world::TPS as f64;
        }
    }

    /// How charged an entity's next swing is, 0..1.
    pub fn swing_strength(&mut self, space_id: u64, c: &'static Content, id: u64, held: ItemStack) -> f32 {
        let Some(e) = self.entities.get(id) else { return 1.0 };
        crate::content::swing_charge(
            c,
            e.kind,
            &held,
            e.swing_ticks as f32,
            self.attribute(space_id, c, id, crate::registry::attr::ATTACK_SPEED, false) as f32,
        )
    }

    /// Effect `effect` on `id` at `level` for `ticks`, replacing the one it had (0 ticks ends it,
    /// `INFINITE_TICKS` never does): asked first (`action::EFFECT`) and told after, an end told
    /// as `EFFECT_ENDED`. An instant one acts once, its hazards at that level, and doesn't stay.
    pub fn give_effect(&mut self, space_id: u64, c: &'static Content, id: u64, effect: u32, level: u32, ticks: u32) {
        let Some(def) = c.game.entity_effects.get(effect as usize) else { return };
        if self.entities.get(id).is_none() {
            return;
        }
        let level = level.max(1);
        let comp = c.game.fields.effect;
        let had: Vec<_> = crate::world::effects_on(self.entities.comps.get(&(id, comp)).map(|b| &b[..]), self.time).collect();
        let change = abi::EffectChange { entity: id, effect, level, ticks, pad: 0 };
        if ticks > 0 {
            let q = abi::Query { entity: id, cause: effect, amount: level as f64, state: ticks, ..abi::Query::DEFAULT };
            if !self.ask(space_id, c, abi::action::EFFECT, &q) {
                return;
            }
            self.emit_engine(space_id, c, abi::action::EFFECT, &change);
        }
        if def.instant {
            for h in def.hazards_at(level) {
                if let Some(cause) = h.cause {
                    self.damage(space_id, c, id, &abi::Damage::new(h.damage, cause));
                }
            }
            return;
        }
        let mut list: Vec<_> = had.iter().copied().filter(|f| f.id != effect).collect();
        if ticks > 0 {
            let until = if ticks == abi::INFINITE_TICKS { crate::world::FOREVER } else { self.time + ticks as u64 };
            list.push(crate::world::OnEffect { id: effect, level, until });
        } else if let Some(f) = had.iter().find(|f| f.id == effect) {
            self.emit_engine(space_id, c, abi::event::EFFECT_ENDED, &abi::EffectChange { level: f.level, ticks: 0, ..change });
        }
        self.entities
            .comps
            .insert((id, comp), crate::world::encode_effects(&list));
        self.entities.dirty_comp.insert((id, comp));
    }

    /// Effects that ran out go (`EFFECT_ENDED`), then each effect with a tick of its own gets every
    /// entity that has it, once.
    fn tick_effects(&mut self, space_id: u64, c: &'static Content) {
        let comp = c.game.fields.effect;
        let mut ended = Vec::new();
        let mut ticking: FxHashMap<u32, (Vec<u64>, Vec<u32>)> = FxHashMap::default();
        let (entities, now) = (&mut self.entities, self.time);
        for id in entities.list.iter().filter(|e| !e.removed).map(|e| e.id) {
            let Some(bytes) = entities.comps.get_mut(&(id, comp)) else { continue };
            for f in crate::world::effects_on(Some(&bytes[..]), now) {
                if c.game.entity_effects[f.id as usize].on_tick.is_some() {
                    let t = ticking.entry(f.id).or_default();
                    t.0.push(id);
                    t.1.push(f.level);
                }
            }
            if crate::world::effects_on(Some(&bytes[..]), u64::MAX).any(|f| f.until <= now) {
                let (on, off): (Vec<_>, Vec<_>) = crate::world::effects_on(Some(&bytes[..]), u64::MAX).partition(|f| f.until > now);
                *bytes = crate::world::encode_effects(&on);
                ended.push((id, off));
            }
        }
        for (id, off) in ended {
            self.entities.dirty_comp.insert((id, comp));
            for f in off {
                self.emit_engine(
                    space_id,
                    c,
                    abi::event::EFFECT_ENDED,
                    &abi::EffectChange { entity: id, effect: f.id, level: f.level, ticks: 0, pad: 0 },
                );
            }
        }
        let mut effects: Vec<u32> = ticking.keys().copied().collect();
        effects.sort();
        for effect in effects {
            let (ids, levels) = &ticking[&effect];
            let Some((owner, cb)) = c.game.entity_effects[effect as usize].on_tick else { continue };
            let (ids, levels, count) = (ids.as_ptr(), levels.as_ptr(), ids.len());
            self.call(space_id, owner, cb, |w| abi::EffectTickArgs { world: w, effect, ids, levels, count });
        }
    }

    /// The damage a fall of `fell` blocks does to `id` (`chain::FALL_DAMAGE`), if any.
    pub fn fall_damage(&mut self, space_id: u64, c: &'static Content, id: u64, fell: f32) -> Option<f32> {
        self.entities.get(id)?;
        let q = abi::Query { entity: id, amount: fell as f64, ..abi::Query::DEFAULT };
        Some(self.compute(space_id, c, abi::chain::FALL_DAMAGE, fell as f64, &q, false) as f32).filter(|&d| d > 0.0)
    }

    /// What everyone near sees of a fall: dust where entity `id` landed from `fell` blocks up
    /// (vanilla's, from a fall that would hurt), and bubbles in the liquid it fell into.
    pub fn fall_effects(&mut self, space_id: u64, c: &'static Content, id: u64, fell: Option<f32>, splash: Option<DVec3>) {
        let safe = self.attribute(space_id, c, id, crate::registry::attr::SAFE_FALL, false) as f32;
        let hurts = fell
            .and_then(|f| self.fall_damage(space_id, c, id, f))
            .is_some();
        let Some(e) = self.entities.get(id) else { return };
        let (pos, w) = (e.pos, e.dims(c).half_width as f32);
        let mut bursts = Vec::new();
        let under = self
            .get(space_id, (pos - DVec3::Y * 0.2).floor().as_ivec3())
            .unwrap_or(AIR);
        let land = c.states[under as usize].effects[crate::content::block_event::LAND];

        if fell.is_some() && under != AIR {
            self.sound(space_id, land.sound, pos, 0.5, 0.0);
        }

        let kind = land.particle;
        if let Some(drop) = fell.filter(|_| hurts && under != AIR && kind != u32::MAX) {
            // Vanilla's `checkFallDamage`, and what its particles make of no speed of their own.
            let count = (150.0 * (0.2 + (drop - safe).ceil() / 15.0).min(2.5)) as u32;
            bursts.push(abi::ParticleBurst {
                kind,
                block: under,
                pos: pos.to_array().into(),
                count,
                velocity: [0.0, 2.0, 0.0].into(),
                speed: 3.6,
                outward: 1,
                ..abi::ParticleBurst::DEFAULT
            });
        }
        if let Some(from) = splash
            && let Some((liquid, surface)) = crate::physics::liquid_surface(c, self.space(space_id), pos.floor().as_ivec3())
        {
            let dy = pos.y - from.y;
            let center = if dy < 0.0 { from.lerp(pos, ((surface - from.y) / dy).clamp(0.0, 1.0)) } else { pos };
            let state = c.liquids[liquid as usize].first_state;
            for (particle, y, velocity) in [(abi::comp::SPLASH, surface, 3.0), (abi::comp::BUBBLE, pos.y, 1.2)] {
                if let Some(kind) = c
                    .components
                    .id(particle)
                    .and_then(|id| c.components.text(comps::BLOCK, state, id))
                    .and_then(|n| c.game.catalog.find(ek::PARTICLE, n))
                {
                    bursts.push(abi::ParticleBurst {
                        kind,
                        pos: center.with_y(y).to_array().into(),
                        count: 12,
                        spread: [w, 0.0, w].into(),
                        speed: 3.0,
                        velocity: [0.0, velocity, 0.0].into(),
                        ..abi::ParticleBurst::DEFAULT
                    });
                }
            }
        }
        let dim = self.id;
        self.events.extend(
            bursts
                .into_iter()
                .map(|burst| DimEvent::Particles { dim, who: abi::Who::Around, burst }),
        );
    }

    /// Damage to an entity, or with a negative `amount` healing: one entry for every change of
    /// health that mods may watch or change. The `HURT` question (`core:immune` first), whether
    /// it is still recovering (`question::RECOVERING`), the `DAMAGE` chain, knockback
    /// (`chain::KNOCKBACK`), then the `HURT` event.
    pub fn damage(&mut self, space_id: u64, c: &'static Content, id: u64, d: &abi::Damage) -> bool {
        matches!(self.damage_outcome(space_id, c, id, d), super::guard::DamageOutcome::Hurt)
    }

    pub(super) fn damage_outcome(&mut self, space_id: u64, c: &'static Content, id: u64, d: &abi::Damage) -> super::guard::DamageOutcome {
        let (amount, source, cause) = (d.amount, d.source, d.cause);
        let Some(e) = self.entities.get(id) else { return super::guard::DamageOutcome::Missed };
        let k = &c.game.entities[e.kind as usize];
        if !e.alive() || amount.is_nan() {
            return super::guard::DamageOutcome::Missed;
        }
        if amount < 0.0 {
            return if self.heal(space_id, c, id, -amount, source, cause) {
                super::guard::DamageOutcome::Hurt
            } else {
                super::guard::DamageOutcome::Missed
            };
        }
        let Some(def) = c.game.damage_causes.get(cause as usize) else {
            crate::host::misuse("damage", || format!("no damage cause {cause}"));
            return super::guard::DamageOutcome::Missed;
        };
        let q = abi::Query {
            entity: id,
            other: source,
            direct: d.direct,
            item: d.item.item,
            cause,
            amount: amount as f64,
            crit: d.crit,
            part: d.part,
            ..abi::Query::DEFAULT
        };
        if !self.ask(space_id, c, abi::action::HURT, &q) {
            return super::guard::DamageOutcome::Missed;
        }
        if let Some(slot) = self.block_damage(c, id, d) {
            return super::guard::DamageOutcome::Blocked(slot);
        }
        let recovering = self.ask_or(space_id, c, abi::question::RECOVERING, &q, false);
        let explain = crate::log::on(crate::log::Channel::Modifiers, log::Level::Debug);
        let amount = self.compute(space_id, c, abi::chain::DAMAGE, amount as f64, &q, explain) as f32;
        if explain {
            log::debug!(target: "modifiers", "{}", super::extend::explanation());
        }
        // While recovering only what goes beyond the last hit lands, and it doesn't knock.
        let last = self.entities.get(id).map_or(0.0, |e| e.last_hurt);
        let landing = if recovering { amount - last } else { amount };
        if landing <= 0.0 {
            return super::guard::DamageOutcome::Missed;
        }
        let amount = landing;
        let direct = if d.direct != 0 { d.direct } else { source };
        if self.entities.get(id).is_some_and(|e| e.brain.is_some())
            && let Some(comp) = c.components.id(abi::brain::HURT)
        {
            let memory = abi::BrainMemory { source, time: self.time.saturating_add(1) };
            let bytes = abi::data::bytes_of(&memory);
            super::holder::set_component(self, c, super::holder::At::Entity(id), comp, Some(&bytes));
        }
        self.emit_engine(
            space_id,
            c,
            abi::action::HURT,
            &abi::Hurt { entity: id, source, direct: d.direct, amount, cause, crit: d.crit, part: d.part },
        );
        let from = self.entities.get(direct).map(|s| s.pos);
        let knock = if recovering || def.has(df::NO_KNOCKBACK) {
            None
        } else if d.knock != abi::Vec3::ZERO {
            Some(
                DVec3::from_array(d.knock.to_array())
                    .with_y(0.0)
                    .normalize_or_zero(),
            )
        } else {
            from.and_then(|from| {
                self.entities.get(id).map(|e| {
                    (e.pos - from)
                        .with_y(0.0)
                        .try_normalize()
                        .unwrap_or(DVec3::X)
                })
            })
        };
        // A death mods may refuse (a totem): it lives on with one health.
        let dying = self
            .entities
            .get(id)
            .is_some_and(|e| e.health - amount <= 0.0);
        let spared = dying && {
            let q = abi::Query { amount: amount as f64, ..q };
            !self.ask(space_id, c, abi::action::DIE, &q)
        };
        let cooldown = self.attribute(space_id, c, id, crate::registry::attr::HURT_COOLDOWN, false) as u32;
        if let Some(way) = knock {
            let strength = self.compute(space_id, c, abi::chain::KNOCKBACK, 0.4, &q, false);
            self.knock(space_id, c, id, way * strength);
        }
        let Some(e) = self.entities.get_mut(id) else { return super::guard::DamageOutcome::Missed };
        e.health = if spared { 1.0 } else { (e.health - amount).max(0.0) };
        e.last_hurt = if recovering { last + amount } else { amount };
        if !recovering {
            e.hurt_cooldown = cooldown;
        }
        let (health, player, pos, kind, on_fire, height) = (e.health, e.player, e.pos, e.kind, e.fire > 0, e.height(c));
        if d.crit != 0 {
            let half = e.dims(c).half_width;
            let toward_hit = from.map_or(DVec3::ZERO, |at| (at - pos).with_y(0.0).normalize_or_zero());
            if let Some(kind) = c.game.catalog.find(ek::PARTICLE, abi::particle::CRIT) {
                self.events.push(DimEvent::Particles {
                    dim: self.id,
                    who: abi::Who::Around,
                    burst: abi::ParticleBurst {
                        kind,
                        pos: (pos + DVec3::Y * (height * 0.55) + toward_hit * half)
                            .to_array()
                            .into(),
                        count: 16,
                        spread: [half, height * 0.5, half].into(),
                        speed: 3.0,
                        outward: 1,
                        ..abi::ParticleBurst::DEFAULT
                    },
                });
            }
        }
        let effects = k.effects;
        use crate::registry::entity_event;
        if health <= 0.0 || !def.has(df::SILENT) {
            self.sound(
                space_id,
                effects[if health <= 0.0 { entity_event::DEATH } else { entity_event::HURT }].sound,
                pos + DVec3::Y * (height * 0.5),
                1.0,
                0.0,
            );
        }
        if health <= 0.0 {
            self.entities.killed.push(id);
        }
        if health <= 0.0 {
            self.emit_engine(
                space_id,
                c,
                abi::action::DIE,
                &abi::Death { entity: id, source: source, kind, cause, pos: pos.to_array().into() },
            );
            let name = self
                .entities
                .comps
                .get(&(id, c.game.name_comp))
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .filter(|n| !n.is_empty());
            if player || name.is_some() {
                use crate::ui::text::{Arg, Name};
                let killer = self
                    .player_identities
                    .read()
                    .get(&source)
                    .map(|p| Arg::Text(p.name.clone()))
                    .or_else(|| {
                        let source = self.entities.get(source)?;
                        let kind = c.game.entities.get(source.kind as usize)?;
                        Some(Arg::Name(Name::Entity(kind.name.clone())))
                    });
                self.events
                    .push(DimEvent::Died { entity: id, cause, killer, name });
            }
            // A player's inventory is the server's to drop (`core:keep_inventory`), after it closes
            // what it had open; everything else's falls here with its loot.
            if !player {
                let by_player = self.entities.get(source).is_some_and(|s| s.player);
                let tool = self.entities.held(source);
                let ctx = crate::loot::Ctx { tool, by_player, on_fire: on_fire || def.has(df::BURNING), ..Default::default() };
                for s in self.roll_loot(c, &k.name, &ctx, pos, source) {
                    self.spawn_item(space_id, c, pos + DVec3::Y * 0.5, s);
                }
                self.drop_inventory(space_id, c, id);
                self.entities.remove(id, abi::removal::DIED);
            }
        }
        super::guard::DamageOutcome::Hurt
    }

    /// `amount` more health up to the most it has, through the `HEAL` chain; the dead stay dead.
    /// No hurt pause, knockback, sound or armor: none of damage's consequences.
    fn heal(&mut self, space_id: u64, c: &'static Content, id: u64, amount: f32, source: u64, cause: u32) -> bool {
        let q = abi::Query { entity: id, other: source, cause, amount: amount as f64, ..abi::Query::DEFAULT };
        let amount = self.compute(space_id, c, abi::chain::HEAL, amount as f64, &q, false) as f32;
        let max = self.max_health(space_id, c, id);
        let Some(e) = self.entities.get_mut(id) else { return false };
        let gained = (e.health + amount).min(max) - e.health;
        if gained <= 0.0 || !gained.is_finite() {
            return false;
        }
        e.health += gained;
        self.emit_engine(
            space_id,
            c,
            abi::event::ENTITY_HEALED,
            &abi::Hurt { entity: id, source, direct: 0, amount: gained, cause, crit: 0, part: 0 },
        );
        true
    }

    /// What a dead player carried falls where it died.
    pub fn drop_inventory(&mut self, space_id: u64, c: &'static Content, id: u64) {
        let Some(items) = self.entities.inventories.get(&id).cloned() else { return };
        let pos = self
            .entities
            .get(id)
            .map(|e| e.pos)
            .or_else(|| self.block_position(id))
            .map_or(DVec3::ZERO, |pos| self.space_pose(space_id).to_local(pos));
        for (i, s) in items.into_iter().enumerate() {
            if !s.is_empty() {
                let item = self.spawn_item(space_id, c, pos + DVec3::Y, s);
                // Vanilla's: flung out in a random direction, and not to be grabbed back at once.
                let (a, f) =
                    (self.random() as f64 / u64::MAX as f64 * std::f64::consts::TAU, self.random() as f64 / u64::MAX as f64 * 10.0);
                if let Some(e) = self.entities.get_mut(item) {
                    e.vel = DVec3::new(a.cos() * f, 4.0, a.sin() * f);
                    e.pickup_delay = 40;
                }
                self.entities.set_slot(id, i as u32, ItemStack::EMPTY);
            }
        }
    }

    pub(super) fn tick(
        &mut self,
        c: &'static Content,
        tick_rate: u32,
        players: Option<&super::spawn::Players>,
        budget: Option<&std::sync::atomic::AtomicU64>,
    ) {
        self.time += 1;
        // Whatever changes from here on changes in this tick, before signals have their turn.
        self.space.signals.now = self.time;
        self.players = None;
        self.tick_blocks_first(0, c);
        self.tick_effects(0, c);
        self.source_modifiers(c);
        if let Some(players) = players {
            self.natural_spawns(c, players, budget);
        }
        let children = self.mount_children();
        let order = self.mount_order(&children);
        let before = self
            .bodies
            .keys()
            .filter_map(|&id| Some((id, self.entities.get(id)?.body_place())))
            .collect();
        let landing = self.advance_mounts(c, &children, &order);
        self.advance_grabs(c, &children);
        self.advance_ships(c, &before, &children);
        self.sample_gaze(c, tick_rate);
        self.tick_entities(c, &before);
        for id in landing {
            self.land_mount(c, id);
        }
        self.tick_bodies(c);
        self.tick_blocks_after(0, c);
        self.compact(0, c);
        self.deliver_events(0, c);
    }

    /// Drops what was removed, telling mods (`event::ENTITY_REMOVED`) what went and why.
    pub fn compact(&mut self, space_id: u64, c: &'static Content) {
        let gone: Vec<abi::Removed> = self
            .entities
            .list
            .iter()
            .filter(|e| e.removed && e.removal != UNTOLD)
            .map(|e| abi::Removed {
                entity: e.id,
                kind: e.kind,
                reason: e.removal,
                pos: self.space_pose(space_id).to_local(e.pos).to_array().into(),
            })
            .collect();
        for r in &gone {
            self.emit_engine(space_id, c, abi::event::ENTITY_REMOVED, r);
        }
        if !self.observations.is_empty() && self.entities.list.iter().any(|e| e.removed) {
            self.clean_observations();
        }
        self.entities.compact();
    }

    /// `spawn`, set up by `init` (its owner, speed), then asked (`action::SPAWN`, `cause` a
    /// `spawn_cause`) and told after: 0 when refused.
    pub fn spawn_by(
        &mut self,
        space_id: u64,
        c: &'static Content,
        kind: u32,
        pos: DVec3,
        cause: u32,
        init: impl FnOnce(&mut Entity),
    ) -> u64 {
        let id = self.spawn(space_id, c, kind, pos);
        let Some(e) = self.entities.get_mut(id) else { return 0 };
        init(e);
        self.announce(space_id, c, id, cause)
    }

    /// A new entity asked about (`action::SPAWN`) and told of: its id, 0 if refused.
    fn announce(&mut self, space_id: u64, c: &'static Content, id: u64, cause: u32) -> u64 {
        let Some(e) = self.entities.get(id) else { return 0 };
        let (kind, at) = (e.kind, self.space_pose(space_id).to_local(e.pos));
        let q = abi::Query { entity: id, cause, pos: abi::Vec3::from(at.to_array()).block(), ..abi::Query::DEFAULT };
        if !self.ask(space_id, c, abi::action::SPAWN, &q) {
            self.entities.remove(id, UNTOLD);
            return 0;
        }
        self.emit_engine(space_id, c, abi::action::SPAWN, &abi::Spawned { entity: id, kind, cause, pos: at.to_array().into() });
        id
    }

    fn tick_blocks_first(&mut self, space_id: u64, c: &'static Content) {
        let updates = c.dims[self.id as usize].block_updates;
        if !updates {
            self.space_mut(space_id)
                .scheduled
                .retain(|t| t.0.2 != c.game.block_tick);
        }

        while self
            .space(space_id)
            .scheduled
            .peek()
            .is_some_and(|t| t.0.0 <= self.time)
        {
            let Some(Reverse(due)) = self.space_mut(space_id).scheduled.pop() else { break };
            // The same timer set several times for one tick fires once.
            while self
                .space(space_id)
                .scheduled
                .peek()
                .is_some_and(|t| t.0 == due)
            {
                self.space_mut(space_id).scheduled.pop();
            }
            let (_, at, event, data) = due;
            if event != c.game.block_tick {
                self.emit(space_id, c, event, &data);
                continue;
            }
            let p = IVec3::from_array(at);
            let pos: Vec<u8> = at.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.emit(space_id, c, event, &pos);
            let wanted = data
                .get(12..)
                .and_then(|name| c.state_by_name(std::str::from_utf8(name).ok()?))
                .map(|state| {
                    let offset = IVec3::from_array(std::array::from_fn(|i| i32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap())));
                    (state, offset)
                });
            if !data.is_empty() && wanted.is_none() {
                continue;
            }
            let parts: Vec<_> = self
                .space(space_id)
                .layout(p)
                .residents(self.get(space_id, p).unwrap_or(AIR))
                .collect();
            for part in parts {
                let state = part.state;
                if wanted.is_some_and(|(s, offset)| {
                    offset != part.offset
                        || c.states[s as usize].block != c.states[state as usize].block
                        || c.states[s as usize].turn != c.states[state as usize].turn
                }) || !self.has_placed(space_id, p, part)
                {
                    continue;
                }
                self.as_placed(space_id, p, part, |d| {
                    if c.states[state as usize].liquid.is_some() {
                        d.liquid_tick(space_id, c, p, state);
                    } else if !d.neighbor_rules(space_id, c, p, state, true) {
                        d.call_block(
                            space_id,
                            c,
                            state,
                            |b| b.on_scheduled,
                            |w| abi::BlockEventArgs { world: w, pos: p.to_array().into(), state, other: [0; 3].into(), entity: 0 },
                        );
                    }
                });
            }
        }

        if updates {
            self.grow(space_id, c);
        }
    }

    fn tick_blocks_after(&mut self, space_id: u64, c: &'static Content) {
        let (space, entities) = self.space_entities(space_id);
        space.grid.tick(entities, space_id, c, None);
        if c.dims[self.id as usize].block_updates {
            self.land_moving(space_id, c);
            self.signal_tick(space_id, c);
        }
        self.flush_neighbors(space_id, c);
    }

    /// Bodies' blocks live like the world's. Those whose entity is gone go with it, and what
    /// belonged to them.
    fn tick_bodies(&mut self, c: &'static Content) {
        let entities = &self.entities;
        self.bodies.retain(|id, _| entities.get(*id).is_some());
        let bodies = &self.bodies;
        let orphans: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| {
                e.structure_parent()
                    .is_some_and(|f| !bodies.contains_key(&f))
            })
            .map(|e| e.id)
            .collect();
        if !orphans.is_empty() {
            for id in self.entities.structure_tree(&orphans) {
                self.entities.remove(id, UNTOLD);
                self.bodies.remove(&id);
            }
        }
        for b in self.bodies.values_mut() {
            b.signals.now = self.time;
        }
        self.cross_cells(c);
        let mut ids: Vec<u64> = self.bodies.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if self.active_space(id) {
                self.tick_blocks_first(id, c);
                self.tick_blocks_after(id, c);
                self.deliver_events(id, c);
            }
        }
        self.exchange_all_signals(c);
        self.forget_gone_bodies(c);
        self.drive_bodies(c);
    }

    /// Players mods see (spectators left out, so mobs neither chase nor spawn around them).
    pub fn visible_players(&mut self) -> &[u64] {
        let list = &self.entities.list;
        self.players.get_or_insert_with(|| {
            list.iter()
                .filter(|e| e.player && !e.removed && !e.mode.has(crate::tag_bits::game_mode::HIDDEN))
                .map(|e| e.id)
                .collect()
        })
    }

    /// Starts or retargets `id`'s walk to `target`. A target close to the current one only
    /// moves it (the path follows at the next look); for a new one the path is looked for in
    /// `navigation`, after the mods' tick, together with every other walk started this tick.
    pub fn navigate(&mut self, id: u64, target: DVec3, speed: f32, profile: Option<super::path::Profile>) -> bool {
        let Some(e) = self.entities.get_mut(id) else { return false };
        if speed <= 0.0 {
            e.nav = None;
            return false;
        }
        if !target.is_finite() || !speed.is_finite() {
            return false;
        }
        let age = e.age;
        // Walkers standing on a body choose a route in its frame.
        let swim = profile.as_ref().is_some_and(|p| p.swim);
        let frame = e.frame.filter(|f| !swim && self.bodies.contains_key(f));
        let place = frame
            .and_then(|f| self.entities.get(f))
            .map(Entity::body_place);
        let target = place.map_or(target, |t| t.to_local(target));
        let Some(e) = self.entities.get_mut(id) else { return false };
        if let Some(nav) = e
            .nav
            .as_mut()
            .filter(|n| n.frame == frame && n.profile == profile && (swim || (n.target - target).length_squared() < 1.0))
        {
            (nav.target, nav.speed) = (target, speed);
            return true;
        }
        e.nav = Some(super::path::Nav {
            target,
            speed,
            profile,
            end: None,
            path: Vec::new(),
            next: 1,
            repath: age,
            found_for: None,
            stuck: 0,
            frame,
        });
        true
    }

    /// Entities walking somewhere by themselves: a new path when the target moved off or the walk
    /// got stuck (at most once a second), steering towards the next cell, done at the target or
    /// after two seconds stuck. Steering a mod gave this
    /// tick (`walk`) wins.
    fn navigation(&mut self, c: &Content) {
        let cubes = &self.space.cubes;
        let block = super::space::CubeCells(cubes);
        let walks = |e: &Entity| e.nav.is_some() && !e.removed && !e.player && e.walk.is_none() && e.facing.is_none();
        // On a body its path is on that, in its coordinates.
        let bodies = &self.bodies;
        let places: FxHashMap<u64, crate::physics::Transform> = self
            .entities
            .list
            .iter()
            .filter(|e| bodies.contains_key(&e.id))
            .map(|e| (e.id, e.body_place()))
            .collect();
        let on_body = |e: &Entity| {
            e.nav
                .as_ref()
                .and_then(|n| n.frame)
                .and_then(|f| places.get(&f).map(|t| (f, *t)))
        };
        let list = &mut self.entities.list;
        let due: Vec<usize> = (0..list.len())
            .filter(|&i| {
                walks(&list[i])
                    && list[i]
                        .nav
                        .as_ref()
                        .is_some_and(|n| n.stale(list[i].age, list[i].pos))
            })
            .take(PATHS_PER_TICK)
            .collect();
        let found: Vec<(Option<DVec3>, Vec<IVec3>)> = due
            .par_iter()
            .map(|&i| {
                let (e, nav) = (&list[i], list[i].nav.as_ref().unwrap());
                let dims = e.dims(c);
                // It goes down no further than it falls without harm.
                let drop = (e.safe_fall.floor() as i32).max(1);
                if let Some(profile) = nav.profile.as_ref().filter(|p| p.swim) {
                    return super::path::swim(c, &block, e.pos, nav.target, dims, profile);
                }
                match on_body(e) {
                    Some((f, place)) => {
                        let deck = &bodies[&f];
                        super::path::walk(c, deck.as_ref(), place.to_local(e.pos), nav.target, dims, drop, nav.profile.as_ref())
                    }
                    None => super::path::walk(c, &block, e.pos, nav.target, dims, drop, nav.profile.as_ref()),
                }
            })
            .collect();
        for (&i, (end, path)) in due.iter().zip(found) {
            let e = &mut list[i];
            let nav = e.nav.as_mut().unwrap();
            nav.path = path;
            nav.end = end;
            let direct = end.is_some() && nav.path.is_empty();
            let delay = if direct {
                4
            } else if nav.swimming() && end.is_none() && nav.path.len() <= 1 {
                40
            } else {
                20
            };
            if nav.swimming() {
                log::trace!(target: "navigation", "{} swim route: direct={direct}, nodes={}", e.id, nav.path.len());
            }
            (nav.next, nav.repath, nav.found_for, nav.stuck) = (1, e.age + delay, Some(nav.target), 0);
        }
        let now = self.time;
        for e in list.iter_mut().filter(|e| walks(e)) {
            let finish = |e: &mut Entity| {
                e.nav = None;
                if let Some(brain) = &mut e.brain {
                    brain.wake_controls(c, abi::brain::MOVE, now);
                }
            };
            let frame = e.nav.as_ref().and_then(|n| n.frame);
            let place = frame.and_then(|f| places.get(&f).copied());
            // Its body went.
            if frame.is_some() && place.is_none() {
                finish(e);
                continue;
            }
            let pos = place.map_or(e.pos, |t| t.to_local(e.pos));
            let dims = e.dims(c);
            let nav = e.nav.as_mut().unwrap();
            let swimming = nav.swimming();
            let blocked = e.blocked || e.moved < 0.0001;
            nav.stuck = if blocked { nav.stuck + 1 } else { 0 };
            if swimming {
                let mut next = nav.end;
                while let Some(&cell) = nav.path.get(nav.next) {
                    let point = super::path::swim_point(cell, dims);
                    if (point - pos).length_squared() >= 0.01 {
                        next = Some(point);
                        break;
                    }
                    nav.next += 1;
                }
                if nav.swim_arrived(pos) {
                    e.walk = Some(super::entity::Walk { swim_velocity: Some(DVec3::ZERO), ..Default::default() });
                    finish(e);
                } else if let Some(next) = next.filter(|p| (*p - pos).length_squared() >= 0.0016) {
                    let delta = next - pos;
                    let top = c.game.entities[e.kind as usize].speed as f64 * nav.speed as f64 * e.moves.swim * crate::world::TICK as f64;
                    let velocity = delta.clamp_length_max(top.max(0.0));
                    if !e
                        .brain
                        .as_ref()
                        .is_some_and(|b| b.controls(abi::brain::LOOK))
                    {
                        let horizontal = delta.x.hypot(delta.z);
                        if horizontal > 0.01 {
                            e.yaw = (-delta.x).atan2(-delta.z) as f32;
                        }
                        e.pitch = delta.y.atan2(horizontal) as f32;
                    }
                    e.flying = false;
                    e.walk = Some(super::entity::Walk { swim_velocity: Some(velocity), ..Default::default() });
                } else {
                    nav.found_for = None;
                }
                continue;
            }
            let approximate = nav.end.is_none()
                && nav.found_for.is_some()
                && (nav.target - pos).with_y(0.0).length_squared() < 1.0
                && (nav.target.y - pos.y).abs() < 1.5;
            let precise = nav.end == Some(nav.target)
                && (nav.target - pos).with_y(0.0).length_squared() < 0.06_f64.powi(2)
                && (nav.target.y - pos.y).abs() < 0.15;
            match super::path::steer(nav, pos, dims, e.blocked) {
                _ if precise || nav.stuck > 40 => finish(e),
                None if approximate => finish(e),
                Some((delta, jump, terminal)) if delta.length_squared() > 1e-8 => {
                    let dir = delta.normalize();
                    let dir = [dir.x as f32, dir.z as f32];
                    let dir = place.map_or(dir, |t| {
                        let v = t.vector_to_world(DVec3::new(dir[0] as f64, 0.0, dir[1] as f64));
                        [v.x as f32, v.z as f32]
                    });
                    if !e
                        .brain
                        .as_ref()
                        .is_some_and(|b| b.controls(abi::brain::LOOK))
                        && e.facing.is_none()
                    {
                        e.yaw = (-dir[0]).atan2(-dir[1]);
                    }
                    let speed = if terminal {
                        let stride = c.game.entities[e.kind as usize].speed as f64 * crate::world::TICK;
                        if stride > 0.0 { nav.speed.min((delta.length() / stride) as f32) } else { nav.speed }
                    } else {
                        nav.speed
                    };
                    e.walk = Some(super::entity::Walk { dir, speed, keys: if jump { abi::walk::JUMP } else { 0 }, swim_velocity: None });
                }
                // The end of a path that stopped short: look again.
                _ => nav.found_for = None,
            }
        }
    }

    /// Natural spawning (`abi::comp::spawn`) around this region's players every two seconds.
    fn natural_spawns(&mut self, c: &'static Content, nearby: &super::spawn::Players, budget: Option<&std::sync::atomic::AtomicU64>) {
        use std::sync::atomic::Ordering::Relaxed;
        let cats = &c.game.categories;
        let players: Vec<DVec3> = self
            .entities
            .list
            .iter()
            .filter(|e| e.player && !e.removed && e.alive())
            .map(|e| e.pos)
            .collect();
        for p in players {
            let mut count = vec![0u32; cats.len()];
            for e in self
                .entities
                .list
                .iter()
                .filter(|e| e.is_body() && !e.player)
            {
                let cat = c.game.entities[e.kind as usize].category as usize;
                if (e.pos - p).length_squared() < cats[cat].cap_radius * cats[cat].cap_radius {
                    count[cat] += 1;
                }
            }
            for _ in 0..4 {
                let a = self.rng.float() as f64 * std::f64::consts::TAU;
                let r = super::spawn::MIN_DISTANCE + self.rng.float() as f64 * 21.0;
                let (x, z) = ((p.x + a.cos() * r).floor() as i32, (p.z + a.sin() * r).floor() as i32);
                // Per rule, the first spot of its kind with room for its mob, scanning down from
                // above the player. Unloaded counts as solid; liquids are not solid.
                let state = |y: i32| {
                    self.get(0, IVec3::new(x, y, z))
                        .map(|s| &c.states[s as usize])
                };
                let solid = |y: i32| state(y).is_none_or(|s| s.solid());
                let air = |y: i32| !solid(y) && state(y).is_some_and(|s| s.liquid.is_none());
                let wet = |y: i32| state(y).is_some_and(|s| s.liquid.is_some());
                let top = p.y.floor() as i32 + 16;
                let spot_for = |s: &crate::registry::SpawnRule| -> Option<(i32, IVec3)> {
                    use abi::spawn_place as sp;
                    let h = s.height;
                    let (y, rests_on) = match s.place {
                        sp::IN_LIQUID => {
                            let y = (top - 32..top)
                                .rev()
                                .find(|&y| (0..h).all(|k| wet(y - k)))?;
                            (y - h + 1, IVec3::new(x, y - h + 1, z))
                        }
                        sp::CEILING => {
                            let y = (top - 32..top)
                                .rev()
                                .find(|&y| solid(y + 1) && (0..h).all(|k| air(y - k)))?;
                            (y - h + 1, IVec3::new(x, y + 1, z))
                        }
                        _ => {
                            let y = (top - 32..top)
                                .rev()
                                .find(|&y| solid(y - 1) && air(y) && (1..h.max(2)).all(|k| !solid(y + k)))?;
                            (y, IVec3::new(x, y - 1, z))
                        }
                    };
                    Some((y, rests_on))
                };
                let biome_at = |spot: IVec3| {
                    self.space
                        .cubes
                        .get(&cube_of(spot))
                        .map_or(0, |cube| cube.biome((x & 15) as usize, (z & 15) as usize)) as u32
                };
                let fits = |s: &crate::registry::SpawnRule| -> Option<(i32, u32)> {
                    if count[s.category as usize] >= cats[s.category as usize].cap || !s.dims.is_empty() && !s.dims.contains(&self.id) {
                        return None;
                    }
                    let (y, rests_on) = spot_for(s)?;
                    let spot = IVec3::new(x, y, z);
                    ((s.y[0]..=s.y[1]).contains(&y)
                        && s.biome
                            .as_ref()
                            .is_none_or(|b| crate::refs::has(b, biome_at(spot))))
                    .then(|| (y, self.get(0, rests_on).unwrap_or(AIR)))
                };
                let spots: Vec<Option<(i32, u32)>> = c.game.spawns.iter().map(fits).collect();
                // Asked about the spot and what it rests on.
                let found: Vec<Option<i32>> = c
                    .game
                    .spawns
                    .iter()
                    .zip(spots)
                    .map(|(s, at)| {
                        let (y, ground) = at?;
                        let pos = DVec3::new(x as f64 + 0.5, y as f64, z as f64 + 0.5);
                        let ask = crate::conditions::Ask { pos, state: ground, ..Default::default() };
                        s.when
                            .is_none_or(|w| crate::conditions::on_server(self, 0, c, w, &ask))
                            .then_some(y)
                    })
                    .collect();
                let total: u32 = c
                    .game
                    .spawns
                    .iter()
                    .zip(&found)
                    .filter(|(_, y)| y.is_some())
                    .map(|(s, _)| s.weight)
                    .sum();
                if total == 0 {
                    continue;
                }
                let mut pick = (self.rng.next() % total as u64) as u32;
                let Some((rule, y)) = c
                    .game
                    .spawns
                    .iter()
                    .zip(found)
                    .filter_map(|(s, y)| y.map(|y| (s, y)))
                    .find(|(s, _)| {
                        let hit = pick < s.weight;
                        pick = pick.saturating_sub(s.weight);
                        hit
                    })
                else {
                    continue;
                };
                if self.rng.float() >= rule.chance {
                    continue;
                }
                let n = rule.group[0] + (self.rng.next() % (rule.group[1] - rule.group[0] + 1) as u64) as u32;
                let at = DVec3::new(x as f64 + 0.5, y as f64, z as f64 + 0.5);
                if !nearby.allows(at) {
                    continue;
                }
                for _ in 0..n {
                    if count[rule.category as usize] >= cats[rule.category as usize].cap
                        || budget.is_some_and(|b| {
                            b.try_update(Relaxed, Relaxed, |n| n.checked_sub(1))
                                .is_err()
                        })
                    {
                        break;
                    }
                    if self.spawn_by(0, c, rule.entity, at, abi::spawn_cause::NATURAL, |_| {}) != 0 {
                        count[rule.category as usize] += 1;
                    } else if let Some(budget) = budget {
                        budget.fetch_add(1, Relaxed);
                    }
                }
            }
        }
    }

    /// Timers (`Host::timer`) of this region's entities that are due fire their events.
    fn fire_timers(&mut self, c: &'static Content) {
        let comp = c.game.timers_comp;
        let ids: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed)
            .map(|e| e.id)
            .filter(|id| self.entities.comps.contains_key(&(*id, comp)))
            .collect();
        for id in ids {
            let Some(bytes) = self.entities.comps.get(&(id, comp)) else { continue };
            let mut due = Vec::new();
            let mut kept = Vec::new();
            for (until, name, data) in read_timers(bytes) {
                if until <= self.time {
                    due.push((name, data));
                } else {
                    push_timer(&mut kept, until, &name, &data);
                }
            }
            if due.is_empty() {
                continue;
            }
            if kept.is_empty() {
                self.entities.comps.remove(&(id, comp));
            } else {
                self.entities
                    .comps
                    .insert((id, comp), kept.into_boxed_slice());
            }
            let frame = self
                .entities
                .get(id)
                .map(|_| self.entities.owner_frame(id))
                .filter(|&frame| self.active_space(frame))
                .unwrap_or(0);
            for (name, data) in due {
                match c.game.event_names.get(&name) {
                    Some(&e) => self.emit(frame, c, e, &data),
                    None => log::warn!("entity {id}: a timer for event {name}, which no mod has now"),
                }
            }
        }
    }

    fn tick_entities(&mut self, c: &'static Content, before_turn: &FxHashMap<u64, crate::physics::Transform>) {
        // Mod logic: one call per kind and callback with every entity of that kind.
        let mut by_kind: FxHashMap<(u64, u32), Vec<u64>> = FxHashMap::default();
        // Copied, mods' calls borrow the region; empty but where only machines are kept.
        let idle = (!self.idle.is_empty()).then(|| self.idle.clone());
        let still = |e: &super::entity::Entity| {
            idle.as_ref()
                .is_some_and(|i| !e.player && i.contains(&cube_of(e.pos.floor().as_ivec3())))
        };
        for e in &self.entities.list {
            if !e.removed && (!e.player || e.alive()) && !still(e) {
                if !c.game.entities[e.kind as usize].on_tick.is_empty() {
                    by_kind.entry((0, e.kind)).or_default().push(e.id);
                }
            }
        }
        let mut kinds: Vec<(u64, u32)> = by_kind.keys().copied().collect();
        kinds.sort();
        for (frame, kind) in kinds {
            if !self.active_space(frame) {
                continue;
            }
            let ids = &by_kind[&(frame, kind)];
            for &(owner, cb) in &c.game.entities[kind as usize].on_tick {
                let ids_ptr = ids.as_ptr();
                let count = ids.len();
                self.call(frame, owner, cb, |w| abi::EntityTickArgs { world: w, kind, ids: ids_ptr, count });
            }
        }
        self.run_systems(c);
        for &(owner, cb) in &c.game.world_ticks {
            self.call(0, owner, cb, |w| abi::WorldTickArgs { world: w });
        }
        self.fire_timers(c);
        self.observe_hands(c);
        self.flush_brain_writes(c);
        let mut ready: Vec<_> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed && e.alive() && !still(e) && self.brain_due(e))
            .map(|e| (e.kind, e.id))
            .collect();
        if ready.len() > super::brain::DECISIONS_PER_TICK {
            ready.sort_unstable_by_key(|&(_, id)| (self.brain_deadline(id), id));
            ready.truncate(super::brain::DECISIONS_PER_TICK);
        }
        self.tick_brains(c, &ready);
        self.navigation(c);
        self.steer_mounts(c);
        self.choose_poses(c);

        // What physics goes by of each body's attributes, a player's on its client.
        let bodies: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed && !e.player && e.item.is_empty())
            .map(|e| e.id)
            .collect();
        for id in bodies {
            let moves = crate::physics::Moves::of(|a| self.attribute(0, c, id, a, false));
            let safe = self.attribute(0, c, id, crate::registry::attr::SAFE_FALL, false) as f32;
            if let Some(e) = self.entities.get_mut(id) {
                (e.moves, e.safe_fall) = (moves, safe);
            }
        }
        let (here, now, gravity) = (self.id, self.time, c.dims[self.id as usize].gravity as f64);
        let players: Vec<DVec3> = self
            .entities
            .list
            .iter()
            .filter(|e| e.player && !e.removed)
            .map(|e| e.pos)
            .collect();
        let cubes = &self.space.cubes;
        let block = super::space::CubeCells(cubes);
        // Solid supports move before the bodies that ride them.
        let mut bodies: Vec<crate::physics::Body> = Vec::new();
        let body_blocks = &self.bodies;
        let mut moving: Vec<_> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed && !e.player && c.game.entities[e.kind as usize].is(ef::SOLID))
            .map(|e| e.id)
            .collect();
        if !body_blocks.is_empty() {
            let mut remaining: FxHashSet<_> = moving.iter().copied().collect();
            let mut order = Vec::with_capacity(moving.len());
            let mut branch = Vec::new();
            for id in moving {
                let mut parent = Some(id);
                while let Some(id) = parent.filter(|id| remaining.remove(id)) {
                    branch.push(id);
                    parent = self.entities.get(id).and_then(|e| e.frame);
                }
                order.extend(branch.drain(..).rev());
            }
            moving = order;
        }
        for id in moving {
            if self
                .entities
                .get(id)
                .is_some_and(|e| e.motion != Motion::Parked && e.mount.is_some())
            {
                self.entities.update_mount_pose(id);
            }
            let Some(e) = self.entities.get_mut(id) else { continue };
            let before = before_turn
                .get(&e.id)
                .copied()
                .unwrap_or_else(|| e.body_place());
            step(c, e, gravity, &block, &bodies);
            bodies.extend(physics_body(c, e, body_blocks.get(&e.id), before));
        }
        let bodies = &bodies;
        let dry: Vec<_> = bodies
            .iter()
            .filter_map(|b| {
                let boxes = &body_blocks.get(&b.id)?.dry_boxes;
                (!boxes.is_empty()).then_some((b.blocks?.1, boxes.as_slice()))
            })
            .collect();
        let block = crate::physics::ObservedCells { cells: block, dry: &dry, bodies };
        // Physics only reads blocks and bodies and writes its own entity, so entities move in
        // parallel; damage is applied afterwards in list order to stay deterministic.
        let inventories = &self.entities.inventories;
        let comps = &self.entities.comps;
        let hits: Vec<TickHit> = self
            .entities
            .list
            .par_iter_mut()
            .with_min_len(64)
            .filter_map(|e| {
                if e.removed || still(e) {
                    return None;
                }
                e.age += 1;
                e.hurt_cooldown = e.hurt_cooldown.saturating_sub(1);
                e.swing_ticks = e.swing_ticks.saturating_add(1);
                e.melee_recovery = e.melee_recovery.saturating_sub(1);
                if let Some(inv) = inventories
                    .get(&e.id)
                    .filter(|_| c.game.entities[e.kind as usize].hand_slots > 0)
                {
                    e.notice_hand(crate::content::in_main_hand(c, e.kind, inv, e.selected));
                }
                if e.player {
                    // Mods saw it this tick; the next `Move` tells of the next.
                    e.jumped = false;
                }
                if e.player && (!e.alive() || e.mode.has(crate::tag_bits::game_mode::NO_INTERACT)) {
                    return None;
                }
                let k = &c.game.entities[e.kind as usize];
                let (mut fell, mut splash) = (None, None);
                let mut crushed = Vec::new();
                // Players move on their client, bodies moved above, projectiles after; the rest here.
                if !e.player && !k.is(ef::SOLID) && k.projectile.is_none() {
                    let (wet, falling, before) = (e.in_liquid, -e.vel.y * crate::world::TICK, e.pos);
                    let s = step(c, e, gravity, &block, bodies);
                    crushed = s.crushed;
                    (fell, splash) = (s.land, super::entity::splashes(wet, e.in_liquid, falling).then_some(before));
                }
                // The liquids where the entity is now, and whether they burn; what sets it on fire
                // keeps it burning.
                let wet = [e.pos, e.pos + DVec3::Y * crate::physics::body_level(e.height(c))]
                    .iter()
                    .any(|p| {
                        crate::physics::liquid_state_at(c, &block, *p)
                            .and_then(|s| c.states[s as usize].liquid)
                            .is_some_and(|l| !c.liquids[l as usize].burns())
                    });
                let mut met = hazards_met(c, &block, e);
                // Being in what burns, which keeps doing its own damage.
                let hot = met.iter().any(|(_, h)| h.fire > 0);
                // Its kind's own and the dimension's, and what squeezed it.
                let own = k.hazards.iter().chain(&c.dims[here as usize].hazards);
                let due = |h: &&crate::content::Hazard| h.every <= 1 || (now + e.id) % h.every as u64 == 0;
                met.extend(
                    own.filter(|h| h.at == abi::hazard_at::SELF)
                        .filter(due)
                        .map(|h| (0, *h)),
                );
                met.extend(crushed.drain(..).map(|h| (0, h)));
                for f in crate::world::effects_on(comps.get(&(e.id, c.game.fields.effect)).map(|b| &b[..]), now) {
                    let of = c
                        .game
                        .entity_effects
                        .get(f.id as usize)
                        .filter(|d| !d.hazards.is_empty())
                        .map_or(Vec::new(), |d| d.hazards_at(f.level));
                    met.extend(
                        of.into_iter()
                            .filter(|h| h.at == abi::hazard_at::SELF)
                            .map(|h| (0, h)),
                    );
                }
                if e.mode.has(crate::tag_bits::game_mode::NOCLIP) {
                    met.clear();
                }
                // What sets things burning burns a dropped stack up, as lava and fire do in vanilla.
                if hot && !e.item.is_empty() {
                    e.gone(abi::removal::BURNED);
                }
                if wet && !hot {
                    e.fire = 0;
                }
                e.fire = e.fire.saturating_sub(1);
                let burn = e.fire > 0 && e.fire % crate::world::TPS == 0 && !hot;
                let despawn = c.game.categories[k.category as usize].despawn;
                // A named one, or one that picked something up or a mod keeps, stays.
                let kept = || comps.contains_key(&(e.id, c.game.persistent_comp)) || comps.contains_key(&(e.id, c.game.name_comp));
                if despawn > 0.0 && e.age % 100 == 0 && !kept() {
                    let near = players
                        .iter()
                        .any(|p| (*p - e.pos).length_squared() < despawn * despawn);
                    if !near {
                        e.gone(abi::removal::DESPAWNED);
                    }
                }
                if !e.item.is_empty() {
                    e.pickup_delay = e.pickup_delay.saturating_sub(1);
                    if e.age > ITEM_LIFETIME {
                        e.gone(abi::removal::EXPIRED);
                    }
                }
                (fell.is_some() || splash.is_some() || burn || !met.is_empty()).then_some(TickHit { id: e.id, fell, splash, burn, met })
            })
            .collect();
        let cram = crate::config::get(c.game.max_cramming) as u32;
        let mut crammed = Vec::new();
        for (i, push, too_many) in super::entity::soft_push(c, &self.entities.list, cram) {
            self.entities.list[i].vel += push;
            if too_many {
                crammed.push(self.entities.list[i].id);
            }
        }
        // Vanilla's: a quarter of the ticks, 6 a time.
        for id in crammed {
            if self.random() % 4 == 0 {
                self.damage(0, c, id, &abi::Damage::new(6.0, damage::CRAMMING));
            }
        }
        self.seat_riders(c);
        for TickHit { id, fell, splash, burn, met } in hits {
            self.fall_effects(0, c, id, fell, splash);
            if let Some(f) = fell {
                if let Some(amount) = self.fall_damage(0, c, id, f) {
                    self.damage(0, c, id, &abi::Damage::new(amount, damage::FALL));
                }
                self.riders_fall(0, c, id, f);
            }
            if burn {
                self.damage(0, c, id, &abi::Damage::new(1.0, damage::FIRE));
            }
            for (state, h) in met {
                let Some(e) = self
                    .entities
                    .get(id)
                    .filter(|_| h.every <= 1 || (self.time + id) % h.every as u64 == 0)
                else {
                    continue;
                };
                // What is about itself is asked at its eyes (the sky over its head).
                let pos = if h.at == abi::hazard_at::SELF { e.pos + DVec3::Y * e.eye(c) } else { e.pos };
                let ask = crate::conditions::Ask { pos, entity: id, state, body: Some(e.body()), ..Default::default() };
                if h.when
                    .is_some_and(|w| !crate::conditions::on_server(self, 0, c, w, &ask))
                {
                    continue;
                }
                if h.fire > 0 {
                    let burning = self.attribute(0, c, id, crate::registry::attr::BURNING_TIME, false);
                    if let Some(e) = self.entities.get_mut(id) {
                        e.fire = e.fire.max((h.fire as f64 * burning) as u32);
                    }
                }
                if let Some(cause) = h.cause {
                    self.damage(0, c, id, &abi::Damage::new(h.damage, cause));
                }
            }
        }
        self.turn_bodies();
        self.fly_projectiles(c);
        self.items(c);
        self.land_falling(c);
    }

    /// Bodies turn by the tick (`physics::turn_body`) after this tick's moves, by how they
    /// moved on what they stand on, as every client turns them from the same moves.
    fn turn_bodies(&mut self) {
        let frames: Vec<(u64, DVec3)> = self
            .bodies
            .keys()
            .filter_map(|&b| self.entities.get(b).map(|e| (b, e.pos - e.body_from)))
            .collect();
        for e in &mut self.entities.list {
            let carried = e
                .frame
                .and_then(|f| frames.iter().find(|b| b.0 == f))
                .map_or(DVec3::ZERO, |b| b.1);
            let walked = e.pos - e.body_from - carried;
            e.body_yaw = crate::physics::turn_body(e.body_yaw, e.yaw, walked);
            let speed = (walked.with_y(0.0).length() * crate::world::TPS as f64) as f32;
            crate::model::walk_limbs(&mut e.strode, &mut e.stride, speed, crate::world::TICK as f32);
            e.body_from = e.pos;
        }
    }

    /// Systems over per-entity components: values copied into one aligned array, handed over
    /// in one call, written back (changes of synced components marked for sending).
    fn run_systems(&mut self, c: &'static Content) {
        if c.game.systems.is_empty() {
            return;
        }
        // Every entity a system works on remembers the time it was simulated to. One that fell
        // behind (unloaded, the time jumped) makes the missed ticks up right here, before its
        // normal tick: in at most `CATCH_UP_PARTS` calls whatever was missed, each followed by
        // the energy network moving that many ticks' worth, so machines that feed each other
        // keep pace (a generator does not fill its buffer and stop while the furnace waits).
        let (sim, now) = (c.game.simulated_comp, self.time);
        let worked: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| {
                !e.removed
                    && c.game
                        .systems
                        .iter()
                        .any(|s| self.entities.comps.contains_key(&(e.id, s.1)))
            })
            .map(|e| e.id)
            .collect();
        let blocks = self.entities.system_blocks.iter().copied();
        let mut missed: Vec<(u64, u64)> = Vec::new();
        for id in worked.into_iter().chain(blocks.collect::<Vec<_>>()) {
            let last = self
                .entities
                .comps
                .get(&(id, sim))
                .and_then(|b| b.get(..8))
                .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
                .filter(|&t| t < now);
            if let Some(t) = last.filter(|&t| now - t > 1) {
                missed.push((id, now - t - 1));
            }
            self.entities
                .comps
                .insert((id, sim), now.to_le_bytes().into());
        }
        let mut part = FxHashMap::default();
        for k in 0..CATCH_UP_PARTS {
            part.clear();
            for &(id, n) in &missed {
                let parts = n.min(CATCH_UP_PARTS);
                if k < parts {
                    let ticks = n * (k + 1) / parts - n * k / parts;
                    part.insert(id, ticks.min(u32::MAX as u64) as u32);
                }
            }
            if part.is_empty() {
                break;
            }
            self.systems_once(c, Some(&part));
            for frame in self.frames_of(&part) {
                if self.active_space(frame) {
                    let (space, entities) = self.space_entities(frame);
                    space.grid.tick(entities, frame, c, Some(&part));
                }
            }
        }
        self.systems_once(c, None);
    }

    /// The frames the blocks of these entities are in.
    fn frames_of(&self, only: &FxHashMap<u64, u32>) -> Vec<u64> {
        let mut frames: Vec<u64> = only
            .keys()
            .map(|id| self.entities.owner_frame(*id))
            .collect();
        frames.sort_unstable();
        frames.dedup();
        frames
    }

    /// Every system once: over `only` (entity, ticks) when given, else over all with one tick,
    /// and for the blocks of each frame in that frame.
    fn systems_once(&mut self, c: &'static Content, only: Option<&FxHashMap<u64, u32>>) {
        for &(owner, comp, cb) in &c.game.systems {
            let size = c.game.components[comp as usize].size as usize;
            if size == 0 {
                continue;
            }
            let mut by_frame: FxHashMap<u64, Vec<u64>> = FxHashMap::default();
            for e in self
                .entities
                .list
                .iter()
                .filter(|e| !e.removed && self.entities.comps.contains_key(&(e.id, comp)) && only.is_none_or(|o| o.contains_key(&e.id)))
            {
                by_frame.entry(0).or_default().push(e.id);
            }
            let mut blocks: Vec<_> = self
                .entities
                .system_blocks
                .iter()
                .copied()
                .filter(|id| self.entities.comps.contains_key(&(*id, comp)) && only.is_none_or(|o| o.contains_key(id)))
                .collect();
            blocks.sort_unstable();
            for id in blocks {
                by_frame
                    .entry(self.entities.blocks[&id].frame)
                    .or_default()
                    .push(id);
            }
            let mut frames: Vec<u64> = by_frame.keys().copied().collect();
            frames.sort_unstable();
            for frame in frames {
                let ids = &by_frame[&frame];
                let mut buf = vec![0u64; (ids.len() * size).div_ceil(8)];
                let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, ids.len() * size) };
                for (i, id) in ids.iter().enumerate() {
                    bytes[i * size..(i + 1) * size].copy_from_slice(&self.entities.comps[&(*id, comp)]);
                }
                let ticks: Vec<u32> = ids
                    .iter()
                    .map(|id| only.and_then(|o| o.get(id).copied()).unwrap_or(1))
                    .collect();
                let owners: Vec<abi::Holder> = ids
                    .iter()
                    .map(|&id| if self.entities.blocks.contains_key(&id) { abi::Holder::block_id(id) } else { abi::Holder::entity(id) })
                    .collect();
                let (values, count, ticks) = (bytes.as_mut_ptr(), ids.len() as u32, ticks.as_ptr());
                if self.active_space(frame) {
                    self.call(frame, owner, cb, |w| abi::SystemArgs {
                        world: w,
                        component: comp,
                        owners: owners.as_ptr(),
                        values,
                        count,
                        stride: size as u32,
                        ticks,
                    });
                }
                let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, ids.len() * size) };
                for (i, id) in ids.iter().enumerate() {
                    let value = &bytes[i * size..(i + 1) * size];
                    if let Some(old) = self
                        .entities
                        .comps
                        .get_mut(&(*id, comp))
                        .filter(|old| &***old != value)
                    {
                        old.copy_from_slice(value);
                        self.brain_component_changed(c, *id, comp);
                    }
                }
            }
        }
    }

    /// Every body with `POSES` but a player (whose client does it) takes its pose by the rules a
    /// player's does: the keys its walk holds, the poses' conditions, room for it, a mod's held
    /// pose.
    fn choose_poses(&mut self, c: &'static Content) {
        let forced_comp = c.game.fields.forced;
        let todo: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.player && e.is_body() && e.alive())
            .filter(|e| c.game.entities[e.kind as usize].is(crate::tag_bits::entity::POSES))
            .map(|e| e.id)
            .collect();
        for id in todo {
            let Some(e) = self.entities.get(id) else { continue };
            let k = &c.game.entities[e.kind as usize];
            let forced = self
                .entities
                .comps
                .get(&(id, forced_comp))
                .and_then(|b| <crate::net::ForcedPoseField as crate::net::codec::Wire>::from_bytes(b))
                .map(|f| crate::world::Pose::from_id(f.pose));
            let size = e.size.unwrap_or(k.size());
            let (pos, walk, hw) = (e.pos, e.walk, size.width as f64 / 2.0);
            let low = e.pose.def().tilt != abi::pose_tilt::UPRIGHT;
            let inv = self
                .entities
                .inventories
                .get(&id)
                .map_or(&[][..], Vec::as_slice);
            let swims = crate::content::swims_in(c, e.kind, crate::content::equipped(c, e.kind, inv));
            let block = super::space::CubeCells(&self.space.cubes);
            let swim_at = pos + DVec3::Y * if low { 0.1 } else { e.dims(c).eye };
            let in_liquid = crate::physics::liquid_at(c, &block, swim_at).is_some_and(|l| swims.get(l as usize) == Some(&true));
            let flying = e.flying;
            let sprints = walk.is_some_and(|w| w.has(abi::walk::SPRINT) && !w.has(abi::walk::SNEAK) && w.dir != [0.0; 2]);
            let fit: Vec<bool> = (0..c.game.poses.len() as u16)
                .map(|p| {
                    let h = crate::physics::posed(size, crate::world::Pose(p)).height;
                    !crate::physics::collides(c, &block, crate::physics::Aabb::around(pos, hw - 1e-3, h))
                })
                .collect();
            let pose = crate::world::Pose::choose(
                forced,
                |d, keys_too| {
                    // A pose fallen back to goes without the keys, sprinting counted, as a player's.
                    let body = crate::conditions::Body { in_liquid, flying, sprinting: sprints || !keys_too };
                    let ask = crate::conditions::Ask { pos, entity: id, body: Some(body), ..Default::default() };
                    (!keys_too || d.key.is_empty() || walk.is_some_and(|w| w.holds(&d.key)))
                        && d.when
                            .is_none_or(|w| crate::conditions::on_server(self, 0, c, w, &ask))
                },
                |p| fit[p.0 as usize],
            );
            if let Some(e) = self.entities.get_mut(id) {
                e.pose = pose;
            }
        }
    }

    /// Item entities: merge neighbors, get picked up by what picks up (`PICKS_UP`).
    fn items(&mut self, c: &'static Content) {
        let takers: Vec<(u64, DVec3)> = self
            .entities
            .list
            .iter()
            .filter(|e| e.is_body() && e.alive() && c.game.entities[e.kind as usize].is(crate::tag_bits::entity::PICKS_UP))
            .map(|e| (e.id, e.pos))
            .collect();
        // Who is next to whom, once a merge is due: everyone asking everyone was the one thing
        // that took a tick over its time with thousands of items lying around.
        let merging = self.time % 10 == 0;
        let mut cells: FxHashMap<IVec3, Vec<usize>> = FxHashMap::default();
        if merging {
            for (i, e) in self
                .entities
                .list
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.removed && !e.item.is_empty())
            {
                cells.entry(e.pos.floor().as_ivec3()).or_default().push(i);
            }
        }
        let items: Vec<(u64, DVec3)> = self
            .entities
            .list
            .iter()
            .filter(|e| !e.removed && !e.item.is_empty())
            .map(|e| (e.id, e.pos))
            .collect();
        for (id, pos) in items {
            let Some(e) = self.entities.get(id) else { continue };
            if e.pickup_delay > 0 {
                continue;
            }
            let stack = e.item;
            // The nearest one that has room, not just the first in reach.
            let near = takers
                .iter()
                .filter(|(_, pp)| (*pp + DVec3::Y * 0.5 - pos).length_squared() < 2.25);
            if let Some(&(pid, _)) = near
                .filter(|(p, _)| self.entities.can_insert(c, *p, stack))
                .min_by(|a, b| {
                    (a.1 - pos)
                        .length_squared()
                        .total_cmp(&(b.1 - pos).length_squared())
                })
            {
                let mut q = abi::Query { entity: pid, other: id, item: stack.item, amount: stack.count as f64, ..abi::Query::DEFAULT };
                if !self.ask(0, c, abi::action::PICKUP_ITEM, &mut q) {
                    continue;
                }
                let rest = self.entities.insert_item(c, pid, stack);
                let taken = stack.count - rest.count;
                if taken > 0 {
                    let pickup = c
                        .game
                        .catalog
                        .find(ek::SOUND, "core:item.pickup")
                        .unwrap_or(u32::MAX);
                    self.sound(0, pickup, pos, 1.0, 0.0);
                    self.emit_engine(
                        0,
                        c,
                        abi::action::PICKUP_ITEM,
                        &abi::Pickup { entity: pid, stack: ItemStack { count: taken, ..stack } },
                    );
                    // A mob that picked something up keeps it: it no longer despawns, as vanilla's.
                    if self.entities.get(pid).is_some_and(|e| !e.player) {
                        self.entities
                            .comps
                            .insert((pid, c.game.persistent_comp), Box::new([]));
                    }
                }
                if let Some(e) = self.entities.get_mut(id) {
                    if rest.is_empty() {
                        e.gone(abi::removal::PICKED_UP);
                        self.entities.taken.push((id, pid));
                    } else {
                        e.item = rest;
                    }
                }
                continue;
            }
            if merging {
                let max = c.max_stack(&stack);
                let at = pos.floor().as_ivec3();
                let list = &self.entities.list;
                let other = (-1..=1)
                    .flat_map(|dx| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| at + IVec3::new(dx, dy, dz))))
                    .flat_map(|cell| cells.get(&cell).into_iter().flatten().copied())
                    .find(|&i| {
                        let o = &list[i];
                        o.id != id
                            && !o.removed
                            && o.item.item == stack.item
                            && o.item.data == stack.data
                            && o.item.count + stack.count <= max
                            && (o.pos - pos).length_squared() < 1.0
                    });
                if let Some(oi) = other {
                    self.entities.list[oi].item.count += stack.count;
                    if let Some(e) = self.entities.get_mut(id) {
                        e.gone(abi::removal::MERGED);
                    }
                }
            }
        }
    }

    pub fn flush_neighbors(&mut self, space_id: u64, c: &'static Content) {
        if !c.dims[self.id as usize].block_updates {
            self.space_mut(space_id).neighbor_queue.clear();
            self.space_mut(space_id).edge_updates.clear();
            return;
        }
        let space = self.space_mut(space_id);
        let share = space.edge_updates.len().min(MAX_NEIGHBOR_UPDATES / 2);
        space
            .neighbor_queue
            .extend(space.edge_updates.drain(..share).map(|p| (p, p)));
        let mut budget = MAX_NEIGHBOR_UPDATES;
        while let Some((p, from)) = self.space_mut(space_id).neighbor_queue.pop_front() {
            if budget == 0 {
                log::warn!(
                    "neighbor update storm in dimension {}, dropping {} updates",
                    self.id,
                    self.space(space_id).neighbor_queue.len()
                );
                self.space_mut(space_id).neighbor_queue.clear();
                break;
            }
            budget -= 1;
            let Some(state) = self.get(space_id, p) else { continue };
            let layout = self.space(space_id).layout(p);
            if *layout == crate::world::placement::CellLayout::EMPTY {
                self.neighbor_changed(space_id, c, p, state, from);
                continue;
            }
            let parts: Vec<_> = layout.parts(state).map(|part| part.owner(p)).collect();
            for (owner, part) in parts {
                if self.has_placed(space_id, owner, part) {
                    self.as_placed(space_id, owner, part, |d| d.neighbor_changed(space_id, c, owner, part.state, from));
                }
            }
        }
    }

    /// A part of the cell at `p` hears of a change at `from`.
    fn neighbor_changed(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32, from: IVec3) {
        if self.neighbor_rules(space_id, c, p, state, false) {
            return;
        }
        for r in &c.states[state as usize].rules {
            if let When::Neighbor(d) = r.when
                && d > 0
            {
                self.schedule(space_id, p, d);
            }
        }
        self.consider(space_id, c, p, self.time);
        if c.states[state as usize].liquid.is_some() {
            self.liquid_neighbor(space_id, c, p, state);
            return;
        }
        self.as_part(space_id, p, state, |d| {
            d.call_block(
                space_id,
                c,
                state,
                |b| b.on_neighbor,
                |w| abi::BlockEventArgs { world: w, pos: p.to_array().into(), state, other: from.to_array().into(), entity: 0 },
            )
        });
    }

    /// The block `state` would be held up at `p`: no neighbor rule of it breaks it, and what it
    /// lies on holds (unloaded neighbors count as holding).
    pub(super) fn held_at(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: Part) -> bool {
        let state = part.state;
        let mut ask = self.ask_at(space_id, p, state);
        ask.placement = Some((p, part));
        !c.states[state as usize]
            .rules
            .iter()
            .filter(|r| matches!(r.when, When::Neighbor(_)) && matches!(r.then, Then::Break | Then::Shed(_)))
            .any(|r| {
                r.cond
                    .is_none_or(|w| crate::conditions::on_server(self, space_id, c, w, &ask))
            })
    }

    /// What `state` at `p` asks its conditions about.
    /// Light counts with the daylight of the moment being caught up on, if it is one.
    fn ask_at(&self, space_id: u64, p: IVec3, state: u32) -> crate::conditions::Ask<'static> {
        let daylight = self
            .space(space_id)
            .growing_at
            .map(|at| self.daylight_at(crate::content::content(), at));
        crate::conditions::Ask {
            pos: self
                .space_pose(space_id)
                .to_world(p.as_dvec3() + DVec3::splat(0.5)),
            state,
            daylight,
            placement: self.resident(space_id, p, state).map(|v| (p, v)),
            own_frame: true,
            ..Default::default()
        }
    }

    /// `part`'s neighbor rules, those at once or (`later`) those after a while: the first that
    /// holds is done. Whether one was.
    fn neighbor_rules(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: u32, later: bool) -> bool {
        let ask = self.ask_at(space_id, p, part);
        for r in &c.states[part as usize].rules {
            let When::Neighbor(d) = r.when else { continue };
            if (d > 0) == later
                && c.acts(part, r.then)
                && r.cond
                    .is_none_or(|w| crate::conditions::on_server(self, space_id, c, w, &ask))
            {
                self.act(space_id, c, p, part, r.then);
                return true;
            }
        }
        false
    }

    /// What a rule of `part` of the cell at `p` does.
    fn act(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: u32, then: Then) {
        match then {
            Then::Become(s) => {
                if !self.replace_part(space_id, c, p, part, s) {
                    self.replace_part(space_id, c, p, part, AIR);
                }
            }
            Then::Grow => match c.block_of(part).cb.on_grow {
                Some(_) => {
                    self.as_part(space_id, p, part, |d| {
                        d.call_block(
                            space_id,
                            c,
                            part,
                            |b| b.on_grow,
                            |w| abi::BlockEventArgs { world: w, pos: p.to_array().into(), state: part, other: [0; 3].into(), entity: 0 },
                        )
                    });
                }
                None => {
                    if let Some(next) = c.grown(part) {
                        self.replace_part(space_id, c, p, part, next);
                    }
                }
            },
            Then::Break => self.break_off(space_id, c, p, part),
            Then::Shed(into) => {
                let alone = c.states[part as usize]
                    .pieces
                    .iter()
                    .find(|pc| pc.left == into)
                    .map(|pc| pc.alone);
                let gone = c.states[part as usize].sides != 0 && c.states[into as usize].sides == 0;
                self.replace_part(space_id, c, p, part, if gone { AIR } else { into });
                self.block_dust(space_id, c, p, part);
                if let Some(alone) = alone {
                    self.drop_block_loot(space_id, c, p, alone, ItemStack::EMPTY, false, 0);
                }
            }
            Then::Fall => {
                self.fall_block(space_id, c, p);
            }
        }
    }

    /// Breaks a block that lost its support, dropping what it drops.
    pub(super) fn break_off(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32) {
        self.replace_part(space_id, c, p, state, AIR);
        self.block_dust(space_id, c, p, state);
        self.drop_block_loot(space_id, c, p, state, ItemStack::EMPTY, false, 0);
        self.emit_engine(space_id, c, abi::action::BREAK_BLOCK, &abi::BlockChange { player: 0, pos: p.to_array().into(), state, item: 0 });
    }

    pub(crate) fn active_space(&self, frame: u64) -> bool {
        self.has_space(frame)
            && (frame == 0
                || self
                    .entities
                    .get(frame)
                    .is_some_and(|e| e.motion != Motion::Parked))
    }

    /// Neighbor and event effects can cross into another space through a mod callback.
    pub fn settle_actions(&mut self, c: &'static Content) {
        self.settle(0, c);
        let mut frames: Vec<_> = self
            .bodies
            .iter()
            .filter_map(|(&frame, space)| (!space.neighbor_queue.is_empty() || !space.event_queue.is_empty()).then_some(frame))
            .collect();
        frames.sort_unstable();
        for frame in frames {
            if self.active_space(frame) {
                self.settle(frame, c);
            }
        }
    }

    pub fn settle(&mut self, space_id: u64, c: &'static Content) {
        self.flush_neighbors(space_id, c);
        self.deliver_events(space_id, c);
    }

    pub(super) fn publish_spaces(&mut self) {
        if !self.bodies.is_empty() && !self.space.light_dirty.is_empty() {
            self.bodies_see_light();
        }
        for space in std::iter::once(&mut *self.space).chain(self.bodies.values_mut().map(Box::as_mut)) {
            space.publish();
        }
    }

    /// What `action::BREAK_BLOCK` says about it.
    pub fn may_break(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32, item: u32, player: u64) -> bool {
        let mut q = abi::Query { entity: player, pos: p.to_array().into(), state, item, ..abi::Query::DEFAULT };
        self.ask(space_id, c, abi::action::BREAK_BLOCK, &mut q)
    }

    /// The surface of a liquid counts, not only the block it is in.
    pub fn head_in_liquid(&self, c: &Content, e: &super::entity::Entity) -> bool {
        self.observed_liquid_at(0, c, e.pos + DVec3::Y * e.eye(c))
            .is_some()
    }

    /// The share of `part` of the cell at `pos` in `frame` an entity's mining does this tick, by
    /// what it holds and where it is now.
    pub fn mining_rate_of(&mut self, c: &'static Content, id: u64, frame: u64, pos: IVec3, part: Part) -> f32 {
        let Some(e) = self.entities.get(id) else { return 0.0 };
        let (wet, on_ground, creative) = (self.head_in_liquid(c, e), e.on_ground, e.mode.has(crate::tag_bits::game_mode::INSTANT_BREAK));
        let tool = self.entities.held(id);
        let mut attr = |a| self.attribute(0, c, id, a, false) as f32;
        use crate::registry::attr as a;
        let speed = attr(a::MINING_SPEED)
            * if wet { attr(a::SUBMERGED_MINING_SPEED) } else { 1.0 }
            * if on_ground { 1.0 } else { attr(a::AIRBORNE_MINING_SPEED) };
        let state = if self.active_space(frame) && self.has_placed(frame, pos, part) { part.state } else { AIR };
        c.mining_rate(state, &tool, speed, creative)
    }

    /// Pushes an entity, by blocks per second. Its client moves a player, so the push goes there.
    pub fn push(&mut self, id: u64, v: DVec3) {
        let Some(e) = self.entities.get_mut(id) else { return };
        if e.player {
            let add = (v / crate::world::TPS as f64).as_vec3().to_array();
            self.entities.told.push((id, Told::Add(add)));
        } else {
            e.vel += v;
        }
    }

    /// One melee hit with what the attacker holds, which wears from it.
    pub fn melee(&mut self, space_id: u64, c: &'static Content, attacker: u64, target: u64) -> bool {
        let Some(slot) = self.entities.get(attacker).map(|e| e.selected) else { return false };
        let held = self.entities.held(attacker);
        let hit = self.attack(space_id, c, attacker, target, held);
        // Bare hands wear nothing, whatever sits in the slot doing nothing.
        if hit && !held.is_empty() {
            super::actions::wear(c, self, attacker, slot, c.stack_value::<u32>(&held, c.ids.wear_per_hit).unwrap_or(2));
        }
        hit
    }

    /// `break_part` of what a player hit at `hit` on `face`: the piece hit, if the state has
    /// pieces (`StateInfo::piece_hit`), goes alone.
    #[allow(clippy::too_many_arguments)]
    pub fn break_piece(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        part: u32,
        face: u8,
        hit: [f32; 3],
        tool: ItemStack,
        player: u64,
        drops: bool,
    ) -> bool {
        let state = part;
        let Some(&piece) = c.states[state as usize]
            .piece_hit(hit, face)
            .filter(|piece| piece.left != AIR)
        else {
            return self.break_part(space_id, c, p, state, tool, player, drops);
        };
        if !self.may_break(space_id, c, p, piece.alone, tool.item, player) {
            return false;
        }
        self.replace_part(space_id, c, p, state, piece.left);
        self.block_dust(space_id, c, p, piece.alone);
        self.sound(
            space_id,
            c.states[state as usize].effects[crate::content::block_event::BREAK].sound,
            p.as_dvec3() + DVec3::splat(0.5),
            1.0,
            0.0,
        );
        if drops {
            self.drop_block_loot(space_id, c, p, piece.alone, tool, true, 0);
        }
        self.emit_engine(
            space_id,
            c,
            abi::action::BREAK_BLOCK,
            &abi::BlockChange { player, pos: p.to_array().into(), state: piece.alone, item: tool.item },
        );
        true
    }

    /// A client's picture of a cell it was wrong about is put right.
    pub(super) fn resend_cell(&mut self, space_id: u64, p: IVec3) {
        if let Some(state) = self.get(space_id, p) {
            let space = self.space_mut(space_id);
            if space.not_shown.contains_key(&p) {
                space.light_dirty.insert(cube_of(p));
                return;
            }
            let parts = space.layout(p).clone();
            space.changes.push((p, state));
            space.extra_changes.push((p, parts));
        }
    }

    pub(super) fn has_placed(&self, space_id: u64, p: IVec3, part: Part) -> bool {
        self.get(space_id, p).is_some_and(|main| {
            self.space(space_id)
                .layout(p)
                .parts(main)
                .any(|v| v == part)
        })
    }

    /// `break_block` of all that is in the cell that can be aimed at: a liquid in it stays.
    pub fn break_block(&mut self, space_id: u64, c: &'static Content, p: IVec3, tool: ItemStack, player: u64, drops: bool) -> bool {
        let Some(main) = self.get(space_id, p) else { return false };
        let parts: Vec<_> = self
            .space(space_id)
            .layout(p)
            .parts(main)
            .filter(|v| c.states[v.state as usize].flags & bf::NOT_TARGETABLE == 0)
            .map(|v| v.owner(p))
            .collect();
        let mut any = false;
        for (owner, part) in parts {
            any |= self.as_placed(space_id, owner, part, |d| d.break_part(space_id, c, owner, part.state, tool, player, drops));
        }
        any
    }

    /// Breaks one block of the cell at `p`.
    pub fn break_part(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        state: u32,
        tool: ItemStack,
        player: u64,
        drops: bool,
    ) -> bool {
        if state == AIR || c.states[state as usize].hardness < 0.0 {
            return false;
        }
        if !self.may_break(space_id, c, p, state, tool.item, player) {
            return false;
        }
        // What the block carries goes back onto the block's item.
        let carried = self.anchored_part(space_id, c, p, state).map_or(0, |id| {
            crate::stack::intern(
                c.game
                    .carried
                    .iter()
                    .filter_map(|&comp| {
                        self.entities
                            .comps
                            .get(&(id, comp))
                            .map(|v| (comp, v.clone()))
                    })
                    .filter(|(_, v)| !v.is_empty())
                    .collect(),
            )
        });
        if !self.replace_part(space_id, c, p, state, AIR) {
            return false;
        }
        self.block_dust(space_id, c, p, state);
        self.sound(
            space_id,
            c.states[state as usize].effects[crate::content::block_event::BREAK].sound,
            p.as_dvec3() + DVec3::splat(0.5),
            1.0,
            0.0,
        );
        if drops {
            self.drop_block_loot(space_id, c, p, state, tool, true, carried);
        }
        self.emit_engine(
            space_id,
            c,
            abi::action::BREAK_BLOCK,
            &abi::BlockChange { player, pos: p.to_array().into(), state, item: tool.item },
        );
        true
    }

    /// `state`'s item used on `face` of `clicked`, at `hit` from its corner: where it goes, if it
    /// does. It goes where the point just outside the surface is, a bottom slab's top being still
    /// in its own cell; something with no room there goes in the next.
    #[allow(clippy::too_many_arguments)]
    pub fn place_clicked(
        &mut self,
        space_id: u64,
        c: &'static Content,
        clicked: IVec3,
        surface: Part,
        face: u8,
        state: u32,
        look: [f32; 3],
        hit: [f32; 3],
        player: u64,
        item: u32,
        by_face: bool,
    ) -> Option<(IVec3, Part)> {
        let space = self.space(space_id);
        let replacing = space
            .layout(clicked)
            .parts(space.get(clicked)?)
            .all(|part| c.states[part.state as usize].flags & bf::REPLACEABLE != 0);
        let (p, placed, hit) = crate::physics::placement(c, state, clicked, surface, face, hit, look, replacing, by_face)?;
        self.place(space_id, c, p, face, placed, look, hit, player, item)
            .then_some((p, placed))
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        face: u8,
        placed: Part,
        look: [f32; 3],
        hit: [f32; 3],
        player: u64,
        item: u32,
    ) -> bool {
        if !c.dims[self.id as usize].editable {
            return false;
        }
        let mut chosen = placed.state;
        let def = c.block_of(chosen);
        let Some(main) = self.get(space_id, p) else { return false };
        // The same block may take it in.
        let mut merged = None;
        if let Some((owner, cb)) = def.cb.merge {
            for resident in self
                .space(space_id)
                .layout(p)
                .residents(main)
                .filter(|part| part.offset == placed.offset && c.states[part.state as usize].block == c.states[chosen as usize].block)
            {
                let part = resident.state;
                let mut args = abi::PlaceArgs {
                    pos: p.to_array().into(),
                    face: face as u32,
                    look,
                    state: part,
                    first: def.first_state,
                    hit,
                    placed: chosen,
                };
                host::invoke(owner, cb, &mut args);
                if args.state != part && args.state >= def.first_state && args.state < def.first_state + def.states {
                    merged = Some((resident, args.state));
                    break;
                }
            }
        }
        let shape_of = merged.map_or(chosen, |m| m.1);
        let result = Part { state: shape_of, ..placed };
        let mut replaced = Vec::new();
        let footprint = result.footprint(c);
        for delta in std::iter::once(IVec3::ZERO).chain(footprint.iter().copied()) {
            let cell = p + delta;
            let Some(main) = self.get(space_id, cell) else { return false };
            let space = self.space(space_id);
            if space.signals.reserved(cell) {
                return false;
            }
            for other in space.layout(cell).parts(main) {
                let (owner, resident) = other.owner(cell);
                if merged.is_some_and(|(old, _)| owner == p && old == resident)
                    || c.may_share(shape_of, other.state)
                    || !result.in_cell(delta).overlaps(other, c)
                {
                    continue;
                }
                let s = &c.states[other.state as usize];
                if s.flags & bf::REPLACEABLE == 0 || s.block == c.states[chosen as usize].block {
                    return false;
                }
                if !replaced.contains(&(owner, resident)) {
                    replaced.push((owner, resident));
                }
            }
        }
        if !self.held_at(space_id, c, p, result) {
            return false;
        }
        // Not into anyone there, the one placing it too, as far as it fills the block. A structure
        // is where its blocks are, not where its entity is.
        let mut shape = Vec::new();
        crate::physics::boxes(c, shape_of, &mut shape);
        let at = p.as_dvec3() + placed.shift();
        let taken = shape.iter().any(|b| {
            let part = self
                .space_pose(space_id)
                .box_to_world(crate::physics::Aabb { min: at + DVec3::new(b[0], b[1], b[2]), max: at + DVec3::new(b[3], b[4], b[5]) });
            self.entities
                .list
                .iter()
                .any(|e| e.is_body() && e.id != space_id && !self.bodies.contains_key(&e.id) && e.aabb(c).intersects(&part))
        });
        if taken {
            return false;
        }
        let mut q = abi::Query { entity: player, pos: p.to_array().into(), state: chosen, item, ..abi::Query::DEFAULT };
        if !self.ask(space_id, c, abi::action::PLACE_BLOCK, &mut q) {
            return false;
        }
        // What it replaces (a flower, tall grass) drops what it would when broken, unlike vanilla,
        // where it is lost or in the way. Creative loses it, as when breaking.
        let creative = self
            .entities
            .get(player)
            .is_some_and(|e| e.mode.has(crate::tag_bits::game_mode::NO_COSTS));
        if replaced.iter().any(|&(owner, part)| {
            part.footprint(c)
                .iter()
                .any(|&o| self.get(space_id, owner + o).is_none())
        }) {
            return false;
        }
        for (owner, gone) in replaced {
            if !self.replace_placed(space_id, c, owner, gone, AIR) {
                return false;
            }
            if !creative {
                self.drop_block_loot(space_id, c, owner, gone.state, ItemStack::EMPTY, false, 0);
            }
        }
        if let Some((old, whole)) = merged {
            if !self.replace_placed(space_id, c, p, old, whole) {
                return false;
            }
            chosen = whole;
        } else if !self.add_placed(space_id, c, p, placed) {
            return false;
        }
        self.drain_if_no_room(space_id, c, p);
        self.sound(
            space_id,
            c.states[chosen as usize].effects[crate::content::block_event::PLACE].sound,
            p.as_dvec3() + DVec3::splat(0.5),
            1.0,
            0.0,
        );
        self.emit_engine(
            space_id,
            c,
            abi::action::PLACE_BLOCK,
            &abi::BlockChange { player, pos: p.to_array().into(), state: chosen, item },
        );
        true
    }

    /// True if the block consumed the click. A sneaking player uses it as its `sneak_use` says.
    pub fn player_use(&mut self, space_id: u64, c: &'static Content, p: IVec3, part: u32, face: u8, player: u64, sneaking: bool) -> bool {
        let state = part;
        if state == AIR {
            return false;
        }
        if sneaking {
            let empty_hands = || {
                let off = self
                    .entities
                    .get(player)
                    .zip(self.entities.inventories.get(&player))
                    .map(|(e, i)| crate::content::in_off_hand(c, e.kind, i));
                self.entities.held(player).is_empty() && off.is_none_or(|s| s.is_empty())
            };
            let uses = match c.block_of(state).sneak_use {
                abi::sneak_use::ALWAYS => true,
                abi::sneak_use::NEVER => false,
                _ => empty_hands(),
            };
            if !uses {
                return false;
            }
        }
        let opens = c.block_of(state).opens;
        if c.block_of(state).usable() {
            let mut q = abi::Query { entity: player, pos: p.to_array().into(), state, ..abi::Query::DEFAULT };
            if !self.ask(space_id, c, abi::action::USE_BLOCK, &mut q) {
                // Refused uses are consumed: no item use or placement behind them.
                return true;
            }
            let item = self.entities.held(player).item;
            self.emit_engine(space_id, c, abi::action::USE_BLOCK, &abi::BlockChange { player, pos: p.to_array().into(), state, item });
        }
        if sneaking && self.edit_controller(space_id, c, p, state, face, player) {
            return true;
        }
        if let Some(menu) = opens {
            // Its inventory menu, or one of its own (a crafting table).
            let on = self
                .anchored_part(space_id, c, p, state)
                .map(|id| crate::server::holder::Inv::own(crate::server::holder::At::Block(id)));
            self.events
                .push(DimEvent::OpenMenu { player, menu, on, block: state });
            return true;
        }
        let r = self.as_part(space_id, p, state, |d| {
            d.call_block(
                space_id,
                c,
                state,
                |b| b.on_use,
                |w| abi::UseArgs {
                    world: w,
                    pos: p.to_array().into(),
                    state,
                    face: face as u32,
                    player,
                    handled: 0,
                    sneaking: sneaking as u32,
                },
            )
        });
        r.is_some_and(|a| a.handled != 0)
    }

    /// What time does to `p` next, in `mean` ticks on average from `from` (an exponential wait
    /// drawn from the place and time, like a random tick would come).
    fn schedule_at(&mut self, space_id: u64, p: IVec3, mean: f64, from: u64) {
        let h = hash(0x6F0 ^ from ^ (self.id as u64) << 48, p.x as i64, p.y as i64, p.z as i64);
        let u = ((h >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
        let at = from + (-u.ln() * mean).max(1.0) as u64;
        self.space_mut(space_id).growth.insert(p, at);
        self.space_mut(space_id)
            .growth_queue
            .push(Reverse((at, p.x, p.y, p.z)));
    }

    /// What may happen to the cell at `p` by itself, and how often (a share per tick): a rule
    /// now and then of a part of it that may hold some time, and something spreading onto it.
    fn chances(&mut self, space_id: u64, c: &'static Content, p: IVec3) -> Vec<(Chance, f64)> {
        let mut out = Vec::new();
        for part in self.parts(space_id, p) {
            let ask = self.ask_at(space_id, p, part);
            for (i, r) in c.states[part as usize].rules.iter().enumerate() {
                let When::Random(mean) = r.when else { continue };
                if c.acts(part, r.then)
                    && r.cond
                        .is_none_or(|w| crate::conditions::may_server(self, space_id, c, w, &ask))
                {
                    out.push((Chance::Rule(part, i), 1.0 / mean));
                }
            }
        }
        if let Some(state) = self.get(space_id, p).filter(|&s| c.spread_target(s)) {
            let ask = self.ask_at(space_id, p, state);
            let rate: f64 = self
                .spreaders_near(space_id, c, p, state)
                .into_iter()
                .filter(|(_, sp)| {
                    sp.when
                        .is_none_or(|w| crate::conditions::may_server(self, space_id, c, w, &ask))
                })
                .map(|(_, sp)| 1.0 / sp.mean)
                .sum();
            if rate > 0.0 {
                out.push((Chance::Spread, rate));
            }
        }
        out
    }

    /// What time does to `p` next, from `from`, if anything can: one wait for all its chances
    /// together. One waits at a time.
    fn consider(&mut self, space_id: u64, c: &'static Content, p: IVec3, from: u64) {
        if self.space(space_id).growth.contains_key(&p) {
            return;
        }
        let rate: f64 = self.chances(space_id, c, p).iter().map(|x| x.1).sum();
        if rate > 0.0 {
            self.schedule_at(space_id, p, 1.0 / rate, from);
        }
    }

    /// The blocks that spread onto `state` at `p` from where they reach it: state and rule.
    fn spreaders_near(&self, space_id: u64, c: &Content, p: IVec3, state: u32) -> Vec<(u32, crate::content::Spread)> {
        let mut out = Vec::new();
        let (lo, hi) = c.spread_search;
        for dy in lo.y..=hi.y {
            for dz in lo.z..=hi.z {
                for dx in lo.x..=hi.x {
                    let at = IVec3::new(dx, dy, dz);
                    let Some(s) = self.get(space_id, p + at) else { continue };
                    if let Some(sp) = c.states[s as usize].spread
                        && (-at).cmpge(sp.min).all()
                        && (-at).cmple(sp.max).all()
                        && c.matchers.has(sp.to, state)
                    {
                        out.push((s, sp));
                    }
                }
            }
        }
        out
    }

    /// Something spreading onto the block at `p` comes due: it takes it if its conditions hold
    /// then (`growing_at` gives the daylight), else tries again later.
    fn spread_onto(&mut self, space_id: u64, c: &'static Content, p: IVec3, state: u32) {
        let ask = self.ask_at(space_id, p, state);
        let into = self
            .spreaders_near(space_id, c, p, state)
            .into_iter()
            .find(|(_, sp)| {
                sp.when
                    .is_none_or(|w| crate::conditions::on_server(self, space_id, c, w, &ask))
            });
        if let Some((into, _)) = into {
            self.set(space_id, c, p, into);
        }
    }

    /// What time does comes due, in time order (catching up on unloaded time included): of the
    /// cell's chances one is picked by how often it comes, and done if it holds then.
    fn grow(&mut self, space_id: u64, c: &'static Content) {
        while let Some(&Reverse((at, x, y, z))) = self.space(space_id).growth_queue.peek() {
            if at > self.time {
                break;
            }
            self.space_mut(space_id).growth_queue.pop();
            let p = IVec3::new(x, y, z);
            if self.space(space_id).growth.get(&p) != Some(&at) {
                continue;
            }
            // Where only machines are kept, it waits as in an unloaded cube.
            if self.idle.contains(&cube_of(p)) {
                continue;
            }
            self.space_mut(space_id).growth.remove(&p);
            self.space_mut(space_id).growing_at = Some(at);
            // A rule breaking it whose cause no neighbor noticed (water two blocks off).
            let parts = self.parts(space_id, p);
            if !parts
                .iter()
                .any(|&s| self.neighbor_rules(space_id, c, p, s, false))
            {
                let chances = self.chances(space_id, c, p);
                let total: f64 = chances.iter().map(|x| x.1).sum();
                let mut pick = weft_noise::unit(hash(0x6F1 ^ at, p.x as i64, p.y as i64, p.z as i64)) * total;
                if let Some(&(chance, _)) = chances.iter().find(|x| {
                    pick -= x.1;
                    pick < 0.0
                }) {
                    match chance {
                        Chance::Rule(part, i) => {
                            let r = c.states[part as usize].rules[i];
                            let ask = self.ask_at(space_id, p, part);
                            if self.has_part(space_id, p, part)
                                && r.cond
                                    .is_none_or(|w| crate::conditions::on_server(self, space_id, c, w, &ask))
                            {
                                self.act(space_id, c, p, part, r.then);
                            }
                        }
                        Chance::Spread => {
                            if let Some(state) = self.get(space_id, p) {
                                self.spread_onto(space_id, c, p, state);
                            }
                        }
                    }
                }
            }
            // The next counts from this one, not from now.
            self.consider(space_id, c, p, at);
            self.space_mut(space_id).growing_at = None;
        }
    }

    /// Timers on the blocks of a cube.
    pub fn timers_in(&self, cube: IVec3) -> Vec<BlockTimer> {
        let mut out: Vec<BlockTimer> = self
            .space
            .scheduled
            .iter()
            .filter(|t| cube_of(IVec3::from_array(t.0.1)) == cube)
            .map(|t| t.0.clone())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The timers on the blocks of a cube going away, taken out with it.
    pub fn take_timers(&mut self, cube: IVec3) -> Vec<BlockTimer> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.space.scheduled)
            .into_iter()
            .partition(|t| cube_of(IVec3::from_array(t.0.1)) == cube);
        self.space.scheduled = kept.into_iter().collect();
        let mut out: Vec<BlockTimer> = gone.into_iter().map(|t| t.0).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The block's own tick (`abi::event::BLOCK_TICK`) in `delay` ticks.
    pub fn schedule(&mut self, space_id: u64, p: IVec3, delay: u32) {
        let c = content();
        let mut target = Vec::new();
        if let Some((_, part)) = self.space(space_id).acting.filter(|(cell, _)| *cell == p) {
            for coord in part.offset.to_array() {
                target.extend_from_slice(&coord.to_le_bytes());
            }
            target.extend_from_slice(c.state_name(part.state).as_bytes());
        }
        self.timer_at(space_id, p, delay, c.game.block_tick, &target);
    }

    pub fn timer_at(&mut self, space_id: u64, p: IVec3, delay: u32, event: u32, data: &[u8]) {
        let at = self.time + delay.max(1) as u64;
        self.space_mut(space_id)
            .scheduled
            .push(Reverse((at, p.to_array(), event, data.into())));
    }

    /// Someone here would see a gizmo of `group` (empty: anyone).
    pub fn gizmo_seen(&self, group: &str) -> bool {
        group.is_empty() || self.gizmo_groups.iter().any(|g| g == group)
    }

    pub fn random(&mut self) -> u64 {
        self.rng.next()
    }

    /// Uniform in 0..1 from the same stream.
    pub fn random_unit(&mut self) -> f32 {
        self.rng.float()
    }

    /// Sound event `sound` (`u32::MAX`: nothing) for players in its range.
    pub fn sound(&mut self, space_id: u64, sound: u32, pos: DVec3, volume: f32, pitch: f32) {
        if sound != u32::MAX {
            self.events.push(DimEvent::Sound {
                dim: self.id,
                who: abi::Who::Around,
                sound,
                pos: Some(self.space_pose(space_id).to_world(pos)),
                volume,
                pitch,
            });
        }
    }

    /// Vanilla's 4x4x4 pieces of a broken block flying apart, for everyone watching.
    pub fn block_dust(&mut self, space_id: u64, c: &Content, p: IVec3, state: u32) {
        let kind = c.states[state as usize].effects[crate::content::block_event::BREAK].particle;
        if kind == u32::MAX {
            return;
        }
        // Vanilla's: a grid over each box of its shape, a piece every quarter block or at least two
        // along a side, far enough from the box's faces not to start inside a neighbor.
        for b in &c.states[state as usize].selection {
            let size = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
            let cells = size.map(|s| ((s / 0.25).ceil() as u32).max(2));
            let pos = self
                .space_pose(space_id)
                .to_world(DVec3::from_array([0, 1, 2].map(|i| p[i] as f64 + (b[i] + size[i] / 2.0) as f64)))
                .to_array();
            let spread = [0, 1, 2].map(|i| size[i] * (0.5 - 0.5 / cells[i] as f32));
            let burst = abi::ParticleBurst {
                kind,
                pos: pos.into(),
                spread: spread.into(),
                velocity: [0.0, 2.0, 0.0].into(),
                speed: 3.6,
                outward: 1,
                count: cells[0] * cells[1] * cells[2],
                block: state,
                ..abi::ParticleBurst::DEFAULT
            };
            self.events
                .push(DimEvent::Particles { dim: self.id, who: abi::Who::Around, burst });
        }
    }

    /// Spawns what a block's loot table gives for `tool`; `mined`: by that tool, not by the world
    /// (support, liquids), which always harvests.
    /// `own_name`: what a renamed block entity was called, given back to the block's item.
    /// `carried`: stack data the block's own item gets (what its block entity carried).
    pub fn drop_block_loot(
        &mut self,
        space_id: u64,
        c: &'static Content,
        p: IVec3,
        state: u32,
        tool: ItemStack,
        mined: bool,
        carried: u32,
    ) {
        let b = c.block_of(state);
        let own_item = c
            .items
            .iter()
            .position(|i| i.block == Some(b.first_state))
            .map(|i| i as u32);
        for _ in 0..c.block_value::<u32>(state, c.ids.loot_rolls).unwrap_or(1) {
            for s in self.roll_loot(
                c,
                &b.name,
                &crate::loot::Ctx { tool, mined: if mined { state } else { 0 }, state, ..Default::default() },
                p.as_dvec3() + DVec3::splat(0.5),
                0,
            ) {
                let s = if Some(s.item) == own_item && carried != 0 { crate::stack::merged_data(s, carried) } else { s };
                self.spawn_item(space_id, c, p.as_dvec3() + DVec3::splat(0.5), s);
            }
        }
    }
}

/// Section = 4x4x4 cubes, the unit regions are made of.
#[inline]
pub fn section_of_cube(cube: IVec3) -> IVec3 {
    cube >> 2
}

#[inline]
pub fn section_of(p: DVec3) -> IVec3 {
    section_of_cube(cube_of(p.floor().as_ivec3()))
}

type Taken = (Entity, Option<Vec<ItemStack>>, Vec<(u32, Box<[u8]>)>);

/// Everything stored for a set of sections, moved between regions when they merge or split.
#[derive(Default)]
pub struct Parcel {
    cubes: Vec<(IVec3, Arc<Cube>, bool, bool)>,
    on_disk: Vec<IVec3>,
    newly_settled: Vec<IVec3>,
    edge_updates: Vec<IVec3>,
    light_dirty: Vec<IVec3>,
    edited: Vec<IVec3>,
    body_light_changes: Vec<(IVec3, [u8; 3])>,
    entities: Vec<Taken>,
    bodies: Vec<(u64, Box<Space>)>,
    scheduled: Vec<Reverse<BlockTimer>>,
    growth: Vec<(IVec3, u64)>,
    blocks: Vec<super::entity::MovedBlock>,
}

impl Dim {
    /// Sections this region occupies: loaded cubes plus where its entities stand.
    pub fn add_cube(&mut self, pos: IVec3, cube: Arc<Cube>) {
        if self.space.cubes.insert(pos, cube).is_none() {
            *self.section_cubes.entry(section_of_cube(pos)).or_default() += 1;
        }
    }

    fn cube_gone(&mut self, pos: IVec3) {
        let s = section_of_cube(pos);
        if let Some(n) = self.section_cubes.get_mut(&s) {
            *n -= 1;
            if *n == 0 {
                self.section_cubes.remove(&s);
            }
        }
    }

    pub fn sections(&self) -> FxHashSet<IVec3> {
        let mut s: FxHashSet<IVec3> = self.section_cubes.keys().copied().collect();
        s.extend(
            self.entities
                .list
                .iter()
                .filter(|e| !e.removed)
                .map(|e| section_of(self.entities.follows(e).pos)),
        );
        s
    }

    /// Moves out cubes, entities and pending ticks located in sections where `take` is true.
    pub fn take_sections(&mut self, take: impl Fn(IVec3) -> bool) -> Parcel {
        self.finish_moving(crate::content::content());
        let mut p = Parcel::default();
        let cubes: Vec<IVec3> = self
            .space
            .cubes
            .keys()
            .copied()
            .filter(|&c| take(section_of_cube(c)))
            .collect();
        for pos in cubes {
            let cube = self.space.cubes.remove(&pos).unwrap();
            self.cube_gone(pos);
            self.space.grid.cube_unloaded(pos);
            self.space
                .signals
                .cube_unloaded(crate::content::content(), pos);
            self.space.conn.remove(&pos);
            p.cubes
                .push((pos, cube, self.space.modified.remove(&pos), self.settled.remove(&pos)));
            if self.on_disk.remove(&pos) {
                p.on_disk.push(pos);
            }
        }
        // What was waiting on these cubes goes with them.
        p.newly_settled = self
            .newly_settled
            .extract_if(.., |q| take(section_of_cube(*q)))
            .collect();
        let (moved, kept): (Vec<IVec3>, Vec<IVec3>) = self
            .space
            .edge_updates
            .drain(..)
            .partition(|q| take(section_of_cube(cube_of(*q))));
        p.edge_updates = moved;
        self.space.edge_updates.extend(kept);
        p.light_dirty = self
            .space
            .light_dirty
            .iter()
            .copied()
            .filter(|q| take(section_of_cube(*q)))
            .collect();
        for q in &p.light_dirty {
            self.space.light_dirty.remove(q);
        }
        p.edited = self
            .space
            .edited
            .iter()
            .copied()
            .filter(|q| take(section_of_cube(*q)))
            .collect();
        for q in &p.edited {
            self.space.edited.remove(q);
        }
        // Not heard by clients yet, and this region may be going.
        p.body_light_changes = self
            .space
            .body_light_changes
            .extract_if(.., |(q, _)| take(section_of_cube(cube_of(*q))))
            .collect();
        let ids: Vec<u64> = self
            .entities
            .list
            .iter()
            .filter(|e| take(section_of(self.entities.follows(e).pos)))
            .map(|e| e.id)
            .collect();
        for id in ids {
            if let Some(t) = self.entities.take(id) {
                p.entities.push(t);
            }
            if let Some(st) = self.bodies.remove(&id) {
                p.bodies.push((id, st));
            }
        }
        if !p.entities.is_empty() {
            self.clean_observations();
        }
        let instances: Vec<_> = p
            .cubes
            .iter()
            .filter_map(|(cube, ..)| self.entities.blocks_in_cube.get(cube))
            .chain(
                p.bodies
                    .iter()
                    .filter_map(|(body, _)| self.entities.blocks_in_body.get(body)),
            )
            .flatten()
            .copied()
            .collect();
        p.blocks = self.entities.take_blocks(instances);
        let (moved, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.space.scheduled)
            .into_iter()
            .partition(|t| take(section_of_cube(cube_of(IVec3::from_array(t.0.1)))));
        self.space.scheduled = kept.into_iter().collect();
        p.scheduled = moved;
        p.growth = self
            .space
            .growth
            .iter()
            .filter(|(q, _)| take(section_of_cube(cube_of(**q))))
            .map(|(q, at)| (*q, *at))
            .collect();
        for (q, _) in &p.growth {
            self.space.growth.remove(q);
        }
        p
    }

    pub fn absorb(&mut self, c: &Content, p: Parcel) {
        self.on_disk.extend(p.on_disk);
        self.newly_settled.extend(p.newly_settled);
        self.space.edge_updates.extend(p.edge_updates);
        self.space.light_dirty.extend(p.light_dirty);
        self.space.edited.extend(p.edited);
        self.space.body_light_changes.extend(p.body_light_changes);
        for (pos, cube, modified, settled) in p.cubes {
            self.space.grid.cube_loaded(c, pos, &cube);
            self.space.signals.cube_loaded(c, pos, &cube);
            self.add_cube(pos, cube);
            if modified {
                self.space.modified.insert(pos);
            }
            if settled {
                self.settled.insert(pos);
                self.settled_gen += 1;
            }
        }
        for (e, inv, comps) in p.entities {
            self.entities.insert_with(c, e, inv, comps);
        }
        self.bodies.extend(p.bodies);
        for (id, block, inv, comps) in p.blocks {
            self.entities.insert_block(c, id, block, inv, comps);
        }
        self.entities.sort_by_id();
        self.space.scheduled.extend(p.scheduled);
        for (q, at) in p.growth {
            self.space.growth.insert(q, at);
            self.space.growth_queue.push(Reverse((at, q.x, q.y, q.z)));
        }
    }

    pub fn connectivity(&mut self, c: &Content, pos: IVec3) -> Option<u16> {
        if let Some(&v) = self.space.conn.get(&pos) {
            return Some(v);
        }
        let v = see_through(c, self.space.cubes.get(&pos)?);
        self.space.conn.insert(pos, v);
        Some(v)
    }

    /// Marks `pos` and the loaded cubes below it settled: nothing above will ever bring sky
    /// light (it is not going to be loaded), so their light is final as it is.
    pub fn settle_down(&mut self, pos: IVec3) {
        let mut p = pos;
        while self.space.cubes.contains_key(&p) && self.settled.insert(p) {
            self.settled_gen += 1;
            self.newly_settled.push(p);
            p -= IVec3::Y;
        }
    }

    pub fn set_rng(&mut self, seed: u64) {
        self.rng = Rng(seed);
    }
}

/// What may happen to a cell with time (`Dim::chances`).
#[derive(Clone, Copy)]
enum Chance {
    /// Rule `.1` of part `.0`.
    Rule(u32, usize),
    Spread,
}
