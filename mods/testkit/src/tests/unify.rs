//! Rules that used to be written twice, each by what a player or a mod sees.

use std::time::Duration;

use weft_sdk::*;

use super::{add, at, probe};
use crate::runner::{R, T};

/// `w.push` does what it says for a player and a mob alike, however strong.
pub async fn push_is_the_same_for_all(t: T) -> R {
    t.flat().await?;
    t.cmd("/rule core:mob_spawning 0").await?;
    let y = t.state().pos[1];
    t.op("push 0 20 0").await?;
    let mut top = y;
    for _ in 0..30 {
        t.secs(0.05).await?;
        top = top.max(t.state().pos[1]);
    }
    let rose = top - y;
    t.note(format!("the player rose {rose:.2}"));
    t.check(rose > 3.0, format!("20 blocks a second up isn't clamped to a knock's 8: rose {rose:.2}"));
    let f = add(t.feet(), [4, 0, 0]);
    let pig = t.op(&format!("spawn base:pig {}", at(f))).await?;
    t.secs(0.5).await?;
    // Its flight is the same step a player's is; sampled from here its top can be missed.
    t.op(&format!("push 0 20 0 {}", pig.trim())).await?;
    let speed: f64 = t.op(&format!("vel_y {}", pig.trim())).await?.trim().parse().unwrap_or(0.0);
    t.note(format!("the pig went at {speed:.1}"));
    t.check((15.0..=21.0).contains(&speed), format!("a pig is pushed as hard as a player: {speed:.1}"));
    Ok(())
}

/// What is in a cell with something else counts: water by a slab ends a fall.
pub async fn water_beside_a_slab_ends_a_fall(t: T) -> R {
    t.flat().await?;
    t.until("full health", Duration::from_secs(5), |t| t.state().health >= 20.0)
        .await?;
    let f = t.feet();
    t.set(f, "base:oak_planks_slab").await?;
    t.op(&format!("addpart {} water", at(f))).await?;
    t.check(t.op(&format!("parts {}", at(f))).await?.contains("water"), "water in the cell");
    t.c().teleport([f[0] as f64 + 0.5, f[1] as f64 + 12.0, f[2] as f64 + 0.5]);
    t.until("falling", Duration::from_secs(5), |t| !t.flag(client::client_flags::ON_GROUND))
        .await?;
    t.until("landed", Duration::from_secs(10), |t| t.flag(client::client_flags::ON_GROUND))
        .await?;
    t.secs(0.3).await?;
    t.check(t.state().health >= 20.0, format!("no fall damage in the water: {}", t.state().health));
    Ok(())
}

/// A jump is what the client says it is: being thrown up isn't one.
pub async fn thrown_up_is_no_jump(t: T) -> R {
    t.flat().await?;
    let jumps = |t: T| async move { Ok::<u32, String>(t.op("jumps").await?.trim().parse().unwrap_or(0)) };
    let before = jumps(t.clone()).await?;
    t.op("push 0 10 0").await?;
    t.secs(1.5).await?;
    t.check_eq(jumps(t.clone()).await?, before, "a push spends nothing");
    t.hold("core:jump", 0.1);
    t.secs(1.2).await?;
    let after = jumps(t.clone()).await?;
    t.check(after > before, format!("a real jump counts: {before} -> {after}"));
    Ok(())
}

/// Mining adds up by what holds at each tick: jumping near the end only slows it down.
pub async fn jumping_while_mining(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let dirt = add(f, [2, 0, 0]);
    t.set(dirt, "dirt").await?;
    t.sync_blocks(dirt, dirt).await?;
    t.look_at(dirt);
    t.frames(2).await?;
    // By hand 0.75 s on the ground, five times as slow in the air.
    t.hold("core:attack", 4.0);
    t.secs(0.55).await?;
    t.hold("core:jump", 0.1);
    let gone = t
        .until("mined", Duration::from_secs(4), |t| t.c().block(dirt) == Some(0))
        .await;
    t.check(gone.is_ok(), "it comes off in the end");
    t.secs(0.4).await?;
    t.check_eq(t.block(dirt).await?, "core:air".into(), "and the server agrees");
    t.check_eq(t.c().block(dirt), Some(0), "the client wasn't given it back");
    Ok(())
}

/// In creative what can't be broken isn't shown as broken for a moment, again and again.
pub async fn unbreakable_in_creative(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    let f = t.feet();
    let rock = add(f, [2, 0, 0]);
    t.set(rock, "testkit:unbreakable").await?;
    t.sync_blocks(rock, rock).await?;
    let state = t.c().block(rock);
    t.look_at(rock);
    t.frames(2).await?;
    t.hold("core:attack", 1.0);
    for _ in 0..40 {
        t.frames(1).await?;
        t.check_eq(t.c().block(rock), state, "still there");
        if t.failed() {
            break;
        }
    }
    Ok(())
}

