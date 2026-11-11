//! Furnace and chest: blocks with an entity holding their inventory and state.

use weft_sdk::*;

use crate::blocks::{CHEST, FURNACE};
use crate::items::FUEL;

pub use base_api::{BURNER, Burner, WORKER, Worker};
pub static FURNACE_MENU: Menu = Menu::named("furnace");
pub static CHEST_MENU: Menu = Menu::named("chest");

pub fn register(r: &mut Registry) {
    r.entity_component(&BURNER, Burner::default(), component_flags::SAVE | component_flags::SYNC_MENU);
    r.entity_component(&WORKER, Worker::default(), component_flags::SAVE | component_flags::SYNC_MENU);
    // The components sync to open menus; screens bind to them.
    // Each kind ticks as a system over its own state: every machine of a region in one call.
    r.system(&BURNER, tick_burners);
    r.system(&WORKER, tick_workers);
    // The furnace's state syncs to whoever has it open: the screen binds to it directly.
    // The furnace is a combination burner and worker with no energy input or output
    r.block(&FURNACE)
        // Front towards the player; unlit and lit.
        .sound_group("stone")
        .facing(Facings::HORIZONTAL, Toward::Player)
        .prop("active", ["false", "true"])
        .states(|p, s| {
            let lit = p.is("active", "true");
            let s = s
                .texture("furnace_side")
                .ends("furnace_top")
                .front(if lit { "furnace_front_lit" } else { "furnace_front" });
            if lit { s.light_rgb(13, 11, 8) } else { s }
        })
        .hardness(3.5)
        .mined_by("pickaxe", 0)
        .tab(&crate::tabs::FUNCTIONAL)
        .entity(|e| {
            e.attach(&BURNER)
                .attach_with(&ENERGY, Energy { stored: 0, capacity: 2_000, max_in: 0, max_out: 0 })
                .attach(&WORKER)
                .inventory(3)
        })
        .opens(&FURNACE_MENU);
    r.menu(&FURNACE_MENU)
        .group("input", Slots::single().holder())
        .group(
            "fuel",
            Slots::single()
                .accepts("#fuel")
                .background("slot_fuel")
                .holder(),
        )
        .group("output", Slots::single().output().holder())
        .recipes(&crate::recipes::SMELTING)
        .player_inventory()
        .screen(|ui| {
            ui.panel("panel", S::column().pad(7.0).gap(4.0).stretch(), |ui| {
                ui.title();
                ui.row(S::row().gap(8.0), |ui| {
                    ui.spacer();
                    ui.column(S::column().gap(2.0), |ui| {
                        ui.group("input");
                        ui.progress_bind_dir("flame", S::default().size(14.0, 14.0), abi::ui_dir::UP, |c| {
                            c.menu_component(&BURNER)
                                .map_or(0.0, |s| ratio(s.burn, s.burn_max))
                        });
                        ui.group("fuel");
                    });
                    ui.recipe_arrow_bind(|c| {
                        c.menu_component(&WORKER)
                            .map_or(0.0, |s| ratio(s.progress, s.progress_max))
                    });
                    ui.group("output");
                    ui.spacer();
                });
                ui.player_inventory(true);
            });
        });

    r.block(&CHEST)
        .sound_group("wood")
        .facing(Facings::HORIZONTAL, Toward::Player)
        .states(|_, s| {
            s.texture("chest_side")
                .ends("chest_top")
                .front("chest_front")
                .model_box([1.0, 0.0, 1.0], [15.0, 14.0, 15.0])
        })
        .hardness(2.5)
        .mined_by("axe", tool_tier::HAND)
        .tab(&crate::tabs::FUNCTIONAL)
        .entity(|e| e.inventory(27))
        .opens(&CHEST_MENU);
    r.menu(&CHEST_MENU)
        .group("chest", Slots::chest(3).holder())
        .default_screen();
}

const PER_TICK: i64 = 40;
const FURNACE_USE: i64 = 20;

