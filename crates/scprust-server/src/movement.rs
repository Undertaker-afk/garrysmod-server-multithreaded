//! Deterministic, bounded AABB movement over static BSP geometry. This is a
//! Quake/Source-inspired controller, **not** a Source movement reimplementation:
//! there is no player animation state, water/ladder handling, ducking, bhop
//! tuning, NPC/vehicle simulation, prop physics or network prediction.
use crate::{
    bsp::{dot, valid_vector, Error, Map, Result, Vec3},
    collision::{Sweep, CONTENTS_SOLID},
};

/// Source-style player hull: feet at the origin, 32x32 wide and 72 tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub mins: Vec3,
    pub maxs: Vec3,
}
impl Hull {
    pub const PLAYER: Self = Self {
        mins: [-16.0, -16.0, 0.0],
        maxs: [16.0, 16.0, 72.0],
    };
    fn validate(&self) -> Result<()> {
        if (0..3).any(|i| {
            !self.mins[i].is_finite()
                || !self.maxs[i].is_finite()
                || self.mins[i] > self.maxs[i]
                || self.mins[i].abs() > 65_536.0
                || self.maxs[i].abs() > 65_536.0
        }) || self.mins[2] != 0.0
        {
            return Err(Error("invalid movement hull (feet must be at z=0)"));
        }
        Ok(())
    }
}