/// The client knows how fast the server runs: mining isn't told off at a slow tick rate.
pub async fn mining_at_a_slow_tick_rate(t: T) -> R {
    t.flat().await?;
    t.cmd("/tick rate 8").await?;
    t.secs(0.2).await?;
    let f = t.feet();
    let dirt = add(f, [2, 0, 0]);
    t.set(dirt, "dirt").await?;
    t.sync_blocks(dirt, dirt).await?;
    t.look_at(dirt);
    t.frames(2).await?;
    // 15 ticks of mining, which at 8 a second take almost two seconds.
    t.hold("core:attack", 3.5);
    let gone = t
        .until("mined", Duration::from_secs(4), |t| t.c().block(dirt) == Some(0))
        .await;
    t.check(gone.is_ok(), "mined at 8 ticks a second");
    t.secs(0.5).await?;
    let server = t.block(dirt).await?;
    t.cmd("/tick rate 20").await?;
    t.check_eq(server, "core:air".into(), "the server agrees");
    Ok(())
}

/// A virtual inventory lives until removed, wherever the player goes.
pub async fn virtual_inventory_stays(t: T) -> R {
    t.flat().await?;
    let id = t.op("vmenu base:chest 27 diamond 3").await?;
    t.screen("core:menu").await?;
    t.c().close_screen();
    t.no_screen().await?;
    let back = t.state().pos;
    t.c().teleport([back[0] + 1500.0, back[1], back[2]]);
    t.secs(2.0).await?;
    t.c().teleport(back);
    t.secs(1.0).await?;
    let inv = t.op(&format!("inv {}", id.trim())).await?;
    t.check(inv.contains("diamond"), format!("its things are still there: {inv}"));
    let near = t.op(&format!("near {} 16", at(t.feet()))).await?;
    t.check(!near.contains("core:virtual"), format!("and nothing around sees it: {near}"));
    t.op(&format!("remove {}", id.trim())).await?;
    Ok(())
}

/// Spectators don't nudge what they pass through.
pub async fn spectators_push_nothing(t: T) -> R {
    t.flat().await?;
    t.cmd("/rule core:mob_spawning 0").await?;
    t.op("mode spectator").await?;
    let f = t.feet();
    let idol = t.op(&format!("spawn testmod:idol {}", at(f))).await?;
    t.secs(0.4).await?;
    let x0 = t.op(&format!("entity {}", idol.trim())).await?;
    t.secs(1.0).await?;
    let x1 = t.op(&format!("entity {}", idol.trim())).await?;
    let xz = |s: &str| {
        let w: Vec<f64> = s.split_whitespace().skip(1).take(3).filter_map(|v| v.parse().ok()).collect();
        (w[0], w[2])
    };
    let (a, b) = (xz(&x0), xz(&x1));
    t.check(((a.0 - b.0).abs() + (a.1 - b.1).abs()) < 0.05, format!("it stayed: {a:?} -> {b:?}"));
    t.op(&format!("remove {}", idol.trim())).await?;
    Ok(())
}

/// A tinted plant is tinted in the hand as in a slot and in the world.
pub async fn tinted_in_the_hand(t: T) -> R {
    t.flat().await?;
    t.c().look(0.0, 60.0);
    t.op("put me 0 testkit:tinted_plant 1").await?;
    t.c().select(0);
    t.secs(0.8).await?;
    let shot = t.shot("fern_in_hand").await?;
    let green = shot.share(0.55, 0.55, 1.0, 0.94, probe::grass);
    t.probe("green in the hand", green, green > 0.005);
    Ok(())
}

/// A swing is spent whatever it hits, on both sides, and another of the same item in hand doesn't
/// start it over.
pub async fn swings_count_on_both_sides(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 iron_sword 1").await?;
    t.op("put me 1 iron_sword 1").await?;
    t.c().select(0);
    let f = t.feet();
    let spot = add(f, [0, 0, -2]);
    for (from, to) in [([-1, 0, -3], [-1, 2, 1]), ([1, 0, -3], [1, 2, 1]), ([0, 0, -3], [0, 2, -3]), ([0, 0, 1], [0, 2, 1])] {
        t.fill(add(f, from), add(f, to), "stone").await?;
    }
    let id = t
        .op(&format!("spawn base:cow {} {} {}", spot[0], spot[1], spot[2]))
        .await?;
    let health = |t: T, id: String| async move {
        let s = t.op(&format!("entity {id}")).await?;
        Ok::<f32, String>(s.split_whitespace().last().and_then(|v| v.parse().ok()).unwrap_or(-1.0))
    };
    t.secs(1.5).await?;
    // Up at the sky: it hits nothing, and leaves the swing spent.
    t.c().look(0.0, 89.0);
    t.frames(2).await?;
    t.press("core:attack");
    t.frames(2).await?;
    t.look_at(spot);
    t.frames(1).await?;
    let before = health(t.clone(), id.clone()).await?;
    t.press("core:attack");
    t.secs(0.15).await?;
    let weak = before - health(t.clone(), id.clone()).await?;
    t.note(format!("right after a swing at nothing: {weak:.2}"));
    t.check((0.1..3.0).contains(&weak), format!("a swing just spent does little: {weak:.2}"));
    t.secs(1.5).await?;
    // The same sword from the next slot keeps the charge.
    t.c().select(1);
    t.frames(4).await?;
    let before = health(t.clone(), id.clone()).await?;
    t.press("core:attack");
    t.secs(0.15).await?;
    let full = before - health(t.clone(), id.clone()).await?;
    t.note(format!("after changing to the same sword: {full:.2}"));
    t.check(full > 5.5, format!("the same item changes nothing: {full:.2}"));
    Ok(())
}

