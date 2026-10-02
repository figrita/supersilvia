// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bricks, a ball and a paddle, played from action inputs and drawn on the CPU.
//!
//! Both halves, in the shape `cellularautomata` and `slimemold` set: the `tick` runs the game
//! and rasterizes it into a small square frame published as a `Texture`, and the picture is
//! WGSL that samples the frame and mixes `Foreground` over `Background` by the ink it finds.
//! silvia's shader clips to the background outside a hairline border, which is what keeps the
//! square field square in an Output of any shape, and that border is kept here.
//!
//! **The paddles are held, not pressed.** An `Action` input carries edges, and a paddle wants
//! a level, so the level is the hand on the button (`ctx.pressed`) or whatever the edges on
//! the port last said (`action::level_after`) — an action input takes many sources and the
//! hand is one more of them.
//!
//! **The four event outputs are one-frame gates.** A brick breaking is a moment rather than a
//! state, and the event half has no pulse: an action fires `Down` when a condition becomes
//! true and `Up` when it stops. So each fires `Down` on the tick it happens and `Up` on the
//! next one, which is the narrowest thing a gate can say.
//!
//! **The field is on the node.** A game is the one node in the library where not being able to
//! watch it is not being able to use it, so the `field` texture is a
//! [`Region::Preview`](super::Region::Preview) under the standard Preview heading — the
//! same band `video` and `maininput` draw their picture in. The knob that launches the ball is
//! `Launch Speed` rather than a second `Ball Speed`, so the thing you set and the thing you
//! read are not the same words a few rows apart.
//!
//! Two of silvia's `values` are dead in silvia — nothing reads `ballSpeed` or `paddleWidth`,
//! which are shadowed by hard-coded numbers — and here they are live ports whose defaults are
//! exactly those numbers. silvia also advances the game once per animation frame with fixed
//! steps; here the step is scaled by `dt x 60`, so the game runs at silvia's pace on a display
//! of any rate.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, UniformNumber, VaryingColor, VaryingNumber};
use crate::nodes::action::level_after;
use crate::nodes::rng::Rng;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Frame, Gate, InputDef, NodeDef, OutputDef,
    OutputKind, Pixels, TickContext,
};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "brickgame",
    category: Category::Generate,
    icon: "🧱",
    label: "Brick Game",
    tooltip: "Classic brick breaking game. Control with action inputs from a sequencer, a \
              button or anything else that fires.",
    inputs: &[
        InputDef {
            key: "leftPaddle",
            label: "Paddle Left",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "rightPaddle",
            label: "Paddle Right",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "startGame",
            label: "Start/Reset",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "pauseGame",
            label: "Pause",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "foregroundColor",
            label: "Foreground",
            ty: VaryingColor,
            control: Control::color("#ffffffff"),
        },
        InputDef {
            key: "backgroundColor",
            label: "Background",
            ty: VaryingColor,
            control: Control::color("#000000ff"),
        },
        InputDef {
            key: "paddleSpeed",
            label: "Paddle Speed",
            ty: UniformNumber,
            control: Control::num(0.02, 0.001, 0.2, 0.001, ""),
        },
        InputDef {
            key: "ballSpeed",
            // **Not "Ball Speed"**, which is what the readout among the outputs is called.
            // This one is what the ball is launched at; that one is how fast it is going now,
            // and two rows a few apart under one name is a node that cannot be read. The key
            // is silvia's `ballSpeed` still, so nothing saved has to be patched.
            label: "Launch Speed",
            ty: UniformNumber,
            control: Control::num(0.008, 0.001, 0.05, 0.001, ""),
        },
        InputDef {
            key: "paddleWidth",
            label: "Paddle Width",
            ty: UniformNumber,
            control: Control::num(0.3, 0.05, 1.0, 0.01, ""),
        },
        InputDef {
            key: "autoReset",
            label: "Auto Reset",
            ty: UniformNumber,
            // silvia's checkbox: a level, so a gate from the graph can arm it.
            control: Control::num(0.0, 0.0, 1.0, 1.0, ""),
        },
        InputDef {
            key: "autoPlay",
            label: "Auto Play",
            ty: UniformNumber,
            control: Control::num(0.0, 0.0, 1.0, 1.0, ""),
        },
    ],
    outputs: &[
        OutputDef {
            key: "field",
            label: "Field",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The game as it was drawn. Worldspace into the frame's own [0,1] by its real
            // aspect, v flipped because rows are uploaded top first — the same mapping the
            // Output's `frame` port uses, which on a square frame is silvia's own
            // `(uv + 1) * 0.5`.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "field");
                let sampler = ctx.sampler(node, "field");
                format!(
                    "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "color",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            // silvia's: the background outside the field's own edge, and the two colors
            // mixed by the ink inside it.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "field");
                let sampler = ctx.sampler(node, "field");
                let foreground = ctx.input(node, "foregroundColor", "uv");
                let background = ctx.input(node, "backgroundColor", "uv");
                format!(
                    "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    let background = {background};
    if (t.x < {EDGE} || t.x > {FAR} || t.y < {EDGE} || t.y > {FAR}) {{
        return background;
    }}
    return mix(background, {foreground}, textureSampleLevel({tex}, {sampler}, t, 0.0).r);"
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "mask",
            label: "Mask",
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            // Coverage: the ink the game drew, and nothing outside the field.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "field");
                let sampler = ctx.sampler(node, "field");
                format!(
                    "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    if (t.x < {EDGE} || t.x > {FAR} || t.y < {EDGE} || t.y > {FAR}) {{
        return 0.0;
    }}
    return textureSampleLevel({tex}, {sampler}, t, 0.0).r;"
                )
            },
            range: Some("[0, 1]"),
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "score",
            label: "Score",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "bricksLeft",
            label: "Bricks Left",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "ballVelocity",
            label: "Ball Speed",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "brickBroken",
            label: "Brick Broken",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "ballLost",
            label: "Ball Lost",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "gameWon",
            label: "Game Won",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "gameStarted",
            label: "Game Started",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
    ],
    options: &[crate::nodes::SHOW_PREVIEW],
    // The game on the node, under the same tick every other source's picture is under. A
    // game you cannot watch is a game you cannot play, and until this was here the only way
    // to see the field was to wire it to an Output and put that Output on a deck.
    regions: &[crate::nodes::Region::Preview("field")],
    width: Some(240.0),
    cpu: Some(CpuDef {
        create: || Box::new(Game::new()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The hairline silvia's shader clips at, so the square field has an edge of its own in an
/// Output of any shape.
const EDGE: &str = "0.004";
const FAR: &str = "0.996";

/// Eight across, six down, silvia's.
const COLUMNS: usize = 8;
const ROWS: usize = 6;
const BRICKS: usize = COLUMNS * ROWS;

/// Where one brick's center is, in worldspace. silvia's own arithmetic.
fn brick_at(i: usize) -> (f32, f32) {
    let (row, column) = (i / COLUMNS, i % COLUMNS);
    (-0.875 + column as f32 * 0.25, 0.8 - row as f32 * 0.15)
}

/// Half a brick, in worldspace: the collision box and the drawn rectangle are the same thing.
const BRICK_HALF: (f32, f32) = (0.1, 0.05);

/// The ball's radius, the paddle's height and where the paddle sits, in worldspace.
const BALL_RADIUS: f32 = 0.02;
const PADDLE_Y: f32 = -0.85;

/// The frame the game is drawn into: square, because the world it is played in is.
const SIDE: u32 = 300;

/// The longest step one tick takes, in silvia's own frames of 1/60 s. A frame that arrived
/// late moves the ball four of silvia's rather than however many it missed, so nothing
/// tunnels through a brick.
const MAX_STEP: f32 = 4.0;

/// The four action outputs, in declaration order, and the index of each.
const PULSES: [&str; 4] = ["brickBroken", "ballLost", "gameWon", "gameStarted"];
const BRICK_BROKEN: usize = 0;
const BALL_LOST: usize = 1;
const GAME_WON: usize = 2;
const GAME_STARTED: usize = 3;

/// Everything the update reads, gathered once.
// A game's inputs are flags, and naming a two-state thing anything but a bool would be a
// worse name for it.
#[allow(clippy::struct_excessive_bools)]
struct Inputs {
    /// How far the game moves this tick, in silvia's frames.
    step: f32,
    paddle_speed: f32,
    ball_speed: f32,
    paddle_width: f32,
    auto_reset: bool,
    auto_play: bool,
    left: bool,
    right: bool,
}

// The same: what a game is, is a handful of flags.
#[allow(clippy::struct_excessive_bools)]
struct Game {
    running: bool,
    paused: bool,
    ball: (f32, f32),
    velocity: (f32, f32),
    paddle: f32,
    /// One per brick: true while it is still there.
    bricks: [bool; BRICKS],
    remaining: u32,
    score: u32,
    /// The level the cables on each paddle last reported, which the hand is added to.
    left_cable: bool,
    right_cable: bool,
    start: Gate,
    pause: Gate,
    /// An action that fired `Down` last tick and owes an `Up` this one.
    owed: [bool; 4],
    /// The last frame published, so a paused game costs no upload.
    frame: Arc<Frame>,
    /// Whether the picture moved since it was last drawn.
    dirty: bool,
    rng: Rng,
}

impl Game {
    fn new() -> Self {
        Self {
            running: false,
            paused: false,
            ball: (0.0, -0.7),
            velocity: (0.006, 0.006),
            paddle: 0.0,
            bricks: [true; BRICKS],
            remaining: BRICKS as u32,
            score: 0,
            left_cable: false,
            right_cable: false,
            start: Gate::default(),
            pause: Gate::default(),
            owed: [false; 4],
            frame: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            dirty: true,
            rng: Rng::new(),
        }
    }

    /// silvia's `_initGame`: the ball back at the bottom with a fresh velocity, every brick
    /// back, and the score kept only when the reset followed a win.
    fn reset(&mut self, won: bool, speed: f32) {
        self.ball = (0.0, -0.7);
        // silvia's spread, with its own hard-coded 0.008 as the scale `Ball Speed` names:
        // sideways within half the speed either way, upward between half of it and all of it.
        self.velocity = (
            (self.rng.next_f32() - 0.5) * speed,
            (0.5 + self.rng.next_f32() * 0.5) * speed,
        );
        self.paddle = 0.0;
        self.bricks = [true; BRICKS];
        self.remaining = BRICKS as u32;
        if !won {
            self.score = 0;
        }
        self.running = false;
        self.paused = false;
        self.dirty = true;
    }

    /// silvia's `_startGame`: the first press starts, a press while running restarts.
    fn begin(&mut self, speed: f32) -> bool {
        if self.running {
            self.reset(false, speed);
            self.running = true;
            false
        } else {
            self.running = true;
            self.paused = false;
            true
        }
    }

    /// One update. Returns which of the four actions happened.
    fn update(&mut self, p: &Inputs) -> [bool; 4] {
        let mut fired = [false; 4];
        let half = p.paddle_width * 0.5;

        if p.auto_play {
            self.autoplay(p, half);
        } else {
            if p.left && self.paddle > -1.0 + half {
                self.paddle -= p.paddle_speed * p.step;
            }
            if p.right && self.paddle < 1.0 - half {
                self.paddle += p.paddle_speed * p.step;
            }
        }

        self.ball.0 += self.velocity.0 * p.step;
        self.ball.1 += self.velocity.1 * p.step;
        self.dirty = true;

        if self.ball.0 <= -1.0 || self.ball.0 >= 1.0 {
            self.velocity.0 = -self.velocity.0;
        }
        if self.ball.1 >= 1.0 {
            self.velocity.1 = -self.velocity.1;
        }

        // Off the bottom: the ball is lost, and the game either restarts or waits to be
        // started again.
        if self.ball.1 <= -1.0 {
            fired[BALL_LOST] = true;
            self.reset(false, p.ball_speed);
            self.running = p.auto_reset;
            return fired;
        }

        // The paddle, with silvia's curve: where it was hit decides the angle and adds a
        // little speed.
        if self.ball.1 <= -0.8 && self.ball.1 >= -0.9 && (self.ball.0 - self.paddle).abs() < half {
            self.velocity.1 = self.velocity.1.abs();
            let hit = (self.ball.0 - self.paddle) / half;
            let boost = 1.0 + hit.abs() * 0.02;
            self.velocity.0 = hit * 0.006 * boost;
            self.velocity.1 *= boost;
        }

        // One brick a tick, silvia's: the first the ball is inside breaks, and the ball turns.
        for i in 0..BRICKS {
            if !self.bricks[i] {
                continue;
            }
            let (bx, by) = brick_at(i);
            if self.ball.0 + BALL_RADIUS > bx - BRICK_HALF.0
                && self.ball.0 - BALL_RADIUS < bx + BRICK_HALF.0
                && self.ball.1 + BALL_RADIUS > by - BRICK_HALF.1
                && self.ball.1 - BALL_RADIUS < by + BRICK_HALF.1
            {
                self.bricks[i] = false;
                self.remaining -= 1;
                self.score += 10;
                self.velocity.1 = -self.velocity.1;
                fired[BRICK_BROKEN] = true;
                break;
            }
        }

        if self.remaining == 0 {
            fired[GAME_WON] = true;
            self.reset(true, p.ball_speed);
            self.running = p.auto_reset;
        }
        fired
    }

    /// silvia's AI: while the ball is coming down, predict where it crosses the paddle's line
    /// — bouncing the prediction off a wall if it would land past one — and move toward it
    /// faster the further away it is.
    fn autoplay(&mut self, p: &Inputs, half: f32) {
        if self.velocity.1 >= 0.0 || self.ball.1 >= 0.2 {
            return;
        }
        let time = (self.ball.1 - PADDLE_Y) / self.velocity.1;
        let predicted = self.ball.0 + self.velocity.0 * time;
        let target = if predicted < -1.0 {
            -2.0 - predicted
        } else if predicted > 1.0 {
            2.0 - predicted
        } else {
            predicted
        };
        let distance = target - self.paddle;
        if distance.abs() <= 0.1 {
            return;
        }
        let fast = p.paddle_speed * 2.0;
        let slow = p.paddle_speed * 0.3;
        let speed = (slow + (fast - slow) * (distance.abs() / 0.5).min(1.0)) * p.step;
        if distance > 0.0 && self.paddle < 1.0 - half {
            self.paddle = (self.paddle + speed).min(1.0 - half);
        } else if distance < 0.0 && self.paddle > -1.0 + half {
            self.paddle = (self.paddle - speed).max(-1.0 + half);
        }
    }

    /// The game, rasterized: white ink on black, rows top first.
    fn draw(&self, paddle_width: f32) -> Frame {
        let mut pixels = vec![0u8; (SIDE * SIDE * 4) as usize];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        let scale = SIDE as f32 * 0.5;
        let to_x = |x: f32| (x + 1.0) * scale;
        let to_y = |y: f32| (1.0 - y) * scale;

        let mut ink = |x0: f32, y0: f32, x1: f32, y1: f32| {
            let x0 = x0.max(0.0) as u32;
            let y0 = y0.max(0.0) as u32;
            let x1 = (x1.max(0.0) as u32).min(SIDE);
            let y1 = (y1.max(0.0) as u32).min(SIDE);
            for y in y0..y1 {
                for x in x0..x1.min(SIDE) {
                    let at = ((y * SIDE + x) * 4) as usize;
                    pixels[at] = 255;
                    pixels[at + 1] = 255;
                    pixels[at + 2] = 255;
                }
            }
        };

        for (i, there) in self.bricks.iter().enumerate() {
            if !there {
                continue;
            }
            let (bx, by) = brick_at(i);
            ink(
                to_x(bx - BRICK_HALF.0),
                to_y(by + BRICK_HALF.1),
                to_x(bx + BRICK_HALF.0),
                to_y(by - BRICK_HALF.1),
            );
        }

        let half = paddle_width * 0.5;
        ink(
            to_x(self.paddle - half),
            to_y(PADDLE_Y) - 3.0,
            to_x(self.paddle + half),
            to_y(PADDLE_Y) + 3.0,
        );

        // The ball, a filled circle rather than its box: it is the one round thing on the
        // field and a square ball reads as a brick.
        let (cx, cy) = (to_x(self.ball.0), to_y(self.ball.1));
        let radius = BALL_RADIUS * scale;
        let rows = ((cy - radius).max(0.0) as u32)..(((cy + radius).max(0.0) as u32) + 1).min(SIDE);
        for y in rows {
            let row = y as f32;
            let dy = row + 0.5 - cy;
            let span = (radius * radius - dy * dy).max(0.0).sqrt();
            ink(cx - span, row, cx + span, row + 1.0);
        }

        Frame {
            width: SIDE,
            height: SIDE,
            pixels: Pixels::Bytes(pixels),
        }
    }
}

impl CpuNode for Game {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.rng.seed(id);
        // Last tick's events close: a pulse is a gate that opens for one frame.
        for (i, port) in PULSES.into_iter().enumerate() {
            if std::mem::take(&mut self.owed[i]) {
                ctx.fire(id, port, Edge::Up);
            }
        }

        let speed = ctx.input(id, "ballSpeed").max(0.0001);
        let mut fired = [false; 4];
        if ctx.downs(id, "startGame", &mut self.start) > 0 && self.begin(speed) {
            fired[GAME_STARTED] = true;
        }
        if ctx.downs(id, "pauseGame", &mut self.pause) % 2 == 1 && self.running {
            self.paused = !self.paused;
        }

        // A paddle is a level: the edges that arrived on the port, and the hand on its button
        // as one more source.
        self.left_cable = level_after(self.left_cable, &ctx.edges(id, "leftPaddle"));
        self.right_cable = level_after(self.right_cable, &ctx.edges(id, "rightPaddle"));
        let play = Inputs {
            step: (ctx.dt * 60.0).clamp(0.0, MAX_STEP),
            paddle_speed: ctx.input(id, "paddleSpeed").max(0.0),
            ball_speed: speed,
            paddle_width: ctx.input(id, "paddleWidth").clamp(0.02, 2.0),
            auto_reset: ctx.input(id, "autoReset") >= 0.5,
            auto_play: ctx.input(id, "autoPlay") >= 0.5,
            left: self.left_cable || ctx.pressed(id, "leftPaddle"),
            right: self.right_cable || ctx.pressed(id, "rightPaddle"),
        };

        // silvia's Auto Reset starts a game that is not running, which is what makes the
        // checkbox alone enough to play.
        if play.auto_reset && !self.running && self.begin(speed) {
            fired[GAME_STARTED] = true;
        }

        if self.running && !self.paused {
            for (happened, also) in fired.iter_mut().zip(self.update(&play)) {
                *happened |= also;
            }
        }

        for (i, port) in PULSES.into_iter().enumerate() {
            if fired[i] {
                ctx.fire(id, port, Edge::Down);
                self.owed[i] = true;
            }
        }

        ctx.publish(id, "score", self.score as f32);
        ctx.publish(id, "bricksLeft", self.remaining as f32);
        let (vx, vy) = self.velocity;
        ctx.publish(id, "ballVelocity", vx.hypot(vy));

        if self.dirty {
            self.frame = Arc::new(self.draw(play.paddle_width));
            self.dirty = false;
        }
        ctx.publish_frame(id, "field", Arc::clone(&self.frame));
    }

    fn debug(&self) -> Option<String> {
        Some(format!(
            "brickgame {}, score {}, {} bricks, ball ({:.3}, {:.3})",
            if !self.running {
                "stopped"
            } else if self.paused {
                "paused"
            } else {
                "playing"
            },
            self.score,
            self.remaining,
            self.ball.0,
            self.ball.1,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play() -> Inputs {
        Inputs {
            step: 1.0,
            paddle_speed: 0.02,
            ball_speed: 0.008,
            paddle_width: 0.3,
            auto_reset: false,
            auto_play: false,
            left: false,
            right: false,
        }
    }

    /// silvia's grid: forty-eight bricks in eight columns of six, the first at the top left.
    #[test]
    fn the_bricks_are_where_silvia_puts_them() {
        assert_eq!(BRICKS, 48);
        assert_eq!(brick_at(0), (-0.875, 0.8));
        assert_eq!(brick_at(7), (0.875, 0.8));
        assert_eq!(brick_at(8), (-0.875, 0.65));
        let (x, y) = brick_at(BRICKS - 1);
        assert!(
            (x - 0.875).abs() < 1e-6 && (y - (0.8 - 0.75)).abs() < 1e-6,
            "{x} {y}"
        );
    }

    /// A fresh game is stopped, unscathed and unscored, and its ball is under the bricks.
    #[test]
    fn a_new_game_waits_to_be_started() {
        let game = Game::new();
        assert!(!game.running && !game.paused);
        assert_eq!((game.score, game.remaining), (0, 48));
        assert_eq!(game.ball, (0.0, -0.7));
    }

    /// The ball's opening velocity is silvia's spread, scaled by the port that names it.
    #[test]
    fn the_ball_starts_at_the_speed_it_was_given() {
        let mut game = Game::new();
        game.rng.seed(crate::graph::NodeId(1));
        for speed in [0.008, 0.02] {
            for _ in 0..16 {
                game.reset(false, speed);
                let (vx, vy) = game.velocity;
                assert!(vx.abs() <= speed * 0.5, "{vx} sideways at {speed}");
                assert!(vy >= speed * 0.5 && vy <= speed, "{vy} upward at {speed}");
            }
        }
    }

    /// A brick the ball reaches breaks, once, and scores silvia's ten.
    #[test]
    fn a_brick_the_ball_reaches_breaks_and_scores() {
        let mut game = Game::new();
        game.running = true;
        let (bx, by) = brick_at(BRICKS - COLUMNS); // The bottom row, leftmost.
        game.ball = (bx, by - BRICK_HALF.1 - BALL_RADIUS * 0.5);
        game.velocity = (0.0, 0.006);
        let fired = game.update(&play());
        assert!(fired[BRICK_BROKEN], "the brick survived");
        assert_eq!(game.remaining, 47);
        assert_eq!(game.score, 10);
        assert!(game.velocity.1 < 0.0, "the ball turned around");
    }

    /// Off the bottom is a lost ball: the event fires, the game resets, and it only carries on
    /// where Auto Reset says so.
    #[test]
    fn a_ball_off_the_bottom_is_lost() {
        for auto in [false, true] {
            let mut game = Game::new();
            game.running = true;
            game.ball = (0.0, -0.999);
            game.velocity = (0.0, -0.01);
            let mut p = play();
            p.auto_reset = auto;
            let fired = game.update(&p);
            assert!(fired[BALL_LOST]);
            assert_eq!(game.ball, (0.0, -0.7), "the ball is back");
            assert_eq!(game.running, auto, "auto reset {auto}");
        }
    }

    /// The last brick wins, and a win keeps the score where a loss clears it.
    #[test]
    fn the_last_brick_wins_and_the_score_survives_it() {
        let mut game = Game::new();
        game.running = true;
        game.bricks = [false; BRICKS];
        game.remaining = 1;
        game.score = 470;
        let i = BRICKS - COLUMNS;
        game.bricks[i] = true;
        let (bx, by) = brick_at(i);
        game.ball = (bx, by - BRICK_HALF.1 - BALL_RADIUS * 0.5);
        game.velocity = (0.0, 0.006);
        let fired = game.update(&play());
        assert!(fired[BRICK_BROKEN] && fired[GAME_WON]);
        assert_eq!(game.score, 480, "a win keeps the score");
        assert_eq!(game.remaining, 48, "and the wall is back");
    }

    /// The paddle moves on a held level and stops at the wall, whichever way it went.
    #[test]
    fn a_held_paddle_moves_and_stops_at_the_wall() {
        let mut game = Game::new();
        game.running = true;
        let mut p = play();
        p.left = true;
        for _ in 0..200 {
            game.update(&p);
        }
        assert!(game.paddle >= -1.0, "{} is off the field", game.paddle);
        assert!(game.paddle < -0.8, "{} never got there", game.paddle);
        p.left = false;
        p.right = true;
        for _ in 0..400 {
            game.update(&p);
        }
        assert!(game.paddle <= 1.0 && game.paddle > 0.8, "{}", game.paddle);
    }

    /// Auto Play keeps the ball up: a hundred and fifty frames of it and the ball is still in
    /// play, which a paddle standing still would not manage.
    #[test]
    fn auto_play_returns_the_ball() {
        let mut game = Game::new();
        game.rng.seed(crate::graph::NodeId(1));
        game.running = true;
        game.ball = (0.5, 0.0);
        game.velocity = (0.004, -0.006);
        let mut p = play();
        p.auto_play = true;
        let mut lost = 0;
        for _ in 0..150 {
            if game.update(&p)[BALL_LOST] {
                lost += 1;
            }
        }
        assert_eq!(lost, 0, "the paddle missed");
    }

    /// The picture is the game: ink where the bricks, the paddle and the ball are, and black
    /// where they are not.
    #[test]
    fn the_drawn_field_has_the_bricks_and_the_paddle_in_it() {
        let game = Game::new();
        let frame = game.draw(0.3);
        assert_eq!((frame.width, frame.height), (SIDE, SIDE));
        let bytes = frame.bytes().unwrap();
        let at = |x: f32, y: f32| {
            let px = (f32::midpoint(x, 1.0) * SIDE as f32) as usize;
            let py = ((1.0 - y) * 0.5 * SIDE as f32) as usize;
            bytes[(py * SIDE as usize + px) * 4]
        };
        let (bx, by) = brick_at(0);
        assert_eq!(at(bx, by), 255, "the first brick");
        assert_eq!(at(0.0, PADDLE_Y), 255, "the paddle");
        assert_eq!(at(0.0, -0.7), 255, "the ball");
        assert_eq!(at(0.0, 0.0), 0, "and nothing in the middle of the field");
        // Alpha is opaque everywhere, so the picture is not read through a hole.
        assert!(bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }
}
