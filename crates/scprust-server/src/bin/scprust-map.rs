//! Offline geometry tool, deliberately not a game-server map activation command.
#![forbid(unsafe_code)]
use scprust_server::{
    bsp::{Map, Vec3},
    collision::Sweep,
};
use std::process::ExitCode;
const HELP: &str = "Offline BSP v20 world-brush inspector (NOT a playable map loader)
scprust-map inspect FILE
scprust-map ray FILE MASK sx sy sz ex ey ez
scprust-map hull FILE MASK sx sy sz ex ey ez minx miny minz maxx maxy maxz
MASK is a decimal or 0x-prefixed contents bitmask; 1 selects CONTENTS_SOLID.
Only static world brushes; no displacements, props, physics, or Source trace parity.";
#[derive(Debug)]
enum Action {
    Help,
    Inspect(String),
    Trace(String, Sweep),
}
fn vector(args: &[String]) -> Result<Vec3, String> {
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = args[i].parse().map_err(|_| "invalid coordinate")?;
    }
    Ok(out)
}
fn parse(args: Vec<String>) -> Result<Action, String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Action::Help);
    }
    if args.len() == 2 && args[0] == "inspect" {
        return Ok(Action::Inspect(args[1].clone()));
    }
    if args.len() < 3 {
        return Err("missing command/arguments".into());
    }
    let hull = args[0] == "hull";
    if (!hull && args[0] != "ray") || args.len() != if hull { 15 } else { 9 } {
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
    Ok(Action::Trace(args[1].clone(), sweep))
}
fn run(action: Action) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        Action::Help => println!("{HELP}"),
        Action::Inspect(path) => {
            let map = Map::from_file(path)?;
            println!("{{\"format\":\"VBSP\",\"version\":20,\"revision\":{},\"entities\":{},\"models\":{},\"planes\":{},\"nodes\":{},\"leaves\":{},\"world_brushes\":{},\"has_displacements\":{},\"collision_scope\":\"world_brushes_only\"}}",map.header.revision,map.entities.len(),map.model_count,map.plane_count(),map.node_count(),map.leaf_count(),map.world_brush_count(),map.has_displacements);
        }
        Action::Trace(path, sweep) => {
            let map = Map::from_file(path)?;
            let hit = map.trace(sweep)?;
            let normal = hit
                .plane_normal
                .map(|n| format!("{n:?}"))
                .unwrap_or_else(|| "null".into());
            let brush = hit
                .brush
                .map(|n| n.to_string())
                .unwrap_or_else(|| "null".into());
            println!("{{\"fraction\":{},\"end_position\":{:?},\"start_solid\":{},\"all_solid\":{},\"normal\":{},\"brush\":{},\"contents\":{},\"tested_brushes\":{},\"collision_scope\":\"world_brushes_only\",\"source_parity\":false}}",hit.fraction,hit.end_position,hit.start_solid,hit.all_solid,normal,brush,hit.contents,hit.tested_brushes);
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
        assert!(args("ray f 0x1 0 0 0 1 1 1").is_ok());
        assert!(args("inspect f").is_ok());
        assert!(args("--help").is_ok());
        assert!(args("inspect f extra").is_err());
    }
}
