//! Spawn-point extraction from the entity lump with static-geometry validation.
//! This is entity parsing plus tracing, **not** Source spawning semantics: entity
//! I/O, point entities, triggers, filters, spawn room logic and GMod player setup
//! are not implemented.
use crate::{
    bsp::{valid_vector, Error, Map, Result, Vec3},
    collision::{Sweep, CONTENTS_SOLID},
};
pub const SPAWN_CLASSES: [&str; 6] = [
    "info_player_start",
    "info_player_terrorist",
    "info_player_counterterrorist",
    "info_player_rebel",
    "info_player_combine",
    "info_player_deathmatch",
];
pub const PLAYER_MINS: Vec3 = [-16.0, -16.0, 0.0];
pub const PLAYER_MAXS: Vec3 = [16.0, 16.0, 72.0];
/// Ground search depth below the entity origin.
pub const GROUND_SEARCH: f64 = 128.0;
pub const MIN_GROUND_NORMAL: f64 = 0.7;
pub const MAX_SPAWN_POINTS: usize = 1024;
/// Spawn origins normally sit exactly on the floor. The probe starts slightly
/// above the origin so legitimate contact is not reported as interpenetration.
const PROBE_SKIN: f64 = 0.03125;

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPoint {
    pub classname: String,
    pub position: Vec3,
    pub angles: Vec3,
    /// Ground exists below this origin within `GROUND_SEARCH`.
    pub on_ground: bool,
    /// Hull is unobstructed and standing on walkable static geometry.
    pub valid: bool,
    pub reason: &'static str,
}
fn parse_vector(value: &[u8]) -> Result<Vec3> {
    let parts: Vec<&[u8]> = value
        .split(|b| b.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return Err(Error("spawn origin/angles is not a three-number vector"));
    }
    let mut out = [0.0; 3];
    for (slot, part) in out.iter_mut().zip(parts) {
        *slot = std::str::from_utf8(part)
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or(Error("invalid spawn vector number"))?;
    }
    if !valid_vector(out) {
        return Err(Error("out-of-bounds spawn vector"));
    }
    Ok(out)
}
impl Map {
    /// Spawn candidates in deterministic `(classname, position)` order. Invalid
    /// points are returned with a reason instead of being silently dropped.
    pub fn spawn_points(&self) -> Result<Vec<SpawnPoint>> {
        let mut points = Vec::new();
        for entity in &self.entities {
            let Some(class) = entity.get(b"classname") else {
                continue;
            };
            let Ok(classname) = std::str::from_utf8(class) else {
                continue;
            };
            if !SPAWN_CLASSES.contains(&classname) {
                continue;
            }
            if points.len() >= MAX_SPAWN_POINTS {
                return Err(Error("too many spawn points"));
            }
            let Some(origin) = entity.get(b"origin") else {
                points.push(SpawnPoint {
                    classname: classname.to_owned(),
                    position: [0.0; 3],
                    angles: [0.0; 3],
                    on_ground: false,
                    valid: false,
                    reason: "missing origin",
                });
                continue;
            };
            let position = parse_vector(origin)?;
            let angles = match entity.get(b"angles") {
                Some(value) => parse_vector(value)?,
                None => [0.0; 3],
            };
            let hit = self.trace_solids(Sweep {
                start: [position[0], position[1], position[2] + PROBE_SKIN],
                end: [
                    position[0],
                    position[1],
                    position[2] + PROBE_SKIN - GROUND_SEARCH,
                ],
                mins: PLAYER_MINS,
                maxs: PLAYER_MAXS,
                contents_mask: CONTENTS_SOLID,
            })?;
            let normal_ok = hit
                .plane_normal
                .is_some_and(|normal| normal[2] >= MIN_GROUND_NORMAL);
            let obstructed = hit.start_solid || hit.all_solid;
            let on_ground = hit.fraction < 1.0 && normal_ok;
            let (valid, reason) = if obstructed {
                (false, "hull obstructed or embedded in solid at origin")
            } else if hit.fraction >= 1.0 {
                (false, "no ground within search distance")
            } else if !normal_ok {
                (false, "ground normal is too steep")
            } else {
                (true, "ok")
            };
            points.push(SpawnPoint {
                classname: classname.to_owned(),
                position,
                angles,
                on_ground,
                valid,
                reason,
            });
        }
        points.sort_by(|a, b| {
            a.classname
                .cmp(&b.classname)
                .then_with(|| a.position[0].total_cmp(&b.position[0]))
                .then_with(|| a.position[1].total_cmp(&b.position[1]))
                .then_with(|| a.position[2].total_cmp(&b.position[2]))
        });
        Ok(points)
    }
    /// First valid spawn in deterministic order.
    pub fn first_spawn(&self) -> Result<Option<SpawnPoint>> {
        Ok(self.spawn_points()?.into_iter().find(|point| point.valid))
    }
}
