use crate::abi::{self, Str};
use crate::host;

/// A dimension as seen from a callback. Only valid for the duration of that callback.
#[derive(Clone, Copy)]
pub struct World {
    raw: *mut abi::WorldCtx,
}

impl World {
    pub(crate) fn from_raw(raw: *mut abi::WorldCtx) -> World {
        World { raw }
    }

    pub(crate) fn raw(&self) -> *mut abi::WorldCtx {
        self.raw
    }

    pub fn get(&self, pos: [i32; 3]) -> u32 {
        unsafe { (host().get_block)(self.raw, &pos) }
    }

    /// `get` that tells air from what isn't loaded here: `None` for that. A callback sees the
    /// region it runs in, so that is also anything far from the players around it.
    pub fn get_loaded(&self, pos: [i32; 3]) -> Option<u32> {
        let s = unsafe { (host().get_block_loaded)(self.raw, &pos) };
        (s != u32::MAX).then_some(s)
    }

    /// The id of this dimension (`Dimension::id`).
    pub fn dimension(&self) -> u32 {
        unsafe { (host().world_dimension)(self.raw) }
    }

    /// False if the position is not loaded.
    pub fn set(&self, pos: [i32; 3], state: u32) -> bool {
        unsafe { (host().set_block)(self.raw, &pos, state) != 0 }
    }

    /// Every block whose middle is within `radius` of `center` becomes `state` (air for a
    /// crater), at the end of the tick and across the whole dimension: loaded cubes at once on
    /// every core, the rest when they load. Far faster than `set` per block for anything big.
    /// `keep_unbreakable`: blocks with negative hardness stay.
    pub fn fill_sphere(&self, center: [f64; 3], radius: f64, state: u32, keep_unbreakable: bool) {
        let flags = if keep_unbreakable { abi::area_flags::KEEP_UNBREAKABLE } else { 0 };
        let desc = abi::AreaFillDesc { center, radius, state, flags };
        unsafe { (host().area_fill)(self.raw, &desc) }
    }

    /// `fill_sphere` choosing what stays (`abi::area_flags`).
    pub fn fill_sphere_keeping(&self, center: [f64; 3], radius: f64, state: u32, flags: u32) {
        let desc = abi::AreaFillDesc { center, radius, state, flags };
        unsafe { (host().area_fill)(self.raw, &desc) }
    }

    /// A block of moving structure `entity` (in its own coordinates; block (0, 0, 0) is centered
    /// on the entity); `None` if the entity is not one.
    pub fn structure_block(&self, entity: u64, pos: [i32; 3]) -> Option<u32> {
        let s = unsafe { (host().structure_block)(self.raw, entity, &pos) };
        (s != u32::MAX).then_some(s)
    }

    /// Sets a block of moving structure `entity`, like `set` in the world.
    pub fn set_structure_block(&self, entity: u64, pos: [i32; 3], state: u32) -> bool {
        unsafe { (host().set_structure_block)(self.raw, entity, &pos, state) != 0 }
    }

    /// Takes `cells` out of the world into a new moving structure, whose block (0, 0, 0) is at
    /// `center` and which they keep their places around. Chests and the like go along with
    /// what is in them, and it stands there till told otherwise: `set_entity` for a speed,
    /// `body_go` to go somewhere, `put_down` to be blocks again. Its entity, or 0 if nothing came.
    pub fn lift(&self, center: [i32; 3], cells: &[[i32; 3]]) -> u64 {
        unsafe { (host().lift_blocks)(self.raw, &center, cells.as_ptr(), cells.len()) }
    }

    /// A moving structure goes back into the world at the nearest whole block. What found its
    /// place taken drops.
    pub fn put_down(&self, entity: u64) {
        unsafe { (host().put_down_body)(self.raw, entity) }
    }

    /// A moving structure goes to `pos` in exactly `ticks`.
    pub fn body_go(&self, entity: u64, pos: [f64; 3], ticks: u32) {
        unsafe { (host().body_go)(self.raw, entity, &pos, ticks) }
    }

    /// What a comparator reads of the block at `pos` (`signal_read`) may be different now.
    pub fn signal_read_changed(&self, pos: [i32; 3]) {
        unsafe { (host().signal_read_changed)(self.raw, &pos) }
    }

    pub fn schedule(&self, pos: [i32; 3], delay_ticks: u32) {
        unsafe { (host().schedule_tick)(self.raw, &pos, delay_ticks) }
    }

    pub fn time(&self) -> u64 {
        unsafe { (host().world_time)(self.raw) }
    }

    pub fn random(&self) -> u64 {
        unsafe { (host().random)(self.raw) }
    }

    /// Uniform in `0..n`.
    pub fn random_below(&self, n: u32) -> u32 {
        ((self.random() >> 32) * n as u64 >> 32) as u32
    }

    pub fn flags(&self, pos: [i32; 3]) -> u32 {
        state_flags(self.get(pos))
    }

    /// The strongest redstone power coming into a cell, 0..15.
    pub fn signal(&self, pos: [i32; 3]) -> u8 {
        unsafe { (host().signal_at)(self.raw, &pos) as u8 }
    }

    /// The value of property `name` of the block at `pos`: `w.prop(pos, "lit")`.
    pub fn prop(&self, pos: [i32; 3], name: &str) -> Option<&'static str> {
        state_prop(self.get(pos), name)
    }

    /// Changes one property of the block at `pos`, keeping the others (its facing stays):
    /// `w.set_prop(pos, "lit", "true")`. False if the block has no such property or value.
    pub fn set_prop(&self, pos: [i32; 3], name: &str, value: &str) -> bool {
        match state_with(self.get(pos), name, value) {
            Some(s) => {
                self.set(pos, s);
                true
            }
            None => false,
        }
    }
}

/// The value of property `name` in `state` (`None` if its block has no such property).
pub fn state_prop(state: u32, name: &str) -> Option<&'static str> {
    let v = unsafe { (host().state_prop)(state, Str::new(name)).as_str() };
    (!v.is_empty()).then_some(v)
}

/// `state` with property `name` set to `value`, its other properties kept.
pub fn state_with(state: u32, name: &str, value: &str) -> Option<u32> {
    let s = unsafe { (host().state_with)(state, Str::new(name), Str::new(value)) };
    (s != u32::MAX).then_some(s)
}

pub fn state_flags(state: u32) -> u32 {
    unsafe { (host().state_flags)(state) }
}

pub fn block_of(state: u32) -> u32 {
    unsafe { (host().block_of)(state) }
}

/// Full name of a block state (`base:furnace[facing=north,lit=false]`), item, entity kind ...
/// by its id; `kind` is `abi::entry_kind`. Empty if unknown.
pub fn name_of(kind: u32, id: u32) -> &'static str {
    unsafe { (host().name_of)(kind, id).as_str() }
}

pub fn block_name(state: u32) -> &'static str {
    name_of(abi::entry_kind::BLOCK, state)
}

pub fn item_name(item: u32) -> &'static str {
    name_of(abi::entry_kind::ITEM, item)
}