pub fn tick_burners(b: &mut SystemBatch<Burner>) {
    let w = b.world();
    for (id, m, ticks) in b.iter_mut() {
        let Some(e) = w.component(id, &ENERGY) else { continue };
        let room = e.capacity - e.stored;
        if ticks > 1 {
            // Catching up: it runs while there is room for what it makes, lighting fuel as it
            // goes (the network takes from it between parts of the catch-up).
            let mut want = (ticks as i64).min(room / PER_TICK).max(0) as u32;
            let mut ran = want.min(m.burn);
            m.burn -= ran;
            want -= ran;
            let slot = w.inventory(id).group("fuel");
            while want > 0 {
                let fuel = slot.get(0);
                let Some(t) = FUEL.on_stack_item(&fuel) else { break };
                (m.burn, m.burn_max) = (t, t);
                slot.set(0, consume_one(fuel));
                let r = want.min(m.burn);
                m.burn -= r;
                ran += r;
                want -= r;
            }
            if let Some(mut e) = w.component(id, &ENERGY) {
                e.stored = (e.stored + ran as i64 * PER_TICK).min(e.capacity);
                w.set_component(id, &ENERGY, e);
            }
            continue;
        }
        if m.burn == 0 && room >= PER_TICK {
            let slot = w.inventory(id).group("fuel");
            let fuel = slot.get(0);
            // Any mod's fuel works: the value is the common `fuel` component.
            if let Some(ticks) = FUEL.on_item(fuel.item).filter(|_| !fuel.is_empty()) {
                m.burn = ticks;
                m.burn_max = ticks;
                // A lava bucket leaves its bucket, like in the furnace.
                slot.set(0, consume_one(fuel));
            }
        }
        if m.burn > 0 {
            m.burn -= 1;
            if let Some(mut e) = w.component(id, &ENERGY) {
                e.stored = (e.stored + PER_TICK).min(e.capacity);
                w.set_component(id, &ENERGY, e);
            }
        }
    }
}

pub fn tick_workers(b: &mut SystemBatch<Worker>) {
    let w = b.world();
    for (id, m, ticks) in b.iter_mut() {
        let Some(e) = w.component(id, &ENERGY) else { continue };
        let stored = e.stored;
        let inv = w.inventory(id);
        let (input_slot, output_slot) = (inv.group("input"), inv.group("output"));
        let (input, output) = (input_slot.get(0), output_slot.get(0));
        let recipe = (!input.is_empty())
            .then(|| recipe_process(&crate::recipes::SMELTING, input.item))
            .flatten();
        let fits = recipe.is_some_and(|(out, _)| output.room_for(out) > 0);
        if !fits || stored < FURNACE_USE {
            m.progress = 0;
            continue;
        }
        let (out, time) = recipe.unwrap();
        // Twice as fast as the base furnace, paid for in energy.
        m.progress_max = (time / 2).max(1);
        if ticks > 1 {
            // Catching up: as many steps as the energy, the input and the room allow.
            let work = (input.count.min(output.room_for(out)) * m.progress_max).saturating_sub(m.progress) as i64;
            let steps = (ticks as i64).min(stored / FURNACE_USE).min(work);
            let total = m.progress + steps as u32;
            let done = total / m.progress_max;
            m.progress = total % m.progress_max;
            if let Some(mut e) = w.component(id, &ENERGY) {
                e.stored -= steps * FURNACE_USE;
                w.set_component(id, &ENERGY, e);
            }
            if done > 0 {
                input_slot.set(0, input.shrink(done));
                output_slot.set(0, output.merged(out, done));
            }
            continue;
        }
        m.progress += 1;
        let done = m.progress >= m.progress_max;
        if done {
            m.progress = 0;
        }
        if let Some(mut e) = w.component(id, &ENERGY) {
            e.stored -= FURNACE_USE;
            w.set_component(id, &ENERGY, e);
        }
        if done {
            input_slot.set(0, input.shrink(1));
            output_slot.set(0, output.merged(out, 1));
        }
    }
}
