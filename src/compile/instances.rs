// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Copies listed, not folded.** A counted repeat — Repeat, Repeat Around, Scatter on Sphere —
//! lists its copies on the GPU each draw, before the picture, and finds the ones near a point
//! through a grid, so a copy is never cut at the edge of a cell however big it is or however
//! deep the repeats nest. See [docs/nodes.md](../../docs/nodes.md#repetition).
//!
//! **A copy is an instance**: a rigid map `w = R p + o` from the repeat's space into the
//! copy's frame — its cell, its place round the ring or on the sphere, its own offset and turn
//! — its scale on each of the input's axes, its color, and its box in the repeat's space, the
//! input's bounds carried out through the map. **A repeat's grid** is a box round all of them
//! cut into cells, none narrower than a copy along any axis, each listing the copies whose box, grown by a margin of half a cell,
//! touches it. A copy not listed in a point's cell is more than the margin from every point of
//! the cell, so the copies listed decide the distance up to the margin: a march is held to it,
//! and every other read takes the copies listed alone. **Empty space is crossed in one step**:
//! every copy lies in the cells that list it, so a point in an empty cell whose surrounding
//! rings of cells are empty is no nearer a copy than the edge of those rings, and a march steps
//! there.
//!
//! **Five compute passes build every repeat's list and grid**, each a dispatch over all of a
//! module's repeats at once, the repeat in the dispatch's `z`: place (each copy's instance and
//! the biggest box among them, the grid's box and count from the first, the counts zeroed),
//! grid (the cells sized by the biggest copy), count (each copy adds one to each cell it
//! touches), scan (one workgroup a repeat turns counts into starts and checks the whole fits),
//! fill (each copy writes its index into its cells), then ring, [`RINGS`] times (each empty
//! cell counts the rings of empty cells round it, a ring more each time). Atomics fill a cell in any order,
//! so a point's distance breaks a tie by the lower index: a render never flickers between two
//! copies at the same distance.
//!
//! **Capacities are fixed when the module is compiled**, a repeat's own, so the buffers' sizes
//! travel on the [`super::Shader`]. A draw whose copies outgrow them — more copies than the
//! capacity, or copies so tangled the lists overflow — marks its grid unusable, and the repeat
//! falls back to measuring by folding for that draw.

use std::fmt::Write as _;

/// The bindings, after the textures' range: instances, grid headers, cell counts, lists.
pub const INSTANCES: u32 = 100;
pub const GRIDS: u32 = 101;
pub const COUNTS: u32 = 102;
pub const LISTS: u32 = 103;

/// A cell's list may hold this many times a repeat's copies: a copy no bigger than a cell,
/// grown by half a cell, touches at most three cells along each axis.
const ENTRIES_PER_ITEM: u32 = 27;

/// The words after a repeat's counts that hold the biggest copy's box along each axis, as the
/// place pass finds it, for the grid pass to size the cells by.
const EXTENT_WORDS: u32 = 3;

/// How many rings of empty cells round an empty cell are counted, a pass each: a march
/// crossing empty cells steps past as many rings as it is sure of, at most this many.
pub const RINGS: u32 = 6;

/// The threads in a workgroup of the place, count and fill passes.
pub const WORKGROUP: u32 = 64;

/// The compute entry points, in the order a draw dispatches them.
pub const ENTRIES: [&str; 6] = [
    "cs_place", "cs_grid", "cs_count", "cs_scan", "cs_fill", "cs_ring",
];

/// Where one repeat's copies, grid, counts and lists sit in the module's buffers, and how many
/// of each it may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Its grid's index in `sd_grid`, and its place in each dispatch's `z`.
    pub grid: u32,
    /// The most copies, cells and listed entries it holds.
    pub items: u32,
    pub cells: u32,
    pub entries: u32,
    /// Its first instance in `sd_inst`, its first count in `sd_count`, and its first word in
    /// `sd_list`: a start per cell, then the entries.
    pub item_base: u32,
    pub count_base: u32,
    pub list_base: u32,
}

