// SPDX-License-Identifier: AGPL-3.0-or-later

//! Conway's Life and the three rulesets beside it, stepped on the CPU and published as a
//! texture.
//!
//! Both halves. The `tick` holds the grid — silvia's `runtimeState`, which a saved file never
//! carried — and publishes two channels of it as a frame: red is the cell, green is silvia's
//! trail map. The `output` half is WGSL that samples that frame and mixes the two color
//! inputs by the trail, which is silvia's own `mix(deadColor, aliveColor, state)`.
//!
//! **The grid's size is an option, and everything else a step reads is a `UniformNumber`
//! input.** Changing the size reallocates the world, which is a structural edit and not
//! something a wiggling cable should do once a frame — `docs/decisions.md`, "Not every CPU
//! number is a `UniformNumber`". silvia's four s-numbers are inputs with controls, so each
//! one saves, scrubs and takes a cable.
//!
//! **The world is a torus and its picture says so.** Every neighborhood count wraps in x
//! and y, and `cells` declares `TextureWrap::Repeat` with `TextureFilter::Nearest` — silvia's
//! own parameters for this texture, and the exception to the mirror-wrap rule in
//! `docs/rendering.md`: the picture is sampled at raw worldspace `uv`, as silvia samples it,
//! so the field tiles across the world, and a mirrored tiling would fold a seam across a
//! simulation that has none. Nearest keeps a cell a cell rather than a smudge.
//!
//! One departure from silvia, forced by something outside this file: silvia re-randomizes the
//! grid the moment a hand moves `Init Threshold`, and here the threshold is read when
//! `randomize` fires, because an input that re-randomized on every change would refill the
//! grid at frame rate under a cable. The node's own help text says so, which is the whole of
//! what the departure costs: a knob that does nothing until a button is pressed has to say it
//! somewhere, or it reads as broken.
//!
//! **The grid is on the node.** silvia draws a square of the world on its node and the world
//! was the only thing worth looking at; here the `cells` texture is a
//! [`Region::Preview`](crate::nodes::Region::Preview) under the standard Preview heading, so a
//! press of Step changes something a hand can see without the node being wired to an Output first.
//! Dragging on it to paint live cells, which silvia also does, needs a node body that takes
//! the pointer and is not here.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber, VaryingColor};
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Frame, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, Pixels, TextureFilter, TextureWrap, TickContext,
};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "cellularautomata",
    category: Category::Generate,
    icon: "🦠",
    label: "Cellular Automata",
    tooltip: "Generates complex patterns using cellular automata rules like Conway's Game of \
              Life. Init Threshold is read when Randomize fires, not as you turn it: a cable \
              on it would otherwise refill the grid sixty times a second.",
    inputs: &[
        InputDef {
            key: "aliveInput",
            label: "Alive Color",
            ty: VaryingColor,
            control: Control::color("#11ccffff"),
        },
        InputDef {
            key: "deadInput",
            label: "Dead Color",
            ty: VaryingColor,
            control: Control::color("#cc11ffff"),
        },
        InputDef {
            key: "step",
            label: "Step",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "randomize",
            label: "Randomize",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "autoRun",
            label: "Auto-Run",
            ty: UniformNumber,
            // silvia's Auto-Run is a checkbox: a level that says whether the simulation is
            // running, not a press. Zero is stopped and anything above it runs, so a gate
            // from the graph can start and stop the world.
            control: Control::num(0.0, 0.0, 1.0, 1.0, ""),
        },
        InputDef {
            key: "initThreshold",
            // silvia's name, kept: a row on a 240 wide node has room for a label, not for a
            // sentence. When it lands is said in the node's help text, under the `?`, because
            // a knob that appears to do nothing when you turn it otherwise reads as broken.
            label: "Init Threshold",
            ty: UniformNumber,
            control: Control::num(0.6, 0.0, 1.0, 0.01, ""),
        },
        InputDef {
            key: "trailDecay",
            label: "Trail Decay",
            ty: UniformNumber,
            control: Control::num(0.05, 0.001, 1.0, 0.005, ""),
        },
        InputDef {
            key: "stepsPerFrame",
            label: "Rate",
            ty: UniformNumber,
            control: Control::num(1.0, 1.0, 20.0, 1.0, "×30/s"),
        },
    ],
    outputs: &[
        OutputDef {
            key: "cells",
            label: "Cells",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The state itself: red is the cell — full for alive, half for Brian's Brain's
            // dying — and green is the trail. Sampled at raw worldspace uv, silvia's own
            // mapping, so the grid tiles across the world rather than being fitted to it.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "cells");
                let sampler = ctx.sampler(node, "cells");
                format!("    return textureSampleLevel({tex}, {sampler}, uv, 0.0);")
            },
            // The grid wraps in the simulation, so it wraps in the texture: silvia's own
            // `REPEAT` and `NEAREST` for this one, and the exception the rule allows.
            wrap: TextureWrap::Repeat,
            filter: TextureFilter::Nearest,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "output",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            // silvia's: the trail, not the cell, is what the mix reads, so a cell that died
            // fades out over `trailDecay` instead of vanishing.
            // The sibling port's own sampler, since `cells` is the texture being read.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "cells");
                let sampler = ctx.sampler(node, "cells");
                let alive = ctx.input(node, "aliveInput", "uv");
                let dead = ctx.input(node, "deadInput", "uv");
                format!(
                    "    let state = textureSampleLevel({tex}, {sampler}, uv, 0.0).g;
    return mix({dead}, {alive}, state);"
                )
            },
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        OptionDef {
            key: "algorithm",
            label: "Algorithm",
            default: "life",
            choices: &[
                ("life", "Conway's Life (B3/S23)"),
                ("highlife", "HighLife (B36/S23)"),
                ("day_and_night", "Day & Night (B3678/S34678)"),
                ("brians_brain", "Brian's Brain (3-state)"),
            ],
            // Read by `tick`, which builds the rule from it. It swaps a lookup table and
            // generates no WGSL, so changing rules mid-set rebuilds nothing.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "gridScale",
            label: "Grid Scale",
            default: "4",
            // Every value silvia's own 1..16 s-number could hold, named by the grid it makes.
            choices: &[
                ("1", "16x16"),
                ("2", "32x32"),
                ("3", "48x48"),
                ("4", "64x64"),
                ("5", "80x80"),
                ("6", "96x96"),
                ("7", "112x112"),
                ("8", "128x128"),
                ("9", "144x144"),
                ("10", "160x160"),
                ("11", "176x176"),
                ("12", "192x192"),
                ("13", "208x208"),
                ("14", "224x224"),
                ("15", "240x240"),
                ("16", "256x256"),
            ],
            // Read by `tick`, which reallocates the grid on it. The picture reaches the
            // fragment as a bound texture whose size the shader asks for, so no WGSL changes.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_PREVIEW,
    ],
    // The grid on the node, under the same tick every other source's picture is under: a
    // press of Step that changes nothing you can see is a control nobody can learn. Drawing
    // *into* the cells is a different ask — a node body that takes the pointer — and is not
    // here.
    regions: &[crate::nodes::Region::Preview("cells")],
    width: Some(240.0),
    cpu: Some(CpuDef {
        create: || Box::new(Automaton::new()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// silvia's grid unit: the `gridScale` option counts sixteens.
const GRID_UNIT: u32 = 16;

/// The rate the simulation advances at while Auto-Run is up, silvia's own.
const HZ: f64 = 30.0;

/// The most generations one batch runs, whatever `stepsPerFrame` asks for: silvia's maximum.
const MAX_STEPS: u32 = 20;

/// Cell updates one batch may cost. A `tick` never waits, so the batch is cut instead of the
/// frame being missed — at the largest grid this is six generations rather than twenty, and
/// at the default it is well above silvia's own ceiling.
const CELL_BUDGET: u32 = 400_000;

/// Dead, alive, and Brian's Brain's third state.
const DEAD: u8 = 0;
const ALIVE: u8 = 1;
const DYING: u8 = 2;

/// Born and survives, by neighbor count, for one of silvia's three two-state rulesets.
/// Brian's Brain is not one of them: its rule reads the cell's own third state.
fn ruleset(algorithm: &str) -> (&'static [u32], &'static [u32]) {
    match algorithm {
        "highlife" => (&[3, 6], &[2, 3]),
        "day_and_night" => (&[3, 6, 7, 8], &[3, 4, 6, 7, 8]),
        _ => (&[3], &[2, 3]),
    }
}

struct Automaton {
    /// One byte a cell: [`DEAD`], [`ALIVE`] or [`DYING`].
    grid: Vec<u8>,
    next: Vec<u8>,
    /// silvia's trail map: 1.0 under a live cell, floored at 0.5 under a dying one, and
    /// multiplied by `1 - trailDecay` everywhere else, once per batch.
    trail: Vec<f32>,
    /// Cells across, which is also cells down.
    size: u32,
    /// The last frame published, so a tick that changed nothing costs no upload.
    frame: Arc<Frame>,
    /// The generations the clock has paid for and the grid has not run: the fraction of one
    /// carried from tick to tick, so the pace is Rate × 30 a second whatever the tick rate.
    owed: f64,
    alive: u32,
    /// The hand on each button, so a held press is one generation and not one a frame.
    step: crate::nodes::Gate,
    randomize: crate::nodes::Gate,
    rng: Rng,
}

impl Automaton {
    fn new() -> Self {
        Self {
            grid: Vec::new(),
            next: Vec::new(),
            trail: Vec::new(),
            size: 0,
            frame: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            owed: 0.0,
            alive: 0,
            step: crate::nodes::Gate::default(),
            randomize: crate::nodes::Gate::default(),
            rng: Rng::new(),
        }
    }

    /// Allocate a grid `size` on a side and fill it.
    fn resize(&mut self, size: u32, threshold: f32) {
        let cells = (size * size) as usize;
        self.grid = vec![DEAD; cells];
        self.next = vec![DEAD; cells];
        self.trail = vec![0.0; cells];
        self.size = size;
        self.refill(threshold);
    }

    /// silvia's: each cell alive where a fresh random is above the threshold, and no trail.
    fn refill(&mut self, threshold: f32) {
        let rng = &mut self.rng;
        for cell in &mut self.grid {
            *cell = u8::from(rng.next_f32() > threshold);
        }
        self.trail.fill(0.0);
    }

    /// One generation, toroidally wrapped.
    fn generation(&mut self, algorithm: &str) {
        let w = self.size as usize;
        let h = w;
        let brians = algorithm == "brians_brain";
        let (birth, survive) = ruleset(algorithm);
        // Indexed by neighbor count, which is what the rule is a function of.
        let mut birth_lut = [false; 9];
        let mut survive_lut = [false; 9];
        for n in birth {
            birth_lut[*n as usize] = true;
        }
        for n in survive {
            survive_lut[*n as usize] = true;
        }

        let grid = &self.grid;
        let next = &mut self.next;
        for y in 0..h {
            let up = (y + h - 1) % h * w;
            let row = y * w;
            let down = (y + 1) % h * w;
            for x in 0..w {
                let left = (x + w - 1) % w;
                let right = (x + 1) % w;
                let live = |i: usize| u32::from(grid[i] == ALIVE);
                let n = live(up + left)
                    + live(up + x)
                    + live(up + right)
                    + live(row + left)
                    + live(row + right)
                    + live(down + left)
                    + live(down + x)
                    + live(down + right);
                let cell = grid[row + x];
                next[row + x] = if brians {
                    // Alive for one generation, dying for one, then dead; born on exactly
                    // two neighbors.
                    match cell {
                        ALIVE => DYING,
                        DYING => DEAD,
                        _ => u8::from(n == 2),
                    }
                } else if cell == ALIVE {
                    u8::from(survive_lut[n as usize])
                } else {
                    u8::from(birth_lut[n as usize])
                };
            }
        }
        std::mem::swap(&mut self.grid, &mut self.next);
    }

    /// Advance the trail and build the frame the shader samples.
    ///
    /// Once per batch rather than once per generation, which is silvia's: only the last step
    /// of a batch renders, and the trail advances with the render.
    fn redraw(&mut self, decay: f32) {
        let retain = 1.0 - decay;
        let mut pixels = Vec::with_capacity(self.grid.len() * 4);
        let mut alive = 0;
        for (cell, trail) in self.grid.iter().zip(&mut self.trail) {
            match *cell {
                ALIVE => {
                    *trail = 1.0;
                    alive += 1;
                }
                DYING => *trail = trail.max(0.5),
                _ => *trail *= retain,
            }
            let state = match *cell {
                ALIVE => 255,
                DYING => 128,
                _ => 0,
            };
            pixels.extend_from_slice(&[state, (*trail * 255.0) as u8, 0, 255]);
        }
        self.alive = alive;
        self.frame = Arc::new(Frame {
            width: self.size,
            height: self.size,
            pixels: Pixels::Bytes(pixels),
        });
    }

    /// The most generations this grid runs in one tick: silvia's own ceiling, and the cell
    /// budget under it.
    fn limit(&self) -> u32 {
        let cells = (self.size * self.size).max(1);
        MAX_STEPS.min((CELL_BUDGET / cells).max(1))
    }

    /// How many generations `stepsPerFrame` asked for, under both bounds. A value off a cable
    /// is whatever the cable says, so it is rounded, floored at one and capped here rather
    /// than trusted.
    fn batch(&self, asked: f32) -> u32 {
        let asked = if asked.is_finite() {
            asked.round().clamp(1.0, f32::from(u16::MAX)) as u32
        } else {
            1
        };
        asked.min(self.limit())
    }
}

impl Automaton {
    /// The generations `dt` seconds pay for at Rate × 30 a second, the fraction of one kept
    /// for the next tick.
    fn due(&mut self, asked: f32, dt: f32) -> u32 {
        let rate = f64::from(self.batch(asked)) * HZ;
        self.owed += f64::from(dt.max(0.0)) * rate;
        let due = self.owed.floor();
        self.owed -= due;
        due as u32
    }
}

impl CpuNode for Automaton {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        let threshold = ctx.input(id, "initThreshold");
        let decay = ctx.input(id, "trailDecay").clamp(0.0, 1.0);
        let algorithm = ctx.option(id, "algorithm").to_string();

        let scale: u32 = ctx.option(id, "gridScale").parse().unwrap_or(4);
        let size = scale.clamp(1, 16) * GRID_UNIT;
        let mut redraw = false;
        if size != self.size {
            self.resize(size, threshold);
            redraw = true;
        }

        // A press is one generation, and two presses in one frame are two: `downs` counts
        // the hand and the cables together.
        let mut steps = ctx.downs(id, "step", &mut self.step);
        if ctx.downs(id, "randomize", &mut self.randomize) > 0 {
            self.refill(threshold);
            redraw = true;
        }

        // Auto-Run owes Rate generations every thirtieth of a second of the transport's
        // `dt`, silvia's 30 Hz batches, and pays them on the tick they fall due, as the slime
        // mold does: the pace is the same whatever the display does, and a pause holds it.
        if ctx.input(id, "autoRun") >= 0.5 {
            steps += self.due(ctx.input(id, "stepsPerFrame"), ctx.dt);
        } else {
            self.owed = 0.0;
        }

        for _ in 0..steps.min(self.limit()) {
            self.generation(&algorithm);
            redraw = true;
        }
        if redraw {
            self.redraw(decay);
        }
        ctx.publish_frame(id, "cells", Arc::clone(&self.frame));
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "cellularautomata {}x{}, {} alive",
            self.size, self.size, self.alive
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many cells are alive. A sum rather than a count of one byte, which clippy reads as
    /// a job for a crate this does not depend on.
    fn alive(sim: &Automaton) -> u32 {
        sim.grid.iter().map(|c| u32::from(*c == ALIVE)).sum()
    }

    /// **The pace is Rate × 30 generations a second whatever the tick rate**: a second of
    /// 60 Hz ticks and a second of 100 Hz ticks each run 30 at Rate 1 and 90 at Rate 3, the
    /// fraction carried rather than dropped, and a tick with no time in it runs none.
    #[test]
    fn a_second_is_rate_times_thirty_generations_at_any_tick_rate() {
        for hz in [60u32, 100, 144, 7] {
            for rate in [1.0f32, 3.0] {
                let mut sim = Automaton::new();
                sim.resize(64, 0.5);
                let total: u32 = (0..hz).map(|_| sim.due(rate, 1.0 / hz as f32)).sum();
                let want = 30 * rate as u32;
                assert!(
                    total.abs_diff(want) <= 1,
                    "{total} generations in a second at {hz} Hz, Rate {rate}"
                );
            }
        }
        let mut sim = Automaton::new();
        sim.resize(64, 0.5);
        assert_eq!(sim.due(1.0, 0.0), 0);
    }

    /// silvia's three two-state rules, as the tables the neighbor count indexes.
    #[test]
    fn every_algorithm_choice_names_a_ruleset() {
        for (value, _) in DEF.option("algorithm").unwrap().choices {
            let (birth, survive) = ruleset(value);
            if *value == "brians_brain" {
                // Brian's Brain reads the cell, not the table, and falls through to Life's.
                continue;
            }
            assert!(!birth.is_empty(), "{value}: nothing is ever born");
            assert!(!survive.is_empty(), "{value}: nothing ever survives");
        }
        assert_eq!(ruleset("life"), (&[3u32][..], &[2u32, 3][..]));
        assert_eq!(ruleset("highlife"), (&[3u32, 6][..], &[2u32, 3][..]));
    }

    /// Every `gridScale` choice is a scale this parses, and the default is silvia's 64.
    #[test]
    fn every_grid_scale_choice_is_a_grid() {
        for (value, _) in DEF.option("gridScale").unwrap().choices {
            let scale: u32 = value
                .parse()
                .unwrap_or_else(|_| panic!("{value} is a scale"));
            assert!(
                (1..=16).contains(&scale),
                "{value} is outside silvia's range"
            );
        }
        let default: u32 = DEF.option("gridScale").unwrap().default.parse().unwrap();
        assert_eq!(default * GRID_UNIT, 64);
    }

    /// A blinker is the smallest thing that proves the rule: three in a row becomes three in
    /// a column and back again.
    #[test]
    fn a_blinker_oscillates_under_lifes_rule() {
        let mut sim = Automaton::new();
        sim.resize(16, 1.0);
        assert_eq!(alive(&sim), 0);
        for x in 4..7 {
            sim.grid[5 * 16 + x] = ALIVE;
        }
        sim.generation("life");
        let column: Vec<u8> = (4..7).map(|y| sim.grid[y * 16 + 5]).collect();
        assert_eq!(column, [ALIVE, ALIVE, ALIVE], "the blinker turned");
        assert_eq!(alive(&sim), 3, "and nothing else was born");
        sim.generation("life");
        let row: Vec<u8> = (4..7).map(|x| sim.grid[5 * 16 + x]).collect();
        assert_eq!(row, [ALIVE, ALIVE, ALIVE], "and turned back");
    }

    /// The five cells of a Life glider, as offsets from its top-left corner:
    ///
    /// ```text
    /// .O.
    /// ..O
    /// OOO
    /// ```
    ///
    /// It translates by one cell on both axes every four generations, which is what makes it
    /// the thing to walk off an edge with.
    const GLIDER: [(usize, usize); 5] = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    /// Put a glider down with its corner at `(x, y)`, wrapped.
    fn glider_at(sim: &mut Automaton, x: usize, y: usize) {
        let w = sim.size as usize;
        for (dx, dy) in GLIDER {
            sim.grid[(y + dy) % w * w + (x + dx) % w] = ALIVE;
        }
    }

    /// Every live cell, in row-major order.
    fn live_cells(sim: &Automaton) -> Vec<(usize, usize)> {
        let w = sim.size as usize;
        (0..w * w)
            .filter(|i| sim.grid[*i] == ALIVE)
            .map(|i| (i % w, i / w))
            .collect()
    }

    /// The grid is a torus: a glider walked off the right edge and the bottom one comes back
    /// on the left and the top, unchanged and still five cells.
    #[test]
    fn a_glider_crossing_an_edge_comes_back_on_the_other_side() {
        let mut sim = Automaton::new();
        sim.resize(16, 1.0);
        glider_at(&mut sim, 13, 13);
        // Six periods of four generations each: the glider translates by six cells on both
        // axes, which takes it over both far edges of a sixteen-cell world.
        for _ in 0..24 {
            sim.generation("life");
        }
        let mut want = Automaton::new();
        want.resize(16, 1.0);
        glider_at(&mut want, (13 + 6) % 16, (13 + 6) % 16);
        assert_eq!(
            live_cells(&sim),
            live_cells(&want),
            "the glider is not where a torus would have put it"
        );
        assert_eq!(alive(&sim), 5, "and it is still a glider");
    }

    /// A cell in the corner has eight neighbors like any other, and three of them are the
    /// other three corners of the grid. Under Life's B3 a dead cell with exactly three is
    /// born, so the corner comes alive off the far corners alone.
    #[test]
    fn a_corner_cells_neighborhood_counts_the_far_corners() {
        let mut sim = Automaton::new();
        sim.resize(16, 1.0);
        let last = 15;
        for (x, y) in [(last, last), (last, 0), (0, last)] {
            sim.grid[y * 16 + x] = ALIVE;
        }
        sim.generation("life");
        assert_eq!(
            sim.grid[0], ALIVE,
            "the corner counted fewer than the three cells around the torus"
        );
    }

    /// A live cell under Brian's Brain dies through the third state rather than at once,
    /// which is the whole of what makes it three-state.
    #[test]
    fn brians_brain_dies_through_a_third_state() {
        let mut sim = Automaton::new();
        sim.resize(16, 1.0);
        sim.grid[5 * 16 + 5] = ALIVE;
        sim.generation("brians_brain");
        assert_eq!(sim.grid[5 * 16 + 5], DYING);
        sim.generation("brians_brain");
        assert_eq!(sim.grid[5 * 16 + 5], DEAD);
    }

    /// The batch is bounded twice: by silvia's own maximum, and by the cell budget, so the
    /// largest grid runs fewer generations rather than costing the frame.
    #[test]
    fn a_batch_is_bounded_by_the_cells_it_would_cost() {
        let mut sim = Automaton::new();
        sim.resize(64, 1.0);
        assert_eq!(sim.batch(1.0), 1);
        assert_eq!(
            sim.batch(20.0),
            MAX_STEPS,
            "the default grid affords silvia's max"
        );
        assert_eq!(sim.batch(1000.0), MAX_STEPS, "and never more than it");
        sim.resize(256, 1.0);
        let big = sim.batch(20.0);
        assert!(big < MAX_STEPS, "the largest grid is cut to {big}");
        assert!(big >= 1, "and never to nothing");
        assert_eq!(sim.batch(f32::NAN), 1, "a NaN from a cable is one step");
    }

    /// The trail is the channel the picture is mixed by: full under a live cell, decaying
    /// where one has gone.
    #[test]
    fn the_trail_decays_where_a_cell_has_gone() {
        let mut sim = Automaton::new();
        sim.resize(16, 1.0);
        sim.grid[0] = ALIVE;
        sim.redraw(0.5);
        let bytes = sim.frame.bytes().unwrap();
        assert_eq!(bytes[0], 255, "red is the cell");
        assert_eq!(bytes[1], 255, "green is the trail");
        sim.grid[0] = DEAD;
        sim.redraw(0.5);
        let bytes = sim.frame.bytes().unwrap();
        assert_eq!(bytes[0], 0);
        assert_eq!(bytes[1], 127, "half the trail is left");
    }
}
