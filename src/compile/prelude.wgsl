const PI: f32 = 3.14159265359;

// The vertex stage every Output shares: one triangle over the whole target, from its index.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

// GLSL's `mod`: x - y * floor(x / y), which follows the sign of y. WGSL's `%` truncates
// instead, and differs from it wherever x / y is negative.
fn floor_mod(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }
fn floor_mod2(x: vec2f, y: vec2f) -> vec2f { return x - y * floor(x / y); }
fn floor_mod3(x: vec3f, y: vec3f) -> vec3f { return x - y * floor(x / y); }
fn floor_mod4(x: vec4f, y: vec4f) -> vec4f { return x - y * floor(x / y); }

// A clock's whole part modulo a whole period, in integers. floor_mod would give the same
// number, but Metal compiles with fast math, which may regroup floor_mod(whole, n) + fraction
// as (whole + fraction) - ..., adding the fraction to the large whole part first and losing
// it; an integer remainder is a value no float sum can be folded into.
fn whole_mod(whole: f32, n: f32) -> f32 {
    let m = i32(n);
    return f32(((i32(whole) % m) + m) % m);
}

// Two numbers from a point, 0 to 1, for a measurement's jitter. Deterministic in its
// argument: the same cell and the same moment give the same offset.
//
// Dave Hoskins' "Hash without Sine" (Shadertoy 4djSRW), MIT; see licenses/hash-without-sine.txt.
fn hash2(p: vec2f) -> vec2f {
    var q = fract(p.xyx * vec3f(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.xx + q.yz) * q.zy);
}

fn hsv2rgb2(c: vec3f) -> vec3f {
    let K = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3f(0.0), vec3f(1.0)), c.y);
}

fn defaultUvMap(uv: vec2f) -> vec4f {
    let hue = (atan2(uv.y, uv.x) / (2.0 * 3.14159)) + 0.5;
    let saturation = clamp(length(uv) * 0.7, 0.0, 1.0);
    let value = 0.95;
    return vec4f(hsv2rgb2(vec3f(hue, saturation, value)), 1.0);
}