impl Slot {
    /// Its first cell's empty rings, after its counts: see [`RINGS`].
    pub fn rings(&self) -> u32 {
        self.count_base + self.cells
    }

    /// The first of the three words after its rings that keep its biggest copy's size.
    pub fn extent(&self) -> u32 {
        self.count_base + 2 * self.cells
    }

    /// A repeat of at most `items` copies in at most `cells` cells, after every slot in
    /// `before`.
    pub fn after(before: &[Slot], items: u32, cells: u32) -> Self {
        let entries = items * ENTRIES_PER_ITEM;
        let last = before.last();
        Self {
            grid: before.len() as u32,
            items,
            cells,
            entries,
            item_base: last.map_or(0, |s| s.item_base + s.items),
            count_base: last.map_or(0, |s| s.count_base + 2 * s.cells + EXTENT_WORDS),
            list_base: last.map_or(0, |s| s.list_base + s.cells + s.entries),
        }
    }
}

/// What a module's buffers hold, for the renderer to size them and dispatch: every repeat's
/// [`Slot`] summed. All zero where no repeat lists its copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Instancing {
    /// The repeats, each a `z` of every dispatch.
    pub repeats: u32,
    /// The most copies, and the most copies or cells, any one repeat holds: what the count
    /// and fill passes, and the place pass, dispatch over in `x`.
    pub items: u32,
    pub place: u32,
    /// The most cells any one repeat holds: what the ring pass dispatches over.
    pub cells: u32,
    /// The buffers' lengths: instances, counts (with each repeat's extent words), list words.
    pub instances: u32,
    pub counts: u32,
    pub lists: u32,
}

impl Instancing {
    pub fn of(slots: &[Slot]) -> Self {
        let Some(last) = slots.last() else {
            return Self::default();
        };
        Self {
            repeats: slots.len() as u32,
            items: slots.iter().map(|s| s.items).max().unwrap_or(0),
            place: slots
                .iter()
                .map(|s| s.items.max(s.cells))
                .max()
                .unwrap_or(0),
            cells: slots.iter().map(|s| s.cells).max().unwrap_or(0),
            instances: last.item_base + last.items,
            counts: last.count_base + 2 * last.cells + EXTENT_WORDS,
            lists: last.list_base + last.cells + last.entries,
        }
    }

    /// Whether the module lists any copies.
    pub fn any(&self) -> bool {
        self.repeats > 0
    }

    /// Each dispatch in the order a draw makes them: the entry point's index in [`ENTRIES`]
    /// and its workgroups. The ring pass runs [`RINGS`] times.
    pub fn dispatches(&self) -> Vec<(usize, (u32, u32, u32))> {
        let over = |n: u32| n.div_ceil(WORKGROUP).max(1);
        let mut out = vec![
            (0, (over(self.place), 1, self.repeats)),
            (1, (1, 1, self.repeats)),
            (2, (over(self.items), 1, self.repeats)),
            (3, (1, 1, self.repeats)),
            (4, (over(self.items), 1, self.repeats)),
        ];
        out.extend((0..RINGS).map(|_| (5, (over(self.cells), 1, self.repeats))));
        out
    }

    /// The bytes of each buffer, in binding order: one `SdInstance` is seven `vec4f`s, one
    /// `SdGrid` four, a count or a list word four bytes.
    pub fn sizes(&self) -> [u64; 4] {
        [
            u64::from(self.instances.max(1)) * 112,
            u64::from(self.repeats.max(1)) * 64,
            u64::from(self.counts.max(1)) * 4,
            u64::from(self.lists.max(1)) * 4,
        ]
    }
}

