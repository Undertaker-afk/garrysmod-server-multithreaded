//! Offline geometry tool, deliberately not a game-server map activation command.
#![forbid(unsafe_code)]
use scprust_server::{
    bsp::{Map, Vec3},
    collision::Sweep,
};
use std::{fmt::Write as _, process::ExitCode};

const HELP: &str = "Offline BSP v20 brush/spawn inspector (NOT a playable map loader)
scprust-map inspect FILE
scprust-map models FILE
scprust-map spawns FILE
scprust-map ray FILE MASK sx sy sz ex ey ez [--world | --model N]
scprust-map hull FILE MASK sx sy sz ex ey ez minx miny minz maxx maxy maxz [--world | --model N]
MASK is a decimal or 0x-prefixed contents bitmask; 1 selects CONTENTS_SOLID.
World and static brush-entity models only; no displacements, props, moving doors, or Source trace parity.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceTarget {
    Solids,
    World,
    Model(usize),
}

#[derive(Debug)]
enum Action {
    Help,
    Inspect(String),
    Models(String),
    Spawns(String),
    Trace(String, Sweep, TraceTarget),
}

fn vector(args: &[String]) -> Result<Vec3, String> {
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = args[i].parse().map_err(|_| "invalid coordinate")?;
    }
    Ok(out)
}

fn parse_target(tail: &[String]) -> Result<TraceTarget, String> {
    match tail {
        [] => Ok(TraceTarget::Solids),
        [flag] if flag == "--world" => Ok(TraceTarget::World),
        [flag, index] if flag == "--model" => {
            let id = index.parse().map_err(|_| "invalid model index")?;
            Ok(TraceTarget::Model(id))
        }
        _ => Err("unknown command or wrong argument count".into()),
    }
}

