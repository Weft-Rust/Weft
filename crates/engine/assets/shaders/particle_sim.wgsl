// Particles on the GPU: every live one moved a frame, stopped by what `cells` says of the blocks
// around the camera, and written out for `particle.wgsl` to draw. The prelude
// (`gpu_layout::particle_sim_prelude`) declares the bindings and `args`.

const THROUGH: u32 = 65536u;
const ON_GROUND: u32 = 131072u;
const STOPPED: u32 = 262144u;
// Vanilla's box: 0.2 across and 0.2 up from where it is.
const BOX_LOW: vec3<f32> = vec3<f32>(-0.1, 0.0, -0.1);
const BOX_HIGH: vec3<f32> = vec3<f32>(0.1, 0.2, 0.1);

// A block's word: light (a nibble each: sky, red, green, blue), then what is solid of it from
// bottom to top in sixteenths. What is not in the window is nothing, under the open sky.
fn cell_word(c: vec3<i32>) -> u32 {
    let cube = c >> vec3<u32>(4u);
    let far = abs(cube - args.center.xyz);
    if far.x > WINDOW_REACH || far.y > WINDOW_REACH || far.z > WINDOW_REACH {
        return 15u;
    }
    let side = WINDOW_REACH * 2 + 1;
    let at = ((cube % side) + side) % side;
    let slot = u32(at.x + side * (at.y + side * at.z));
    let local = c & vec3<i32>(15);
    return cells[slot * 4096u + u32(local.x + 16 * (local.z + 16 * local.y))];
}

// How far up and from where a block is solid, in blocks: empty when bottom is not under top.
fn solid_span(word: u32) -> vec2<f32> {
    return vec2<f32>(f32((word >> 16u) & 31u), f32((word >> 21u) & 31u)) / 16.0;
}

// Whether the box around `p` overlaps anything solid (`p` from the origin).
fn overlaps(p: vec3<f32>) -> bool {
    let lo = vec3<i32>(floor(p + BOX_LOW));
    let hi = vec3<i32>(floor(p + BOX_HIGH));
    for (var z = lo.z; z <= hi.z; z++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var x = lo.x; x <= hi.x; x++) {
                let s = solid_span(cell_word(vec3(x, y, z) + args.origin.xyz));
                if s.y > s.x && p.y + BOX_HIGH.y > f32(y) + s.x && p.y + BOX_LOW.y < f32(y) + s.y {
                    return true;
                }
            }
        }
    }
    return false;
}

// How far the box around `p` gets along `axis` of the distance `d`, up to what stops it.
fn sweep(p: vec3<f32>, axis: u32, d: f32) -> f32 {
    var to = p;
    to[axis] = p[axis] + d;
    let lo = vec3<i32>(floor(min(p, to) + BOX_LOW));
    let hi = vec3<i32>(floor(max(p, to) + BOX_HIGH));
    let box_low = p + BOX_LOW;
    let box_high = p + BOX_HIGH;
    var left = d;
    for (var z = lo.z; z <= hi.z; z++) {
        for (var y = lo.y; y <= hi.y; y++) {
            for (var x = lo.x; x <= hi.x; x++) {
                let s = solid_span(cell_word(vec3(x, y, z) + args.origin.xyz));
                if s.y <= s.x {
                    continue;
                }
                let low = vec3<f32>(f32(x), f32(y) + s.x, f32(z));
                let high = vec3<f32>(f32(x + 1), f32(y) + s.y, f32(z + 1));
                // Beside it on the other two axes: never in its way.
                var beside = false;
                for (var i = 0u; i < 3u; i++) {
                    if i != axis && (box_high[i] <= low[i] || box_low[i] >= high[i]) {
                        beside = true;
                    }
                }
                if beside {
                    continue;
                }
                if d > 0.0 && box_high[axis] <= low[axis] + 1e-4 {
                    left = min(left, max(low[axis] - box_high[axis], 0.0));
                } else if d < 0.0 && box_low[axis] >= high[axis] - 1e-4 {
                    left = max(left, min(high[axis] - box_low[axis], 0.0));
                }
            }
        }
    }
    return left;
}

fn mix_color(a: u32, b: u32, t: f32) -> u32 {
    var out = 0u;
    for (var s = 0u; s < 32u; s += 8u) {
        let x = f32((a >> s) & 255u);
        let y = f32((b >> s) & 255u);
        out |= u32(round(x + (y - x) * t)) << s;
    }
    return out;
}