/// The declarations and helpers every module whose repeats list their copies carries.
pub const WGSL: &str = "// Copies listed, not folded: see compile/instances.rs.
// A copy: the rigid map into its frame as three rows, w = (dot(r0.xyz, p) + r0.w, ...), its
// scale on the input's axes, its color, and its box in the repeat's space.
struct SdInstance {
    r0: vec4f,
    r1: vec4f,
    r2: vec4f,
    scale: vec4f,
    color: vec4f,
    lo: vec4f,
    hi: vec4f,
}
// A repeat's grid: its corner (w the margin), the far corner of its cells (w the copies), a
// cell's size (w 1 where this draw's lists hold every copy), and the cells along each axis (w
// all of them).
struct SdGrid {
    lo: vec4f,
    hi: vec4f,
    cs: vec4f,
    res: vec4u,
}
@group(0) @binding(100) var<storage, read_write> sd_inst: array<SdInstance>;
@group(0) @binding(101) var<storage, read_write> sd_grid: array<SdGrid>;
@group(0) @binding(102) var<storage, read_write> sd_count: array<atomic<u32>>;
@group(0) @binding(103) var<storage, read_write> sd_list: array<u32>;
var<workgroup> sd_scan_part: array<u32, 256>;

// The copy whose frame map takes the origin to `o` and the three axes to `ex`, `ey` and `ez`,
// scaled by `scale` and colored `color`, over an input held by `b`.
fn sd_instance(o: vec3f, ex: vec3f, ey: vec3f, ez: vec3f, scale: vec3f, color: vec3f, b: SdBounds) -> SdInstance {
    let cx = ex - o;
    let cy = ey - o;
    let cz = ez - o;
    var lo = vec3f(1.0e9);
    var hi = vec3f(-1.0e9);
    if (!sd_bounds_empty(b)) {
        for (var k = 0u; k < 8u; k++) {
            let corner = select(b.lo, b.hi, vec3<bool>((k & 1u) != 0u, (k & 2u) != 0u, (k & 4u) != 0u));
            let v = corner * scale - o;
            let q = vec3f(dot(cx, v), dot(cy, v), dot(cz, v));
            lo = min(lo, q);
            hi = max(hi, q);
        }
    }
    return SdInstance(
        vec4f(cx.x, cy.x, cz.x, o.x),
        vec4f(cx.y, cy.y, cz.y, o.y),
        vec4f(cx.z, cy.z, cz.z, o.z),
        vec4f(scale, 0.0),
        vec4f(color, 0.0),
        vec4f(lo, 0.0),
        vec4f(hi, 0.0),
    );
}

// A copy's own frame inside its place: the point moved to the copy's origin `at` and turned
// by `turn`, in radians, about y.
fn sd_copy_frame(x: vec3f, at: vec3f, turn: f32) -> vec3f {
    let m = x - at;
    let mt = sd_rot2(m.xz, turn);
    return vec3f(mt.x, m.y, mt.y);
}

// Copy `c` written at `at`, its box's size kept where it is the biggest yet: in the three words
// from `ext`, as the bits of a positive float, which order as the floats do.
fn sd_place(at: u32, ext: u32, c: SdInstance) {
    sd_inst[at] = c;
    if (all(c.lo.xyz <= c.hi.xyz)) {
        let e = c.hi.xyz - c.lo.xyz;
        atomicMax(&sd_count[ext], bitcast<u32>(e.x));
        atomicMax(&sd_count[ext + 1u], bitcast<u32>(e.y));
        atomicMax(&sd_count[ext + 2u], bitcast<u32>(e.z));
    }
}

// A repeat's grid cut from the box and count the place pass wrote, by the biggest copy it
// found, and that copy's size forgotten for the next draw.
fn sd_grid_finish(gi: u32, ext: u32, cap: f32) {
    let h = sd_grid[gi];
    let size = vec3f(
        bitcast<f32>(atomicLoad(&sd_count[ext])),
        bitcast<f32>(atomicLoad(&sd_count[ext + 1u])),
        bitcast<f32>(atomicLoad(&sd_count[ext + 2u])),
    );
    var g = sd_grid_make(SdBounds(h.lo.xyz, h.hi.xyz), h.hi.w, size, cap);
    g.cs.w = min(g.cs.w, h.cs.w);
    sd_grid[gi] = g;
    atomicStore(&sd_count[ext], 0u);
    atomicStore(&sd_count[ext + 1u], 0u);
    atomicStore(&sd_count[ext + 2u], 0u);
}