fn parse(args: Vec<String>) -> Result<Action, String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Action::Help);
    }
    if args.len() == 2 && args[0] == "inspect" {
        return Ok(Action::Inspect(args[1].clone()));
    }
    if args.len() == 2 && args[0] == "models" {
        return Ok(Action::Models(args[1].clone()));
    }
    if args.len() == 2 && args[0] == "spawns" {
        return Ok(Action::Spawns(args[1].clone()));
    }
    if args.len() < 3 {
        return Err("missing command/arguments".into());
    }
    let hull = args[0] == "hull";
    let base_len = if hull { 15 } else { 9 };
    if (!hull && args[0] != "ray") || args.len() < base_len {
        return Err("unknown command or wrong argument count".into());
    }
    let mask = if let Some(hex) = args[2].strip_prefix("0x") {
        u32::from_str_radix(hex, 16)
    } else {
        args[2].parse()
    }
    .map_err(|_| "invalid contents mask")?;
    let mut sweep = Sweep::ray(vector(&args[3..6])?, vector(&args[6..9])?, mask);
    if hull {
        sweep.mins = vector(&args[9..12])?;
        sweep.maxs = vector(&args[12..15])?;
    }
    sweep.validate().map_err(|e| e.to_string())?;
    let target = parse_target(&args[base_len..])?;
    Ok(Action::Trace(args[1].clone(), sweep, target))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn run(action: Action) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        Action::Help => println!("{HELP}"),
        Action::Inspect(path) => {
            let map = Map::from_file(path)?;
            let spawns = map.spawn_points()?;
            let valid_spawns = spawns.iter().filter(|s| s.valid).count();
            println!(
                "{{\"format\":\"VBSP\",\"version\":20,\"revision\":{},\"entities\":{},\"models\":{},\"solid_models\":{},\"planes\":{},\"nodes\":{},\"leaves\":{},\"world_brushes\":{},\"has_displacements\":{},\"spawn_points\":{},\"valid_spawns\":{},\"collision_scope\":\"world_and_static_brush_models\"}}",
                map.header.revision,
                map.entities.len(),
                map.model_count,
                map.solid_models().len(),
                map.plane_count(),
                map.node_count(),
                map.leaf_count(),
                map.world_brush_count(),
                map.has_displacements,
                spawns.len(),
                valid_spawns
            );
        }
        Action::Models(path) => {
            let map = Map::from_file(path)?;
            let mut items = Vec::new();
            for model in map.solid_models() {
                items.push(format!(
                    "{{\"index\":{},\"name\":{},\"origin\":{:?},\"angles\":{:?},\"brushes\":{}}}",
                    model.index,
                    json_string(&model.name),
                    model.origin,
                    model.angles,
                    model.brush_count
                ));
            }
            println!(
                "{{\"models\":[{}],\"collision_scope\":\"world_and_static_brush_models\"}}",
                items.join(",")
            );
        }
        Action::Spawns(path) => {
            let map = Map::from_file(path)?;
            let spawns = map.spawn_points()?;
            let mut items = Vec::new();
            for spawn in spawns {
                items.push(format!(
                    "{{\"classname\":{},\"position\":{:?},\"angles\":{:?},\"on_ground\":{},\"valid\":{},\"reason\":{}}}",
                    json_string(&spawn.classname),
                    spawn.position,
                    spawn.angles,
                    spawn.on_ground,
                    spawn.valid,
                    json_string(spawn.reason)
                ));
            }
            println!(
                "{{\"spawns\":[{}],\"collision_scope\":\"world_and_static_brush_models\"}}",
                items.join(",")
            );
        }
        Action::Trace(path, sweep, target) => {
            let map = Map::from_file(path)?;
            let (hit, scope) = match target {
                TraceTarget::Solids => (map.trace_solids(sweep)?, "world_and_static_brush_models"),
                TraceTarget::World => (map.trace(sweep)?, "world_brushes_only"),
                TraceTarget::Model(id) => (map.trace_model(id, sweep)?, "single_brush_model"),
            };
            let normal = hit
                .plane_normal
                .map(|n| format!("{n:?}"))
                .unwrap_or_else(|| "null".into());
            let brush = hit
                .brush
                .map(|n| n.to_string())
                .unwrap_or_else(|| "null".into());
            println!(
                "{{\"fraction\":{},\"end_position\":{:?},\"start_solid\":{},\"all_solid\":{},\"normal\":{},\"brush\":{},\"contents\":{},\"tested_brushes\":{},\"collision_scope\":{},\"source_parity\":false}}",
                hit.fraction,
                hit.end_position,
                hit.start_solid,
                hit.all_solid,
                normal,
                brush,
                hit.contents,
                hit.tested_brushes,
                json_string(scope)
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1).collect()) {
        Err(error) => {
            eprintln!("{error}\n{HELP}");
            ExitCode::from(2)
        }
        Ok(action) => match run(action) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("map error: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Action, String> {
        parse(s.split_whitespace().map(str::to_string).collect())
    }

    #[test]
    fn validates_before_opening_a_map() {
        assert!(args("ray not-opened 1 NaN 0 0 1 0 0").is_err());
        assert!(args("hull not-opened 1 0 0 0 1 1 1 2 2 2 1 1 1").is_err());
        assert!(args("ray f 4294967296 0 0 0 1 1 1").is_err());
        assert!(matches!(
            args("ray f 0x1 0 0 0 1 1 1"),
            Ok(Action::Trace(_, _, TraceTarget::Solids))
        ));
        assert!(matches!(
            args("ray f 0x1 0 0 0 1 1 1 --world"),
            Ok(Action::Trace(_, _, TraceTarget::World))
        ));
        assert!(matches!(
            args("hull f 1 0 0 0 1 1 1 -1 -1 0 1 1 2 --model 1"),
            Ok(Action::Trace(_, _, TraceTarget::Model(1)))
        ));
        assert!(args("ray f 1 0 0 0 1 1 1 --model nope").is_err());
        assert!(args("ray f 1 0 0 0 1 1 1 --bogus").is_err());
        assert!(args("inspect f").is_ok());
        assert!(args("models f").is_ok());
        assert!(args("spawns f").is_ok());
        assert!(args("--help").is_ok());
        assert!(args("inspect f extra").is_err());
    }
}