/// Approximate GMod/Source defaults. Values are configurable, not parity claims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveConfig {
    pub gravity: f64,
    pub max_speed: f64,
    pub accelerate: f64,
    pub air_accelerate: f64,
    pub friction: f64,
    pub stop_speed: f64,
    pub jump_speed: f64,
    pub step_height: f64,
    pub max_slope_cos: f64,
}
impl Default for MoveConfig {
    fn default() -> Self {
        Self {
            gravity: 600.0,
            max_speed: 250.0,
            accelerate: 10.0,
            air_accelerate: 10.0,
            friction: 4.0,
            stop_speed: 100.0,
            jump_speed: 220.0,
            step_height: 18.0,
            max_slope_cos: 0.7,
        }
    }
}
impl MoveConfig {
    fn validate(&self) -> Result<()> {
        let finite = [
            self.gravity,
            self.max_speed,
            self.accelerate,
            self.air_accelerate,
            self.friction,
            self.stop_speed,
            self.jump_speed,
            self.step_height,
            self.max_slope_cos,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite
            || !(0.0..=2000.0).contains(&self.gravity)
            || !(0.0..=2000.0).contains(&self.max_speed)
            || !(0.0..=100.0).contains(&self.accelerate)
            || !(0.0..=100.0).contains(&self.air_accelerate)
            || !(0.0..=100.0).contains(&self.friction)
            || !(0.0..=1000.0).contains(&self.stop_speed)
            || !(0.0..=1000.0).contains(&self.jump_speed)
            || !(0.0..=64.0).contains(&self.step_height)
            || !(0.1..=1.0).contains(&self.max_slope_cos)
        {
            return Err(Error("movement configuration out of range"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub ground_normal: Option<Vec3>,
}
impl PlayerState {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: [0.0; 3],
            on_ground: false,
            ground_normal: None,
        }
    }
    fn validate(&self) -> Result<()> {
        if !valid_vector(self.position)
            || !valid_vector(self.velocity)
            || self.velocity.iter().any(|v| v.abs() > 10_000.0)
            || self.ground_normal.is_some_and(|n| !valid_vector(n))
        {
            return Err(Error("invalid player state"));
        }
        Ok(())
    }
}

/// Wish direction is a unit vector in world space; wish speed is clamped to
/// `MoveConfig::max_speed`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveInput {
    pub wish_dir: Vec3,
    pub wish_speed: f64,
    pub jump: bool,
}
impl MoveInput {
    pub const IDLE: Self = Self {
        wish_dir: [0.0, 0.0, 0.0],
        wish_speed: 0.0,
        jump: false,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveReport {
    pub substeps: usize,
    pub clipped: bool,
    pub stepped: bool,
    pub landed: bool,
}

/// Extra trace separation. Only affects trace *fractions*, never contact
/// geometry, so resting height stays exact.
const SKIN: f64 = 0.03125;
/// Fixed upper bound on substep length: results do not depend on floating-point
/// division of the caller's frame time beyond this granularity.
const MAX_SUBSTEP: f64 = 1.0 / 60.0;
const MAX_SUBSTEPPED_DT: f64 = 0.25;
const MAX_SLIDE_PLANES: usize = 4;

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(v: Vec3, s: f64) -> Vec3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn trace(map: &Map, hull: &Hull, from: Vec3, to: Vec3) -> Result<crate::collision::Trace> {
    let skin = [0.0, 0.0, SKIN];
    let mut hit = map.trace_solids(Sweep {
        start: add(from, skin),
        end: add(to, skin),
        mins: hull.mins,
        maxs: hull.maxs,
        contents_mask: CONTENTS_SOLID,
    })?;
    if hit.fraction >= 1.0 {
        hit.end_position = to;
    } else if hit.start_solid {
        // Start-solid means "no displacement"; do not leak the probe offset
        // into the reported position.
        hit.end_position = from;
    } else if !hit.plane_normal.is_some_and(|n| n[2] >= 0.7) {
        // Non-walkable hits (walls/ceilings) should not inherit the vertical
        // floor-separation offset, or repeated wall contact ratchets z upward.
        hit.end_position = sub(hit.end_position, skin);
    }
    Ok(hit)
}

fn ground(map: &Map, hull: &Hull, position: Vec3) -> Result<Option<(Option<Vec3>, bool)>> {
    let hit = trace(map, hull, position, sub(position, [0.0, 0.0, 2.0]))?;
    // `start_solid` means the hull is already interpenetrating geometry. That is
    // reported to the caller as a failure instead of inventing a ground normal.
    if hit.start_solid || hit.all_solid {
        return Ok(Some((None, true)));
    }
    if hit.fraction >= 1.0 {
        return Ok(None);
    }
    Ok(Some((hit.plane_normal, false)))
}

fn friction(velocity: &mut Vec3, cfg: &MoveConfig, dt: f64) {
    let speed = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
    if speed < 1e-6 {
        velocity[0] = 0.0;
        velocity[1] = 0.0;
        return;
    }
    let control = speed.max(cfg.stop_speed);
    let drop = control * cfg.friction * dt;
    let new_speed = (speed - drop).max(0.0);
    let factor = new_speed / speed;
    velocity[0] *= factor;
    velocity[1] *= factor;
}

fn accelerate(velocity: &mut Vec3, wish_dir: Vec3, wish_speed: f64, accel: f64, dt: f64) {
    let current = dot(*velocity, wish_dir);
    let add_speed = wish_speed - current;
    if add_speed <= 0.0 {
        return;
    }
    let accel_speed = (accel * wish_speed * dt).min(add_speed);
    for i in 0..3 {
        velocity[i] += wish_dir[i] * accel_speed;
    }
}

/// Attempt to step over a low obstruction. Returns the new feet position only if
/// the move gains height and lands on a walkable surface within `step_height`.
fn try_step(
    map: &Map,
    hull: &Hull,
    cfg: &MoveConfig,
    position: Vec3,
    remaining: Vec3,
) -> Result<Option<Vec3>> {
    let horizontal = [remaining[0], remaining[1], 0.0];
    if horizontal[0] == 0.0 && horizontal[1] == 0.0 {
        return Ok(None);
    }
    let raised = add(position, [0.0, 0.0, cfg.step_height]);
    if trace(map, hull, position, raised)?.fraction < 1.0 {
        return Ok(None);
    }
    let forward = trace(map, hull, raised, add(raised, horizontal))?;
    if forward.start_solid {
        return Ok(None);
    }
    let ahead = add(raised, scale(horizontal, forward.fraction));
    let down = trace(
        map,
        hull,
        ahead,
        sub(ahead, [0.0, 0.0, cfg.step_height + 0.125]),
    )?;
    if down.fraction >= 1.0 || down.start_solid {
        return Ok(None);
    }
    let Some(normal) = down.plane_normal else {
        return Ok(None);
    };
    let landed = down.end_position;
    if normal[2] < cfg.max_slope_cos
        || landed[2] <= position[2] + 0.01
        || landed[2] > position[2] + cfg.step_height + 0.125
    {
        return Ok(None);
    }
    Ok(Some(landed))
}

/// Advance movement by sliding the hull along contacted planes. Returns
/// (clipped, stepped).
fn move_and_slide(
    map: &Map,
    hull: &Hull,
    cfg: &MoveConfig,
    state: &mut PlayerState,
    on_ground: bool,
    dt: f64,
) -> Result<(bool, bool)> {
    let mut remaining = scale(state.velocity, dt);
    let mut planes: Vec<Vec3> = Vec::new();
    let mut clipped = false;
    for _ in 0..MAX_SLIDE_PLANES {
        let length = dot(remaining, remaining).sqrt();
        if length < 1e-9 {
            break;
        }
        let target = add(state.position, remaining);
        let hit = trace(map, hull, state.position, target)?;
        if hit.fraction >= 1.0 {
            state.position = target;
            break;
        }
        if on_ground && planes.is_empty() && hit.plane_normal.is_some() {
            if let Some(stepped) = try_step(map, hull, cfg, state.position, remaining)? {
                state.position = stepped;
                state.velocity[2] = 0.0;
                return Ok((clipped, true));
            }
        }
        if hit.start_solid || hit.all_solid {
            state.position = hit.end_position;
            state.velocity = [0.0; 3];
            return Ok((true, false));
        }
        let Some(normal) = hit.plane_normal else {
            break;
        };
        if planes.iter().any(|seen| dot(*seen, normal) > 0.999) {
            break;
        }
        let leftover = 1.0 - hit.fraction;
        // Keep a tiny clearance off non-floor planes so resting against a wall
        // does not leave the hull flush (distance 0) and trigger start_solid
        // on ground probes or wall-sliding moves.
        let mut landed = hit.end_position;
        if normal[2] < cfg.max_slope_cos {
            let into_wall = -dot(sub(landed, state.position), normal);
            if into_wall > 0.0 {
                landed = add(landed, scale(normal, into_wall.min(1e-4)));
            }
        }
        state.position = landed;
        clipped = true;
        let velocity_into = dot(state.velocity, normal);
        if velocity_into < 0.0 {
            state.velocity = sub(state.velocity, scale(normal, velocity_into));
        }
        remaining = scale(remaining, leftover);
        let into = dot(remaining, normal);
        if into < 0.0 {
            remaining = sub(remaining, scale(normal, into));
        }
        planes.push(normal);
    }
    Ok((clipped, false))
}

/// Advance one bounded movement frame. `dt` is clamped to 0.25 s; larger values
/// must be split by the caller so a single frame cannot tunnel arbitrarily.
pub fn step(
    map: &Map,
    hull: &Hull,
    cfg: &MoveConfig,
    state: &mut PlayerState,
    input: &MoveInput,
    dt: f64,
) -> Result<MoveReport> {
    hull.validate()?;
    cfg.validate()?;
    state.validate()?;
    if !dt.is_finite() || !(0.0..=MAX_SUBSTEPPED_DT).contains(&dt) {
        return Err(Error("invalid movement timestep"));
    }
    if !valid_vector(input.wish_dir)
        || !input.wish_speed.is_finite()
        || !(0.0..=2000.0).contains(&input.wish_speed)
    {
        return Err(Error("invalid movement input"));
    }
    let wish_len_sq = dot(input.wish_dir, input.wish_dir);
    if wish_len_sq > 1.0 + 1e-6 {
        return Err(Error("wish direction is not a unit vector"));
    }
    if dt == 0.0 {
        return Ok(MoveReport::default());
    }
    let substeps = ((dt / MAX_SUBSTEP).ceil() as usize).max(1);
    let substep_dt = dt / substeps as f64;
    let mut report = MoveReport {
        substeps,
        ..MoveReport::default()
    };
    for _ in 0..substeps {
        let was_ground = state.on_ground;
        let ground_probe = ground(map, hull, state.position)?;
        let on_ground =
            matches!(&ground_probe, Some((Some(normal), false)) if normal[2] >= cfg.max_slope_cos);
        state.ground_normal = match ground_probe {
            Some((normal, false)) => normal,
            _ => None,
        };
        state.on_ground = on_ground;
        if !on_ground && matches!(&state.ground_normal, Some(n) if n[2] < cfg.max_slope_cos) {
            // Too steep to stand on: treat as airborne so gravity keeps applying.
            state.ground_normal = None;
        }
        if on_ground {
            friction(&mut state.velocity, cfg, substep_dt);
        }
        let accel = if on_ground {
            cfg.accelerate
        } else {
            cfg.air_accelerate
        };
        accelerate(
            &mut state.velocity,
            input.wish_dir,
            input.wish_speed.min(cfg.max_speed),
            accel,
            substep_dt,
        );
        let mut jumping = false;
        if input.jump && on_ground {
            state.velocity[2] = cfg.jump_speed;
            state.on_ground = false;
            jumping = true;
        }
        if jumping || !on_ground {
            state.velocity[2] -= cfg.gravity * substep_dt;
        } else if state.velocity[2] < 0.0 {
            state.velocity[2] = 0.0;
        }
        let (clipped, stepped) = move_and_slide(map, hull, cfg, state, on_ground, substep_dt)?;
        report.clipped |= clipped;
        report.stepped |= stepped;
        if on_ground && !jumping && state.velocity[2] <= 0.0 {
            // Snap down over small drops instead of floating off ledges.
            let probe = trace(
                map,
                hull,
                state.position,
                sub(state.position, [0.0, 0.0, cfg.step_height]),
            )?;
            if probe.fraction < 1.0 && !probe.start_solid {
                if let Some(normal) = probe.plane_normal {
                    if normal[2] >= cfg.max_slope_cos {
                        state.position = probe.end_position;
                        state.velocity[2] = 0.0;
                    }
                }
            }
        }
        let still = ground(map, hull, state.position)?;
        let grounded_now = matches!(&still, Some((Some(n), false)) if n[2] >= cfg.max_slope_cos);
        if !was_ground && grounded_now {
            report.landed = true;
        }
        state.on_ground = grounded_now;
        state.ground_normal = match still {
            Some((normal, false)) => normal,
            _ => None,
        };
    }
    Ok(report)
}