// One step of counting how far each cell is from a cell that lists a copy, in cells along the
// furthest axis: 0 where it lists one, and otherwise one more than the least of its 26
// neighbors, a neighbor past the grid's edge as far as can be, at most `RINGS`. Read while other
// cells are being written, a neighbor's is its last or its new, each no more than the truth, so
// the cell's is never more than the truth either. A cell `n` away has `n - 1` empty rings round
// it.
fn sd_grid_ring(gi: u32, count_base: u32, rings: u32, t: u32, most: u32) {
    let g = sd_grid[gi];
    if (t >= g.res.w) { return; }
    if (atomicLoad(&sd_count[count_base + t]) > 0u) {
        atomicStore(&sd_count[rings + t], 0u);
        return;
    }
    let c = vec3i(vec3u(t % g.res.x, (t / g.res.x) % g.res.y, t / (g.res.x * g.res.y)));
    let r = vec3i(g.res.xyz);
    var least = most;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let n = c + vec3i(dx, dy, dz);
                if (any(n < vec3i(0)) || any(n >= r) || all(vec3i(dx, dy, dz) == vec3i(0))) { continue; }
                let k = u32(n.x) + g.res.x * (u32(n.y) + g.res.y * u32(n.z));
                least = min(least, atomicLoad(&sd_count[rings + k]));
            }
        }
    }
    atomicStore(&sd_count[rings + t], min(least + 1u, most));
}

// How far `q`, in cell `c`, is from the nearest cell that may list a copy, `c` being `n` cells
// from one: past the `n - 1` empty rings round it, along the axes the rings stop short of the
// grid's edge on; far where they reach every edge, and nothing known where `n` is 0.
fn sd_grid_clear(g: SdGrid, c: vec3u, n: u32, q: vec3f) -> f32 {
    if (n == 0u) { return 0.0; }
    let l = n - 1u;
    let lo = g.lo.xyz + (vec3f(c) - f32(l)) * g.cs.xyz;
    let hi = g.lo.xyz + (vec3f(c) + f32(l) + 1.0) * g.cs.xyz;
    let low = vec3f(c) >= vec3f(f32(l) + 0.5);
    let high = vec3f(c) + f32(l) + 1.0 <= vec3f(g.res.xyz) - 0.5;
    let below = select(vec3f(1.0e9), q - lo, low);
    let above = select(vec3f(1.0e9), hi - q, high);
    let d = min(below, above);
    return min(d.x, min(d.y, d.z));
}

// The point `p` of the repeat's space in a copy's frame, before its scale.
fn sd_instance_point(c: SdInstance, p: vec3f) -> vec3f {
    return vec3f(dot(c.r0.xyz, p) + c.r0.w, dot(c.r1.xyz, p) + c.r1.w, dot(c.r2.xyz, p) + c.r2.w);
}

// The grid for `n` copies inside `b`, each at most `size` across on each axis: about two cells a
// copy, none narrower than a copy along any axis, at most `cap` in all. The margin is half the
// narrowest cell, so a copy grown by it touches at most three cells along each axis.
fn sd_grid_make(b: SdBounds, n: f32, size: vec3f, cap: f32) -> SdGrid {
    var lo = b.lo;
    var e = b.hi - b.lo;
    if (sd_bounds_empty(b)) {
        lo = vec3f(0.0);
        e = vec3f(0.0);
    }
    e = max(e, vec3f(1.0e-4));
    let want = clamp(2.0 * n, 1.0, cap);
    var c = max(e.x, max(e.y, e.z));
    for (var k = 0; k < 64; k++) {
        let r = max(ceil(e / c), vec3f(1.0));
        if (r.x * r.y * r.z >= want) { break; }
        c *= 0.85;
    }
    var cs = max(vec3f(c), size);
    for (var k = 0; k < 64; k++) {
        let r = max(ceil(e / cs), vec3f(1.0));
        if (r.x * r.y * r.z <= cap) { break; }
        cs *= 1.2;
    }
    let res = vec3u(max(ceil(e / cs), vec3f(1.0)));
    return SdGrid(
        vec4f(lo, 0.5 * min(cs.x, min(cs.y, cs.z))),
        vec4f(lo + vec3f(res) * cs, n),
        vec4f(cs, 1.0),
        vec4u(res, res.x * res.y * res.z),
    );
}