@compute @workgroup_size(256)
fn simulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= args.count {
        return;
    }
    let i = args.first + id.x;
    var p = sim[i];
    let life = p.life.x;
    var age = p.vel_age.w;
    var position = p.pos_size.xyz + args.shift.xyz;
    if age >= life {
        dst[i].pos_size = vec4(0.0);
        sim[i].pos_size = vec4(position, p.pos_size.w);
        return;
    }
    var flags = p.data.x;
    let kind = kinds[flags & 65535u];
    let dt = args.dt;
    age += dt;
    var vel = p.vel_age.xyz;
    vel.y -= kind.motion.x * dt;
    vel *= kind.motion.y;
    // At most a block a step, the most `sweep` looks ahead.
    let delta = clamp(vel * dt, vec3(-1.0), vec3(1.0));
    if (flags & THROUGH) != 0u {
        position += delta;
        if !overlaps(position) {
            flags &= ~THROUGH;
        }
    } else if (kind.data.z & PARTICLE_FLAG_COLLIDE) != 0u && (flags & STOPPED) == 0u {
        let was_on_ground = (flags & ON_GROUND) != 0u;
        var hit = vec3<bool>(false);
        for (var n = 0u; n < 3u; n++) {
            // Up and down first, as anything else in the world moves.
            let axis = (n + 1u) % 3u;
            if delta[axis] == 0.0 {
                continue;
            }
            let moved = sweep(position, axis, delta[axis]);
            position[axis] += moved;
            // As in vanilla: one that can't go up or down any more stays where it is for good.
            if axis == 1u && abs(delta.y) >= 1e-5 && abs(moved) < 1e-5 {
                flags |= STOPPED;
            }
            if moved != delta[axis] {
                hit[axis] = true;
                vel[axis] = 0.0;
            }
        }
        let on_ground = (hit.y && delta.y < 0.0) || (was_on_ground && hit.y);
        flags = (flags & ~ON_GROUND) | select(0u, ON_GROUND, on_ground);
        if on_ground {
            if (kind.data.z & PARTICLE_FLAG_DIES_ON_LAND) != 0u {
                age = life;
            }
            vel.x *= kind.motion.z;
            vel.z *= kind.motion.z;
        }
    } else if (kind.data.z & PARTICLE_FLAG_COLLIDE) == 0u {
        position += delta;
    }
    sim[i].pos_size = vec4(position, p.pos_size.w);
    sim[i].vel_age = vec4(vel, age);
    sim[i].data.x = flags;

    let t = clamp(age / life, 0.0, 1.0);
    let size = select(p.pos_size.w, p.pos_size.w * (1.0 - t), (kind.data.z & PARTICLE_FLAG_SHRINK) != 0u);
    // A kind's frames over its life, or the block a piece of dust is of.
    var texture = p.data.z;
    var uv = p.data.w;
    if kind.frames.y > 0u {
        texture = frames[kind.frames.x + min(u32(t * f32(kind.frames.y)), kind.frames.y - 1u)];
        uv = 0xFFFF0000u;
    }
    var light = 0xFFFFFFFFu;
    if (kind.data.z & PARTICLE_FLAG_FULLBRIGHT) == 0u {
        // The middle of its box: on a floor, rounding must not put it in the floor's dark.
        let w = cell_word(vec3<i32>(floor(position + vec3(0.0, 0.1, 0.0))) + args.origin.xyz);
        light = (w & 15u) | (((w >> 4u) & 15u) << 8u) | (((w >> 8u) & 15u) << 16u) | (((w >> 12u) & 15u) << 24u);
    }
    let color = mix_color(kind.data.x, kind.data.y, t);
    let tint = p.data.y;
    let tinted = (((((color >> 24u) & 255u) * ((tint >> 16u) & 255u)) / 255u) << 24u)
        | (((((color >> 16u) & 255u) * ((tint >> 8u) & 255u)) / 255u) << 16u)
        | (((((color >> 8u) & 255u) * (tint & 255u)) / 255u) << 8u)
        | (color & 255u);
    dst[i].pos_size = vec4(position + args.rel.xyz, size);
    dst[i].data = vec4(tinted, (texture & PARTICLE_TEX_MASK) | (kind.data.w << PARTICLE_SLOT_SHIFT), uv, light);
    dst[i].extra = vec4(t, 0.0, 0.0, 0.0);
}
