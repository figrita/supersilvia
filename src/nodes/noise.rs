// SPDX-License-Identifier: AGPL-3.0-or-later

//! Noise fields: Perlin, simplex, Worley, fractal Brownian motion, and television static.
//!
//! Ported from silvia's `perlinnoise.js`, `simplex.js`, `worleynoise.js`, `fractalnoise.js`
//! and `static.js`. Each one computes a varying number and then maps it to two colors, so
//! each publishes that number: `value`, the raw 0 to 1 quantity the picture was made from.
//! It is not `mask`, which means coverage — see [decisions.md] and the vocabulary table in
//! [nodes.md].
//!
//! The classic and simplex gradient functions (`mod289`, `permute`, `taylorInvSqrt`, and the
//! 3D/4D noise bodies themselves) are Ian McEwan / Ashima Arts and Stefan Gustavson's
//! `webgl-noise`, MIT; see `licenses/webgl-noise.txt`.
//!
//! [decisions.md]: ../../../docs/decisions.md
//! [nodes.md]: ../../../docs/nodes.md
//!
//! **The gradient functions are `wgsl_utils`, not prelude.** The prelude goes whole into
//! every shader the app builds; a Perlin gradient nobody called would then be sixty lines of
//! dead WGSL in front of a checkerboard. `NodeDef::wgsl_utils` is the mechanism for this —
//! `hsla` declares its HSL conversion the same way — and the compiler emits each named block
//! once into a shader that reaches a node declaring it, and never into one that does not.
//!
//! Two departures from the source. silvia's `timeSpeed` multiplies `u_time` inside the body;
//! here each of the four with time reads **Time and Offset** in lattice cells along its time
//! axis — Static in rolls — Perlin at silvia's half a cell a second and the rest still at
//! rest, as silvia's are, so a gear cabled into Time drives the evolution and nothing jumps.
//! And silvia
//! writes each body twice, once for the color and once for the field, which is where its
//! `worleynoise` mask and color could drift apart; here the shared half is the macro's
//! `wgsl_common`.
//!
//! **Repeat** is an option on each, Never — silvia's look — or every 1, 2, 4, 8 or 16 cells
//! (Static every 4 to 128 rolls): with a length `N` the noise walks a circle of circumference
//! `N` through a four-dimensional noise, turned `(Time mod N + Offset) ÷ N` — Time's whole part
//! reduced by `N` and its fraction added — so its picture comes back every `N` and a loop
//! closes on it, at any count; Static reads its roll modulo `N`. It is an option
//! a person chooses, since a circle through four dimensions is not the line through three, and
//! it rebuilds. See `docs/nodes.md#timing`.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, Timing,
};

// ------------------------------------------------------------------------ shader helpers

/// Classic Perlin noise over three dimensions, from silvia's `perlinnoise.js`, itself
/// Stefan Gustavson's `webgl-noise`.
///
/// Its helpers carry a `perlin` prefix so a shader holding this and [`SIMPLEX3D_WGSL`] — a
/// graph with both nodes in it — declares no name twice. WGSL has no overloading, so the
/// `vec3` and `vec4` versions of `perlinMod289` are two functions with two names.
pub const PERLIN3D_WGSL: &str = "fn perlinMod289_3(x: vec3f) -> vec3f { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn perlinMod289_4(x: vec4f) -> vec4f { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn perlinPermute(x: vec4f) -> vec4f { return perlinMod289_4(((x * 34.0) + 1.0) * x); }
fn perlinTaylorInvSqrt(r: vec4f) -> vec4f { return 1.79284291400159 - 0.85373472090914 * r; }