// The cells, first and last along each axis, that a box grown by the margin touches.
fn sd_grid_span(g: SdGrid, lo: vec3f, hi: vec3f) -> array<vec3u, 2> {
    let top = vec3f(g.res.xyz) - 1.0;
    let a = clamp(floor((lo - g.lo.w - g.lo.xyz) / g.cs.xyz), vec3f(0.0), top);
    let z = clamp(floor((hi + g.lo.w - g.lo.xyz) / g.cs.xyz), vec3f(0.0), top);
    return array<vec3u, 2>(vec3u(a), vec3u(z));
}

// The cell holding `p`, or the nearest to it.
fn sd_grid_cell(g: SdGrid, p: vec3f) -> u32 {
    let c = vec3u(clamp(floor((p - g.lo.xyz) / g.cs.xyz), vec3f(0.0), vec3f(g.res.xyz) - 1.0));
    return c.x + g.res.x * (c.y + g.res.y * c.z);
}

// Copy `i`'s count added to every cell it touches.
fn sd_grid_count(gi: u32, count_base: u32, item_base: u32, i: u32) {
    let g = sd_grid[gi];
    let c = sd_inst[item_base + i];
    if (any(c.lo.xyz > c.hi.xyz)) { return; }
    let s = sd_grid_span(g, c.lo.xyz, c.hi.xyz);
    for (var z = s[0].z; z <= s[1].z; z++) {
        for (var y = s[0].y; y <= s[1].y; y++) {
            for (var x = s[0].x; x <= s[1].x; x++) {
                atomicAdd(&sd_count[count_base + x + g.res.x * (y + g.res.y * z)], 1u);
            }
        }
    }
}

// Copy `i` written into every cell it touches.
fn sd_grid_fill(gi: u32, count_base: u32, list_base: u32, cells: u32, entries: u32, item_base: u32, i: u32) {
    let g = sd_grid[gi];
    let c = sd_inst[item_base + i];
    if (any(c.lo.xyz > c.hi.xyz)) { return; }
    let s = sd_grid_span(g, c.lo.xyz, c.hi.xyz);
    for (var z = s[0].z; z <= s[1].z; z++) {
        for (var y = s[0].y; y <= s[1].y; y++) {
            for (var x = s[0].x; x <= s[1].x; x++) {
                let cell = x + g.res.x * (y + g.res.y * z);
                let at = sd_list[list_base + cell] + atomicAdd(&sd_count[count_base + cell], 1u);
                if (at < entries) { sd_list[list_base + cells + at] = i; }
            }
        }
    }
}

// One workgroup of 256: every cell's count turned into the start of its list, the counts
// zeroed for the fill to count again, and the grid marked unusable where the lists overflow.
fn sd_grid_scan(gi: u32, count_base: u32, list_base: u32, entries: u32, lid: u32) {
    let cells = sd_grid[gi].res.w;
    let per = (cells + 255u) / 256u;
    let first = min(lid * per, cells);
    let last = min(first + per, cells);
    var sum = 0u;
    for (var k = first; k < last; k++) {
        sum += atomicLoad(&sd_count[count_base + k]);
    }
    sd_scan_part[lid] = sum;
    workgroupBarrier();
    for (var off = 1u; off < 256u; off *= 2u) {
        var v = 0u;
        if (lid >= off) { v = sd_scan_part[lid - off]; }
        workgroupBarrier();
        sd_scan_part[lid] += v;
        workgroupBarrier();
    }
    var start = sd_scan_part[lid] - sum;
    for (var k = first; k < last; k++) {
        let n = atomicLoad(&sd_count[count_base + k]);
        sd_list[list_base + k] = start;
        start += n;
        atomicStore(&sd_count[count_base + k], 0u);
    }
    if (lid == 255u && sd_scan_part[255] > entries) {
        sd_grid[gi].cs.w = 0.0;
    }
}
";