/// Thrown from a menu or by the key, an item flies the same way and waits the same two seconds.
pub async fn thrown_from_a_menu(t: T) -> R {
    t.flat().await?;
    t.c().look(0.0, 0.0);
    t.op("put me 9 cobblestone 1").await?;
    t.frames(2).await?;
    t.press("core:inventory");
    t.screen("core:menu").await?;
    t.settled().await?;
    t.check(t.click_ui("slot:9", 0), "took it");
    t.frames(3).await?;
    let y = t.state().pos[1];
    t.c().cursor(0.01, 0.01);
    t.frames(2).await?;
    t.c().click(0, 0.1);
    t.until("thrown", Duration::from_secs(3), |t| t.c().entities().iter().any(|&e| t.c().entity(e).is_some()))
        .await?;
    t.c().close_screen();
    t.no_screen().await?;
    // Off with it, and over the time an item merely dropped would be back.
    t.secs(1.0).await?;
    t.check_eq(t.c().inventory_slot(9).count, 0, "not picked up after a second");
    let e = t.c().entities().first().copied().ok_or("no item")?;
    let at = t.c().entity(e).ok_or("no item")?.pos;
    let (dx, dz) = (at[0] - t.state().pos[0], at[2] - t.state().pos[2]);
    t.check((dx * dx + dz * dz).sqrt() > 1.0, format!("it flew off where the player looks: {dx:.2} {dz:.2}, feet at {y:.2}"));
    Ok(())
}

/// `creative` means creative: a spectator is exempt, but isn't that.
pub async fn game_modes_for_mods(t: T) -> R {
    t.flat().await?;
    t.check_eq(t.op("modes").await?, "0 0 0".to_string(), "survival");
    t.op("mode creative").await?;
    t.check_eq(t.op("modes").await?, "1 1 1".to_string(), "creative");
    t.op("mode spectator").await?;
    t.check_eq(t.op("modes").await?, "2 0 1".to_string(), "spectator");
    t.op("mode survival").await?;
    Ok(())
}

/// A wrong id from a mod names the mod and goes no further.
pub async fn wrong_item_ids(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let before = t.op(&format!("near {} 8", at(f))).await?;
    t.op("bad_item").await?;
    t.secs(0.3).await?;
    t.check_eq(t.op(&format!("near {} 8", at(f))).await?, before, "nothing came of it");
    Ok(())
}

/// A bed's drop is in its loot table, for mods to see and change.
pub async fn bed_drop_in_its_table(t: T) -> R {
    t.flat().await?;
    let drops = t.op("loot base:bed 1").await?;
    t.check(drops.contains("base:bed"), format!("the table gives a bed: {drops}"));
    let foot = super::gameplay::place_bed(&t).await?;
    t.op(&format!("break {}", at(add(foot, [0, 0, -1])))).await?;
    t.secs(0.5).await?;
    t.check_eq(t.block(foot).await?, "core:air".into(), "the other half goes");
    let near = t.op(&format!("near {} 4", at(foot))).await?;
    t.check_eq(near.matches("core:item").count(), 1, "one bed, whichever half broke");
    Ok(())
}

/// A bed turned any way shows the same bed, its pillow at its head, from one picture of its top.
pub async fn beds_every_way(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    t.op("give base:bed 8").await?;
    t.c().select(0);
    let f = t.feet();
    let floor = |d: [i32; 3]| add(f, [d[0], -1, d[2]]);
    // Each looked at from the middle, away from it: its head further on.
    for (d, yaw) in [([0, 0, -2], 0.0), ([2, 0, 0], -90.0), ([0, 0, 2], 180.0), ([-2, 0, 0], 90.0)] {
        t.c().look(yaw, -45.0);
        t.look_at(floor(d));
        t.c().look(yaw, -48.0);
        t.frames(3).await?;
        t.press("core:use");
        t.secs(0.2).await?;
    }
    t.sync_blocks(add(f, [-4, 0, -4]), add(f, [4, 0, 4])).await?;
    t.c().teleport([f[0] as f64 + 0.5, f[1] as f64 + 6.0, f[2] as f64 + 0.5]);
    t.c().look(0.0, -89.0);
    t.secs(0.5).await?;
    t.shot("from_above").await?;
    for d in [[0, 0, -2], [2, 0, 0], [0, 0, 2], [-2, 0, 0]] {
        t.check(t.block(add(f, d)).await?.contains("base:bed"), format!("a bed at {d:?}: {}", t.block(add(f, d)).await?));
    }
    Ok(())
}