fn cnoise3(P: vec3f) -> f32 {
    let Pi0 = perlinMod289_3(floor(P));
    let Pi1 = perlinMod289_3(floor(P) + vec3f(1.0));
    let Pf0 = fract(P);
    let Pf1 = Pf0 - vec3f(1.0);
    let ix = vec4f(Pi0.x, Pi1.x, Pi0.x, Pi1.x);
    let iy = vec4f(Pi0.yy, Pi1.yy);
    let iz0 = Pi0.zzzz;
    let iz1 = Pi1.zzzz;
    let ixy = perlinPermute(perlinPermute(ix) + iy);
    let ixy0 = perlinPermute(ixy + iz0);
    let ixy1 = perlinPermute(ixy + iz1);
    var gx0 = ixy0 * (1.0 / 7.0);
    var gy0 = fract(floor(gx0) * (1.0 / 7.0)) - 0.5;
    gx0 = fract(gx0);
    let gz0 = vec4f(0.5) - abs(gx0) - abs(gy0);
    let sz0 = step(gz0, vec4f(0.0));
    gx0 -= sz0 * (step(vec4f(0.0), gx0) - 0.5);
    gy0 -= sz0 * (step(vec4f(0.0), gy0) - 0.5);
    var gx1 = ixy1 * (1.0 / 7.0);
    var gy1 = fract(floor(gx1) * (1.0 / 7.0)) - 0.5;
    gx1 = fract(gx1);
    let gz1 = vec4f(0.5) - abs(gx1) - abs(gy1);
    let sz1 = step(gz1, vec4f(0.0));
    gx1 -= sz1 * (step(vec4f(0.0), gx1) - 0.5);
    gy1 -= sz1 * (step(vec4f(0.0), gy1) - 0.5);
    var g000 = vec3f(gx0.x, gy0.x, gz0.x);
    var g100 = vec3f(gx0.y, gy0.y, gz0.y);
    var g010 = vec3f(gx0.z, gy0.z, gz0.z);
    var g110 = vec3f(gx0.w, gy0.w, gz0.w);
    var g001 = vec3f(gx1.x, gy1.x, gz1.x);
    var g101 = vec3f(gx1.y, gy1.y, gz1.y);
    var g011 = vec3f(gx1.z, gy1.z, gz1.z);
    var g111 = vec3f(gx1.w, gy1.w, gz1.w);
    let norm0 = perlinTaylorInvSqrt(vec4f(dot(g000, g000), dot(g010, g010), dot(g100, g100), dot(g110, g110)));
    g000 *= norm0.x; g010 *= norm0.y; g100 *= norm0.z; g110 *= norm0.w;
    let norm1 = perlinTaylorInvSqrt(vec4f(dot(g001, g001), dot(g011, g011), dot(g101, g101), dot(g111, g111)));
    g001 *= norm1.x; g011 *= norm1.y; g101 *= norm1.z; g111 *= norm1.w;
    let n000 = dot(g000, Pf0);
    let n100 = dot(g100, vec3f(Pf1.x, Pf0.yz));
    let n010 = dot(g010, vec3f(Pf0.x, Pf1.y, Pf0.z));
    let n110 = dot(g110, vec3f(Pf1.xy, Pf0.z));
    let n001 = dot(g001, vec3f(Pf0.xy, Pf1.z));
    let n101 = dot(g101, vec3f(Pf1.x, Pf0.y, Pf1.z));
    let n011 = dot(g011, vec3f(Pf0.x, Pf1.yz));
    let n111 = dot(g111, Pf1);
    let fade = Pf0 * Pf0 * Pf0 * (Pf0 * (Pf0 * 6.0 - 15.0) + 10.0);
    let n_z = mix(vec4f(n000, n100, n010, n110), vec4f(n001, n101, n011, n111), fade.z);
    let n_yz = mix(n_z.xy, n_z.zw, fade.y);
    return 2.2 * mix(n_yz.x, n_yz.y, fade.x);
}";

/// Simplex noise over three dimensions, from silvia's `shaderUtils.SIMPLEX3D`, itself Ian
/// McEwan and Stefan Gustavson's `webgl-noise`.
///
/// WGSL has no overloading, so the `vec3` and `vec4` versions of `mod289_3d` are
/// `mod289_3d_v3` and `mod289_3d_v4`.
pub const SIMPLEX3D_WGSL: &str =
    "fn mod289_3d_v3(x: vec3f) -> vec3f { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn mod289_3d_v4(x: vec4f) -> vec4f { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn permute3d(x: vec4f) -> vec4f { return mod289_3d_v4(((x * 34.0) + 1.0) * x); }
fn taylorInvSqrt3d(r: vec4f) -> vec4f { return 1.79284291400159 - 0.85373472095314 * r; }