/// The compute entry points over `slots`, each a `switch` on the dispatch's `z` calling every
/// repeat's own `place` function by name, and the shared helpers for the rest.
pub fn entries(slots: &[(Slot, String)]) -> String {
    let mut place = String::new();
    let mut grid = String::new();
    let mut count = String::new();
    let mut scan = String::new();
    let mut fill = String::new();
    let mut ring = String::new();
    for (s, name) in slots {
        let g = s.grid;
        writeln!(place, "        case {g}u: {{ {name}(t); }}").unwrap();
        writeln!(
            grid,
            "        case {g}u: {{ sd_grid_finish({g}u, {ext}u, {cells}.0); }}",
            ext = s.extent(),
            cells = s.cells
        )
        .unwrap();
        writeln!(
            count,
            "        case {g}u: {{ if (t < u32(sd_grid[{g}u].hi.w) && t < {items}u) {{ sd_grid_count({g}u, {cb}u, {ib}u, t); }} }}",
            items = s.items,
            cb = s.count_base,
            ib = s.item_base
        )
        .unwrap();
        writeln!(
            scan,
            "        case {g}u: {{ sd_grid_scan({g}u, {cb}u, {lb}u, {e}u, lid); }}",
            cb = s.count_base,
            lb = s.list_base,
            e = s.entries
        )
        .unwrap();
        writeln!(
            ring,
            "        case {g}u: {{ sd_grid_ring({g}u, {cb}u, {rb}u, t, {RINGS}u); }}",
            cb = s.count_base,
            rb = s.rings(),
        )
        .unwrap();
        writeln!(
            fill,
            "        case {g}u: {{ if (t < u32(sd_grid[{g}u].hi.w) && t < {items}u) {{ sd_grid_fill({g}u, {cb}u, {lb}u, {cells}u, {e}u, {ib}u, t); }} }}",
            items = s.items,
            cb = s.count_base,
            lb = s.list_base,
            cells = s.cells,
            e = s.entries,
            ib = s.item_base
        )
        .unwrap();
    }
    format!(
        "
@compute @workgroup_size({WORKGROUP})
fn cs_place(@builtin(global_invocation_id) id: vec3u) {{
    let t = id.x;
    switch id.z {{
{place}        default: {{}}
    }}
}}

@compute @workgroup_size(1)
fn cs_grid(@builtin(workgroup_id) wg: vec3u) {{
    switch wg.z {{
{grid}        default: {{}}
    }}
}}

@compute @workgroup_size({WORKGROUP})
fn cs_count(@builtin(global_invocation_id) id: vec3u) {{
    let t = id.x;
    switch id.z {{
{count}        default: {{}}
    }}
}}

@compute @workgroup_size(256)
fn cs_scan(@builtin(workgroup_id) wg: vec3u, @builtin(local_invocation_index) lid: u32) {{
    switch wg.z {{
{scan}        default: {{}}
    }}
}}

@compute @workgroup_size({WORKGROUP})
fn cs_fill(@builtin(global_invocation_id) id: vec3u) {{
    let t = id.x;
    switch id.z {{
{fill}        default: {{}}
    }}
}}

@compute @workgroup_size({WORKGROUP})
fn cs_ring(@builtin(global_invocation_id) id: vec3u) {{
    let t = id.x;
    switch id.z {{
{ring}        default: {{}}
    }}
}}
"
    )
}

/// The statements that start a repeat's `place` function for copy `t`: from the first
/// thread the grid's box, `bounds`, and its copies, `n`, of which the capacity holds `items`,
/// for the grid pass to cut into cells; every count zeroed. The caller writes copy `t` after,
/// for `t` below `min(n, items)`, with `sd_place` at [`Slot::item_base`] and [`Slot::extent`].
pub fn place_head(s: &Slot, n: &str, bounds: &str) -> String {
    format!(
        "    let total = max({n}, 0.0);
    if (t == 0u) {{
        var b = {bounds};
        if (sd_bounds_empty(b)) {{ b = SdBounds(vec3f(0.0), vec3f(0.0)); }}
        sd_grid[{grid}u] = SdGrid(
            vec4f(b.lo, 0.0),
            vec4f(b.hi, total),
            vec4f(0.0, 0.0, 0.0, select(1.0, 0.0, total > {items}.0)),
            vec4u(0u),
        );
    }}
    if (t < {cells}u) {{
        atomicStore(&sd_count[{cb}u + t], 0u);
        atomicStore(&sd_count[{cb}u + {cells}u + t], 0u);
    }}
    if (f32(t) >= min(total, {items}.0)) {{ return; }}
",
        cells = s.cells,
        items = s.items,
        grid = s.grid,
        cb = s.count_base,
    )
}

/// The statements that measure a point `p` of the repeat's space through its grid into
/// `best`, `eval` measuring one listed copy `c` from `w`, its point in the copy's frame, into
/// `r` (distance and color) and `k` (what the distance is multiplied back by), and `color`
/// writing what a copy's color is from `r` and `c`. Ends returning `best`; the caller puts it
/// under `if (sd_grid[..].cs.w > 0.5)`.
pub fn lookup(s: &Slot, eval: &str, color: &str) -> String {
    format!(
        "        let g = sd_grid[{grid}u];
        let gap = sd_box_gap(p, (g.lo.xyz + g.hi.xyz) * 0.5, (g.hi.xyz - g.lo.xyz) * 0.5);
        var best = vec4f(1.0e9, 0.0, 0.0, 0.0);
        var bi = 0xffffffffu;
        let cell = sd_grid_cell(g, p);
        let count = min(atomicLoad(&sd_count[{cb}u + cell]), {entries}u);
        if (sd_march) {{
            let q = clamp(p, g.lo.xyz, g.hi.xyz);
            var inner = g.lo.w;
            if (count == 0u) {{
                let cc = vec3u(cell % g.res.x, (cell / g.res.x) % g.res.y, cell / (g.res.x * g.res.y));
                inner = max(inner, sd_grid_clear(g, cc, atomicLoad(&sd_count[{rb}u + cell]), q));
            }}
            best.x = sqrt(gap * gap + inner * inner);
        }}
        let start = sd_list[{lb}u + cell];
        for (var j = 0u; j < count; j++) {{
            if (start + j >= {entries}u) {{ break; }}
            let i = sd_list[{lb}u + {cells}u + start + j];
            let c = sd_inst[{ib}u + i];
            let cgap = sd_box_gap(p, (c.lo.xyz + c.hi.xyz) * 0.5, (c.hi.xyz - c.lo.xyz) * 0.5);
            if (cgap > best.x || (cgap == best.x && i > bi)) {{ continue; }}
            let w = sd_instance_point(c, p);
            let sc = c.scale.xyz;
{eval}            let d = select(r.x * k, max(r.x * k, cgap), cgap > 0.0);
            if (d < best.x || (d == best.x && i < bi)) {{
                best = vec4f(d, {color});
                bi = i;
            }}
        }}
        return best;
",
        grid = s.grid,
        cb = s.count_base,
        lb = s.list_base,
        ib = s.item_base,
        cells = s.cells,
        entries = s.entries,
        rb = s.rings(),
    )
}