fn snoise3(v: vec3f) -> f32 {
    const C = vec2f(1.0 / 6.0, 1.0 / 3.0);
    const D = vec4f(0.0, 0.5, 1.0, 2.0);
    var i = floor(v + dot(v, C.yyy));
    let x0 = v - i + dot(i, C.xxx);
    let g = step(x0.yzx, x0.xyz);
    let l = 1.0 - g;
    let i1 = min(g.xyz, l.zxy);
    let i2 = max(g.xyz, l.zxy);
    let x1 = x0 - i1 + C.xxx;
    let x2 = x0 - i2 + C.yyy;
    let x3 = x0 - D.yyy;
    i = mod289_3d_v3(i);
    let p = permute3d(permute3d(permute3d(
        i.z + vec4f(0.0, i1.z, i2.z, 1.0))
        + i.y + vec4f(0.0, i1.y, i2.y, 1.0))
        + i.x + vec4f(0.0, i1.x, i2.x, 1.0));
    let n_ = 0.142857142857;
    let ns = n_ * D.wyz - D.xzx;
    let j = p - 49.0 * floor(p * ns.z * ns.z);
    let x_ = floor(j * ns.z);
    let y_ = floor(j - 7.0 * x_);
    let x = x_ * ns.x + ns.yyyy;
    let y = y_ * ns.x + ns.yyyy;
    let h = 1.0 - abs(x) - abs(y);
    let b0 = vec4f(x.xy, y.xy);
    let b1 = vec4f(x.zw, y.zw);
    let s0 = floor(b0) * 2.0 + 1.0;
    let s1 = floor(b1) * 2.0 + 1.0;
    let sh = -step(h, vec4f(0.0));
    let a0 = b0.xzyw + s0.xzyw * sh.xxyy;
    let a1 = b1.xzyw + s1.xzyw * sh.zzww;
    var p0 = vec3f(a0.xy, h.x);
    var p1 = vec3f(a0.zw, h.y);
    var p2 = vec3f(a1.xy, h.z);
    var p3 = vec3f(a1.zw, h.w);
    let norm = taylorInvSqrt3d(vec4f(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;
    var m = max(0.6 - vec4f(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4f(0.0));
    m = m * m;
    return 42.0 * dot(m * m, vec4f(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}";

/// Fractal Brownian motion over [`SIMPLEX3D_WGSL`], shared by `fractal` and `domainwarp`.
///
/// `shape` is 0 for the plain sum, 1 for turbulence — the octave folded about its middle —
/// and 2 for ridged, which is turbulence inverted and squared. silvia writes the same three
/// cases into both nodes by string interpolation; one function taking the case as an
/// argument is what stops the two drifting.
///
/// The octave count is a `for` bound of 8 with an early `break` rather than the loop variable
/// itself, so the trip count is a constant however the count arrives. `freq` is a parameter,
/// and WGSL's are immutable, so the loop steps a copy.
pub const FBM_WGSL: &str = "fn fbmNoise(p: vec2f, freq0: f32, octaves: i32, lacunarity: f32, gain: f32, t: f32, shape: i32) -> f32 {
    var freq = freq0;
    var sum = 0.0;
    var amp = 1.0;
    var total = 0.0;
    for (var i = 0; i < 8; i++) {
        if (i >= octaves) { break; }
        var n = snoise3(vec3f(p * freq, t + f32(i) * 0.3)) * 0.5 + 0.5;
        if (shape == 1) {
            n = abs(n * 2.0 - 1.0);
        } else if (shape == 2) {
            n = 1.0 - abs(n * 2.0 - 1.0);
            n = n * n;
        }
        sum += amp * n;
        total += amp;
        freq *= lacunarity;
        amp *= gain;
    }
    return sum / max(total, 1e-6);
}";

/// Classic Perlin noise over four dimensions, from Stefan Gustavson's `webgl-noise`
/// `classicnoise4D`: what [`PERLIN3D_WGSL`]'s line through time becomes as a circle through
/// the fourth. It calls that util's `perlinMod289_4`, `perlinPermute` and
/// `perlinTaylorInvSqrt`, which every node declaring this one also declares.
pub const PERLIN4D_WGSL: &str = "fn perlinFade4(t: vec4f) -> vec4f { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }

fn perlinGrad4(ixy: vec4f) -> array<vec4f, 4> {
    var gx = ixy * (1.0 / 7.0);
    var gy = floor(gx) * (1.0 / 7.0);
    var gz = floor(gy) * (1.0 / 6.0);
    gx = fract(gx) - 0.5;
    gy = fract(gy) - 0.5;
    gz = fract(gz) - 0.5;
    let gw = vec4f(0.75) - abs(gx) - abs(gy) - abs(gz);
    let sw = step(gw, vec4f(0.0));
    gx -= sw * (step(vec4f(0.0), gx) - 0.5);
    gy -= sw * (step(vec4f(0.0), gy) - 0.5);
    var g = array<vec4f, 4>(
        vec4f(gx.x, gy.x, gz.x, gw.x),
        vec4f(gx.y, gy.y, gz.y, gw.y),
        vec4f(gx.z, gy.z, gz.z, gw.z),
        vec4f(gx.w, gy.w, gz.w, gw.w));
    let norm = perlinTaylorInvSqrt(vec4f(dot(g[0], g[0]), dot(g[1], g[1]), dot(g[2], g[2]), dot(g[3], g[3])));
    g[0] *= norm.x;
    g[1] *= norm.y;
    g[2] *= norm.z;
    g[3] *= norm.w;
    return g;
}

fn cnoise4(P: vec4f) -> f32 {
    let Pi0 = perlinMod289_4(floor(P));
    let Pi1 = perlinMod289_4(floor(P) + vec4f(1.0));
    let Pf0 = fract(P);
    let Pf1 = Pf0 - vec4f(1.0);
    let ix = vec4f(Pi0.x, Pi1.x, Pi0.x, Pi1.x);
    let iy = vec4f(Pi0.yy, Pi1.yy);
    let ixy = perlinPermute(perlinPermute(ix) + iy);
    let ixy0 = perlinPermute(ixy + Pi0.zzzz);
    let ixy1 = perlinPermute(ixy + Pi1.zzzz);
    // Each corner group: x and y across the four lanes, z and w fixed. The lanes run
    // (x0 y0), (x1 y0), (x0 y1), (x1 y1).
    let g00 = perlinGrad4(perlinPermute(ixy0 + Pi0.wwww));
    let g01 = perlinGrad4(perlinPermute(ixy0 + Pi1.wwww));
    let g10 = perlinGrad4(perlinPermute(ixy1 + Pi0.wwww));
    let g11 = perlinGrad4(perlinPermute(ixy1 + Pi1.wwww));
    let n00 = vec4f(
        dot(g00[0], Pf0),
        dot(g00[1], vec4f(Pf1.x, Pf0.yzw)),
        dot(g00[2], vec4f(Pf0.x, Pf1.y, Pf0.zw)),
        dot(g00[3], vec4f(Pf1.xy, Pf0.zw)));
    let n10 = vec4f(
        dot(g10[0], vec4f(Pf0.xy, Pf1.z, Pf0.w)),
        dot(g10[1], vec4f(Pf1.x, Pf0.y, Pf1.z, Pf0.w)),
        dot(g10[2], vec4f(Pf0.x, Pf1.yz, Pf0.w)),
        dot(g10[3], vec4f(Pf1.xyz, Pf0.w)));
    let n01 = vec4f(
        dot(g01[0], vec4f(Pf0.xyz, Pf1.w)),
        dot(g01[1], vec4f(Pf1.x, Pf0.yz, Pf1.w)),
        dot(g01[2], vec4f(Pf0.x, Pf1.y, Pf0.z, Pf1.w)),
        dot(g01[3], vec4f(Pf1.xy, Pf0.z, Pf1.w)));
    let n11 = vec4f(
        dot(g11[0], vec4f(Pf0.xy, Pf1.zw)),
        dot(g11[1], vec4f(Pf1.x, Pf0.y, Pf1.zw)),
        dot(g11[2], vec4f(Pf0.x, Pf1.yzw)),
        dot(g11[3], Pf1));
    let fade = perlinFade4(Pf0);
    let n_0w = mix(n00, n01, fade.w);
    let n_1w = mix(n10, n11, fade.w);
    let n_zw = mix(n_0w, n_1w, fade.z);
    let n_yzw = mix(n_zw.xy, n_zw.zw, fade.y);
    return 2.2 * mix(n_yzw.x, n_yzw.y, fade.x);
}";

/// Simplex noise over four dimensions, from Ian McEwan and Stefan Gustavson's `webgl-noise`
/// `noise4D`: [`SIMPLEX3D_WGSL`]'s circle. Its helpers carry a `4d` of their own,
/// the scalar and vector versions two names each, since WGSL has no overloading.
pub const SIMPLEX4D_WGSL: &str =
    "fn mod289_4d_v4(x: vec4f) -> vec4f { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn mod289_4d_f(x: f32) -> f32 { return x - floor(x * (1.0 / 289.0)) * 289.0; }
fn permute4d_v4(x: vec4f) -> vec4f { return mod289_4d_v4(((x * 34.0) + 1.0) * x); }
fn permute4d_f(x: f32) -> f32 { return mod289_4d_f(((x * 34.0) + 1.0) * x); }
fn taylorInvSqrt4d(r: vec4f) -> vec4f { return 1.79284291400159 - 0.85373472095314 * r; }

fn grad4d(j: f32, ip: vec4f) -> vec4f {
    let pxyz = floor(fract(vec3f(j) * ip.xyz) * 7.0) * ip.z - 1.0;
    let pw = 1.5 - dot(abs(pxyz), vec3f(1.0));
    let s = select(vec4f(0.0), vec4f(1.0), vec4f(pxyz, pw) < vec4f(0.0));
    return vec4f(pxyz + (s.xyz * 2.0 - 1.0) * s.w, pw);
}

fn snoise4(v: vec4f) -> f32 {
    const F4 = 0.309016994374947451;
    const C = vec4f(0.138196601125011, 0.276393202250021, 0.414589803375032, -0.447213595499958);
    var i = floor(v + dot(v, vec4f(F4)));
    let x0 = v - i + dot(i, C.xxxx);
    // Rank sorting, Bill Licea-Kane's: i0 holds 0, 1, 2 and 3 once each.
    let isX = step(x0.yzw, x0.xxx);
    let isYZ = step(x0.zww, x0.yyz);
    var i0 = vec4f(isX.x + isX.y + isX.z, 1.0 - isX);
    i0.y += isYZ.x + isYZ.y;
    i0.z += 1.0 - isYZ.x;
    i0.w += 1.0 - isYZ.y;
    i0.z += isYZ.z;
    i0.w += 1.0 - isYZ.z;
    let i3 = clamp(i0, vec4f(0.0), vec4f(1.0));
    let i2 = clamp(i0 - 1.0, vec4f(0.0), vec4f(1.0));
    let i1 = clamp(i0 - 2.0, vec4f(0.0), vec4f(1.0));
    let x1 = x0 - i1 + C.xxxx;
    let x2 = x0 - i2 + C.yyyy;
    let x3 = x0 - i3 + C.zzzz;
    let x4 = x0 + C.wwww;
    i = mod289_4d_v4(i);
    let j0 = permute4d_f(permute4d_f(permute4d_f(permute4d_f(i.w) + i.z) + i.y) + i.x);
    let j1 = permute4d_v4(permute4d_v4(permute4d_v4(permute4d_v4(
        i.w + vec4f(i1.w, i2.w, i3.w, 1.0))
        + i.z + vec4f(i1.z, i2.z, i3.z, 1.0))
        + i.y + vec4f(i1.y, i2.y, i3.y, 1.0))
        + i.x + vec4f(i1.x, i2.x, i3.x, 1.0));
    let ip = vec4f(1.0 / 294.0, 1.0 / 49.0, 1.0 / 7.0, 0.0);
    var p0 = grad4d(j0, ip);
    var p1 = grad4d(j1.x, ip);
    var p2 = grad4d(j1.y, ip);
    var p3 = grad4d(j1.z, ip);
    var p4 = grad4d(j1.w, ip);
    let norm = taylorInvSqrt4d(vec4f(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;
    p4 *= taylorInvSqrt4d(vec4f(dot(p4, p4))).x;
    var m0 = max(0.6 - vec3f(dot(x0, x0), dot(x1, x1), dot(x2, x2)), vec3f(0.0));
    var m1 = max(0.6 - vec2f(dot(x3, x3), dot(x4, x4)), vec2f(0.0));
    m0 = m0 * m0;
    m1 = m1 * m1;
    return 49.0 * (dot(m0 * m0, vec3f(dot(p0, x0), dot(p1, x1), dot(p2, x2)))
        + dot(m1 * m1, vec2f(dot(p3, x3), dot(p4, x4))));
}";

/// [`FBM_WGSL`] on [`SIMPLEX4D_WGSL`]'s circle: each octave reads the loop's point `c`, offset
/// along the circle's first axis as the line form offsets each octave's time.
pub const FBM_LOOP_WGSL: &str = "fn fbmNoiseLoop(p: vec2f, freq0: f32, octaves: i32, lacunarity: f32, gain: f32, c: vec2f, shape: i32) -> f32 {
    var freq = freq0;
    var sum = 0.0;
    var amp = 1.0;
    var total = 0.0;
    for (var i = 0; i < 8; i++) {
        if (i >= octaves) { break; }
        var n = snoise4(vec4f(p * freq, c.x + f32(i) * 0.3, c.y)) * 0.5 + 0.5;
        if (shape == 1) {
            n = abs(n * 2.0 - 1.0);
        } else if (shape == 2) {
            n = 1.0 - abs(n * 2.0 - 1.0);
            n = n * n;
        }
        sum += amp * n;
        total += amp;
        freq *= lacunarity;
        amp *= gain;
    }
    return sum / max(total, 1e-6);
}";

/// The point a noise reads its time at on a circle `reach` long, `turn` of the way round, so
/// one turn walks it once and ends where it began: the noise moves as fast as it does on its
/// line.
pub const LOOP_CIRCLE_WGSL: &str = "fn loopCircle(turn: f32, reach: f32) -> vec2f {
    let angle = 2.0 * PI * turn;
    return reach / (2.0 * PI) * vec2f(cos(angle), sin(angle));
}";

/// One 0 to 1 number per lattice cell, from a seeded sine hash. silvia's `static.js` writes
/// this expression out five times per generator.
pub const CELL_HASH_WGSL: &str = "fn cellHash(cell: vec2f, seed: f32) -> f32 {
    return fract(sin(dot(cell, vec2f(12.9898, 78.233)) + seed) * 43758.5453);
}";

/// The two colors every noise ramps its field between.
const NOISE_COLOR: &str = "    return mix({background}, {foreground}, value);";

const NOISE_VALUE: &str = "    return value;";

/// The `shape` argument [`FBM_WGSL`] takes, from the node's `type` option.
fn fbm_shape(kind: &str) -> &'static str {
    match kind {
        "turbulence" => "1",
        "ridged" => "2",
        _ => "0",
    }
}

/// The Repeat a noise's options name, in its own units, or `None` for Never: its period.
pub fn repeat_of(node: &crate::graph::Node) -> Option<f64> {
    node.options
        .get("repeat")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|n| *n > 0.0)
}

/// A noise's timing: `pace` cells a second, coming back where its Repeat says.
pub const fn noise_timing(pace: f64) -> Timing {
    Timing::repeating(pace, repeat_of)
}

/// A Repeat option's length as a WGSL literal, or `None` for Never.
pub fn repeat_wgsl(value: &str) -> Option<String> {
    let n = value.parse::<f64>().ok().filter(|n| *n > 0.0)?;
    Some(format!("{n:?}"))
}

/// Where a noise reads its time: `t` on the line, or with a Repeat of `n`, the point `t ÷ n`
/// of the way round a circle `n` long — Time's whole part taken modulo `n` and its fraction
/// added before Offset is, so a Time of `n` is a Time of zero to the bit, at any count.
pub fn noise_time(repeat: Option<&str>) -> String {
    match repeat {
        Some(n) => format!("loopCircle(time_repeat({{clock}}, {n}, {{phaseOffset}}) / {n}, {n})"),
        None => "time_unbounded({clock}, {phaseOffset})".to_string(),
    }
}

/// The fractal sum a body calls and the time it hands it: [`FBM_WGSL`] on the line, or
/// [`FBM_LOOP_WGSL`] round the circle a Repeat names.
pub fn fbm_time(repeat: &str) -> (&'static str, String) {
    let repeat = repeat_wgsl(repeat);
    let fbm = if repeat.is_some() {
        "fbmNoiseLoop"
    } else {
        "fbmNoise"
    };
    (fbm, noise_time(repeat.as_deref()))
}

// ------------------------------------------------------------------------------ the five

node! {
    /// Gradient noise on a cubic lattice, evolving through its third dimension.
    PERLIN,
    slug: "perlin",
    icon: "☁",
    label: "Perlin Noise",
    category: Generate,
    tooltip: "Smooth gradient noise. Scale sets the lattice size and Time walks through the \
              third dimension, half a cell a second; its value is the field the colors are \
              mixed along. Repeat makes the walk a circle that comes back.",
    timing: noise_timing(0.5),
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "scale" "Scale" = Control::num_log(10.0, 0.1, 100.0, 0.1, "/⬓"),
    ],
    after_time: [
        VaryingNumber "contrast" "Contrast" = Control::num(1.0, 0.0, 5.0, 0.01, ""),
    ],
    options: [
        "repeat" "Repeat" = "never" [
            "never" => "Never",
            "1" => "Every 1",
            "2" => "Every 2",
            "4" => "Every 4",
            "8" => "Every 8",
            "16" => "Every 16",
        ],
    ],
    wgsl_utils: [PERLIN3D_WGSL, PERLIN4D_WGSL, LOOP_CIRCLE_WGSL],
    // The [-1, 1] noise is halved rather than remapped and re-centerd: (n + 1) / 2 - 1 / 2
    // is n / 2, and the contrast is a gain about the middle either way.
    wgsl_common: varying(|node, ctx| {
        let raw = match repeat_wgsl(ctx.option(node, "repeat")) {
            Some(n) => format!(
                "cnoise4(vec4f(uv * noiseScale, {}))",
                noise_time(Some(&n))
            ),
            None => format!("cnoise3(vec3f(uv * noiseScale, {}))", noise_time(None)),
        };
        format!(
            "    let noiseScale = {{scale}};
    let contrast = {{contrast}};
    let raw = {raw};
    let value = clamp(raw * 0.5 * contrast + 0.5, 0.0, 1.0);
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = NOISE_COLOR,
        VaryingNumber "value" "Value" = NOISE_VALUE,
    ],
}

node! {
    /// Gradient noise on a simplex lattice: the same idea with fewer corners per cell.
    SIMPLEX,
    slug: "simplex",
    icon: "🌊",
    label: "Simplex Noise",
    category: Generate,
    tooltip: "Gradient noise on a triangular lattice, which has fewer directional artifacts \
              than Perlin. Offset X and Y pan the field without moving the picture; it is still \
              until its Speed or a gear drives it.",
    timing: noise_timing(0.5).still(),
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "scale" "Scale" = Control::num_log(5.0, 0.1, 50.0, 0.1, "/⬓"),
    ],
    after_time: [
        VaryingNumber "offsetX" "Offset X" = Control::num(0.0, -100.0, 100.0, 0.1, "⬓"),
        VaryingNumber "offsetY" "Offset Y" = Control::num(0.0, -100.0, 100.0, 0.1, "⬓"),
        VaryingNumber "contrast" "Contrast" = Control::num(1.0, 0.0, 5.0, 0.01, ""),
    ],
    options: [
        "repeat" "Repeat" = "never" [
            "never" => "Never",
            "1" => "Every 1",
            "2" => "Every 2",
            "4" => "Every 4",
            "8" => "Every 8",
            "16" => "Every 16",
        ],
    ],
    wgsl_utils: [SIMPLEX3D_WGSL, SIMPLEX4D_WGSL, LOOP_CIRCLE_WGSL],
    wgsl_common: varying(|node, ctx| {
        let raw = match repeat_wgsl(ctx.option(node, "repeat")) {
            Some(n) => format!("snoise4(vec4f(p, {}))", noise_time(Some(&n))),
            None => format!("snoise3(vec3f(p, {}))", noise_time(None)),
        };
        format!(
            "    let noiseScale = {{scale}};
    let contrast = {{contrast}};
    let p = (uv + vec2f({{offsetX}}, {{offsetY}})) * noiseScale;
    let raw = {raw};
    let value = clamp(raw * 0.5 * contrast + 0.5, 0.0, 1.0);
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = NOISE_COLOR,
        VaryingNumber "value" "Value" = NOISE_VALUE,
    ],
}

/// The nine-cell search every Worley mode starts from: the nearest feature point, the second
/// nearest, and the point itself.
///
/// A Minkowski distance of exponent `metric` — 1 is a diamond, 2 a circle, large a square —
/// so the cells change shape without changing where their centers are.
const WORLEY_CELLS_WGSL: &str = "    let noiseScale = {scale};
    let randomness = clamp({randomness}, 0.0, 1.0);
    let metric = max({metric}, 0.1);
    let contrast = {contrast};
    let p = uv * noiseScale;
    let cellIndex = floor(p);
    let cellFract = fract(p);
    var nearest = 10.0;
    var second = 10.0;
    var nearestPoint = vec2f(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let neighbor = vec2f(f32(x), f32(y));
            let cell = cellIndex + neighbor;
            let jitter = vec2f(
                fract(sin(dot(cell, vec2f(12.9898, 78.233))) * 43758.5453),
                fract(sin(dot(cell, vec2f(269.5, 183.3))) * 43758.5453));
            let point = neighbor + mix(vec2f(0.5), jitter, randomness);
            let diff = point - cellFract;
            let dist = pow(pow(abs(diff.x), metric) + pow(abs(diff.y), metric), 1.0 / metric);
            if (dist < nearest) {
                second = nearest;
                nearest = dist;
                nearestPoint = cell + jitter;
            } else if (dist < second) {
                second = dist;
            }
        }
    }
";

/// Brightness falling away from each feature point.
const WORLEY_DISTANCE_WGSL: &str = "    let raw = 1.0 - nearest;\n";

/// One flat random tone per cell, keyed on the feature point.
const WORLEY_CELLS_MODE_WGSL: &str =
    "    let raw = fract(sin(dot(nearestPoint, vec2f(12.9898, 78.233))) * 43758.5453);\n";

/// Dark only where the two nearest points are equidistant: the Voronoi edges.
const WORLEY_BORDERS_WGSL: &str = "    let raw = smoothstep(0.0, 0.05, second - nearest);\n";

const WORLEY_CONTRAST_WGSL: &str =
    "    let value = clamp((raw - 0.5) * contrast + 0.5, 0.0, 1.0);\n";

node! {
    /// Cellular noise: the distance to the nearest of one random point per lattice cell.
    WORLEY,
    slug: "worley",
    icon: "🦗",
    label: "Worley Noise",
    category: Generate,
    tooltip: "Cellular noise from one random point per lattice cell. Distance shades away \
              from each point, Cells flattens each to one tone, Borders draws the Voronoi \
              edges.",
    // "Distance Metric" does not fit the default 200.
    width: 240.0,
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "scale" "Frequency" = Control::num_log(10.0, 1.0, 50.0, 0.1, "/⬓"),
        VaryingNumber "randomness" "Randomness" = Control::num(1.0, 0.0, 1.0, 0.01, ""),
        VaryingNumber "metric" "Distance Metric" = Control::num(2.0, 0.5, 10.0, 0.1, ""),
        VaryingNumber "contrast" "Contrast" = Control::num(1.0, 0.0, 5.0, 0.01, ""),
    ],
    options: [
        "mode" "Mode" = "distance" [
            "distance" => "Distance",
            "cells" => "Cells",
            "borders" => "Borders",
        ],
    ],
    wgsl_common: varying(|node, ctx| {
        [
            WORLEY_CELLS_WGSL,
            match ctx.option(node, "mode") {
                "cells" => WORLEY_CELLS_MODE_WGSL,
                "borders" => WORLEY_BORDERS_WGSL,
                _ => WORLEY_DISTANCE_WGSL,
            },
            WORLEY_CONTRAST_WGSL,
        ]
        .concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = NOISE_COLOR,
        VaryingNumber "value" "Value" = NOISE_VALUE,
    ],
}

node! {
    /// Octaves of simplex noise summed at falling amplitude: fractal Brownian motion.
    FRACTAL,
    slug: "fractal",
    icon: "🏔",
    label: "Fractal Noise",
    category: Generate,
    tooltip: "Octaves of simplex noise summed at falling amplitude. Lacunarity is how much \
              finer each octave is, gain how much quieter; Ridged and Turbulence fold each \
              octave about its middle.",
    timing: noise_timing(0.5).still(),
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "scale" "Scale" = Control::num_log(5.0, 0.1, 50.0, 0.1, "/⬓"),
    ],
    after_time: [
        VaryingNumber "octaves" "Octaves" = Control::num(4.0, 1.0, 8.0, 1.0, ""),
        VaryingNumber "lacunarity" "Lacunarity" = Control::num(2.0, 1.0, 4.0, 0.1, "x"),
        VaryingNumber "gain" "Gain" = Control::num(0.5, 0.1, 1.0, 0.01, "x"),
        VaryingNumber "contrast" "Contrast" = Control::num(1.0, 0.0, 5.0, 0.01, ""),
    ],
    options: [
        "type" "Type" = "standard" [
            "standard" => "Standard",
            "turbulence" => "Turbulence",
            "ridged" => "Ridged",
        ],
        "repeat" "Repeat" = "never" [
            "never" => "Never",
            "1" => "Every 1",
            "2" => "Every 2",
            "4" => "Every 4",
            "8" => "Every 8",
            "16" => "Every 16",
        ],
    ],
    wgsl_utils: [SIMPLEX3D_WGSL, FBM_WGSL, SIMPLEX4D_WGSL, FBM_LOOP_WGSL, LOOP_CIRCLE_WGSL],
    wgsl_common: varying(|node, ctx| {
        let (fbm, time) = fbm_time(ctx.option(node, "repeat"));
        format!(
            "    let contrast = {{contrast}};
    let octaves = i32(clamp({{octaves}}, 1.0, 8.0));
    let raw = {fbm}(uv * ({{scale}}), 1.0, octaves, {{lacunarity}}, {{gain}}, {time}, {});
    let value = clamp((raw - 0.5) * contrast + 0.5, 0.0, 1.0);
",
            fbm_shape(ctx.option(node, "type"))
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = NOISE_COLOR,
        VaryingNumber "value" "Value" = NOISE_VALUE,
    ],
}

node! {
    /// One random tone per lattice cell, reshuffled on a schedule: television static.
    STATIC,
    slug: "static",
    icon: "🌨",
    label: "Static",
    category: Generate,
    tooltip: "One random tone per cell of a square lattice. Time counts rolls, each one the \
              whole field redrawn — still until its Speed or a gear drives it; smoothness \
              blurs each cell into its neighbors, and Repeat plays the same rolls again.",
    timing: noise_timing(6.0).still(),
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "seed" "Seed" = Control::num(0.0, 0.0, 1000.0, 1.0, ""),
        VaryingNumber "scale" "Scale" = Control::num_log(10.0, 1.0, 100.0, 0.1, "/⬓"),
    ],
    after_time: [
        VaryingNumber "smoothness" "Smoothness" = Control::num(0.0, 0.0, 1.0, 0.01, ""),
    ],
    options: [
        "repeat" "Repeat" = "never" [
            "never" => "Never",
            "4" => "Every 4",
            "8" => "Every 8",
            "16" => "Every 16",
            "32" => "Every 32",
            "64" => "Every 64",
            "128" => "Every 128",
        ],
    ],
    wgsl_utils: [CELL_HASH_WGSL],
    // The seed steps rather than sliding: `floor` of the accumulated count is what makes the
    // field hold still between redraws instead of crawling, and a rate of zero floors to
    // zero, which is the still picture with no branch to say so.
    //
    // silvia switches between a hard cell and a bilinear one at a threshold; here smoothness
    // mixes them. The hard value is the cell's own corner, which is what the bilinear one
    // reads at the corner, so the two ends of the control agree with silvia's two branches
    // and everything between is continuous.
    wgsl_common: varying(|node, ctx| {
        let roll = match repeat_wgsl(ctx.option(node, "repeat")) {
            Some(n) => format!(
                "    let roll = floor_mod(floor(time_repeat({{clock}}, {n}, {{phaseOffset}})), {n});\n"
            ),
            None => "    let roll = floor(time_unbounded({clock}, {phaseOffset}));\n".to_string(),
        };
        format!(
            "    let noiseScale = {{scale}};
    let smoothness = clamp({{smoothness}}, 0.0, 1.0);
    let gridUV = uv * noiseScale;
    let cell = floor(gridUV);
    let cellFract = fract(gridUV);
{roll}    let animSeed = {{seed}} + roll;
    let tl = cellHash(cell, animSeed);
    let tr = cellHash(cell + vec2f(1.0, 0.0), animSeed);
    let bl = cellHash(cell + vec2f(0.0, 1.0), animSeed);
    let br = cellHash(cell + vec2f(1.0, 1.0), animSeed);
    let blend = smoothstep(vec2f(0.0), vec2f(1.0), cellFract);
    let smoothed = mix(mix(tl, tr, blend.x), mix(bl, br, blend.x), blend.y);
    let value = mix(tl, smoothed, smoothness);
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = NOISE_COLOR,
        VaryingNumber "value" "Value" = NOISE_VALUE,
    ],
}
