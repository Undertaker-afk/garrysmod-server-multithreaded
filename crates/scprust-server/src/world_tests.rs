//! Fixture-driven tests for brush-model collision, spawn selection and movement.
//! Fixtures are generated synthetic geometry, not real Source map data.
use crate::{
    bsp::{Map, Vec3, HEADER_BYTES},
    collision::Sweep,
    movement::{step, Hull, MoveConfig, MoveInput, PlayerState},
    spawn::SPAWN_CLASSES,
};

#[derive(Clone, Copy)]
struct Box3 {
    mins: [f64; 3],
    maxs: [f64; 3],
}
fn bx(mins: [f64; 3], maxs: [f64; 3]) -> Box3 {
    Box3 { mins, maxs }
}

#[derive(Default)]
struct Builder {
    planes: Vec<(Vec3, f64)>,
    sides: Vec<u16>,
    brushes: Vec<(i32, i32)>,
    leaf_brushes: Vec<u16>,
    leaves: Vec<Vec<u8>>,
    models: Vec<(Vec3, Vec3, i32)>,
}
impl Builder {
    fn brush(&mut self, b: Box3) -> u16 {
        let index = self.brushes.len() as u16;
        let first = self.sides.len() as u16;
        let faces = [
            ([1.0, 0.0, 0.0], b.maxs[0]),
            ([-1.0, 0.0, 0.0], -b.mins[0]),
            ([0.0, 1.0, 0.0], b.maxs[1]),
            ([0.0, -1.0, 0.0], -b.mins[1]),
            ([0.0, 0.0, 1.0], b.maxs[2]),
            ([0.0, 0.0, -1.0], -b.mins[2]),
        ];
        for (normal, distance) in faces {
            self.sides.push(self.planes.len() as u16);
            self.planes.push((normal, distance));
        }
        self.brushes.push((first as i32, 6));
        index
    }
    fn leaf(&mut self, brushes: &[u16]) -> i32 {
        let first = self.leaf_brushes.len() as u16;
        self.leaf_brushes.extend_from_slice(brushes);
        let mut leaf = vec![0u8; 32];
        leaf[0..4].copy_from_slice(&1i32.to_le_bytes());
        leaf[4..6].copy_from_slice(&(-1i16).to_le_bytes());
        leaf[24..26].copy_from_slice(&first.to_le_bytes());
        leaf[26..28].copy_from_slice(&(brushes.len() as u16).to_le_bytes());
        self.leaves.push(leaf);
        -(self.leaves.len() as i32)
    }
    fn model(&mut self, mins: Vec3, maxs: Vec3, headnode: i32) {
        self.models.push((mins, maxs, headnode));
    }
}

fn assemble(entities: &str, builder: Builder) -> Map {
    let mut bytes = vec![0u8; HEADER_BYTES];
    bytes[..4].copy_from_slice(b"VBSP");
    bytes[4..8].copy_from_slice(&20i32.to_le_bytes());
    bytes[1032..1036].copy_from_slice(&7i32.to_le_bytes());
    fn lump(bytes: &mut Vec<u8>, id: usize, version: i32, data: &[u8]) {
        let offset = bytes.len() as i32;
        bytes[8 + id * 16..12 + id * 16].copy_from_slice(&offset.to_le_bytes());
        bytes[12 + id * 16..16 + id * 16].copy_from_slice(&(data.len() as i32).to_le_bytes());
        bytes[16 + id * 16..20 + id * 16].copy_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(data);
    }
    let mut entity_lump = entities.as_bytes().to_vec();
    entity_lump.push(0);
    lump(&mut bytes, 0, 0, &entity_lump);
    let mut planes = Vec::new();
    for (normal, distance) in &builder.planes {
        for component in normal {
            planes.extend_from_slice(&(*component as f32).to_le_bytes());
        }
        planes.extend_from_slice(&(*distance as f32).to_le_bytes());
        planes.extend_from_slice(&0i32.to_le_bytes());
    }
    lump(&mut bytes, 1, 0, &planes);
    let mut sides = Vec::new();
    for plane in &builder.sides {
        sides.extend_from_slice(&plane.to_le_bytes());
        sides.extend_from_slice(&(-1i16).to_le_bytes());
        sides.extend_from_slice(&(-1i16).to_le_bytes());
        sides.extend_from_slice(&0i16.to_le_bytes());
    }
    lump(&mut bytes, 19, 0, &sides);
    let mut brushes = Vec::new();
    for (first, count) in &builder.brushes {
        brushes.extend_from_slice(&first.to_le_bytes());
        brushes.extend_from_slice(&count.to_le_bytes());
        brushes.extend_from_slice(&1i32.to_le_bytes());
    }
    lump(&mut bytes, 18, 0, &brushes);
    let mut leaf_brushes = Vec::new();
    for brush in &builder.leaf_brushes {
        leaf_brushes.extend_from_slice(&brush.to_le_bytes());
    }
    lump(&mut bytes, 17, 0, &leaf_brushes);
    let mut leaves = Vec::new();
    for leaf in &builder.leaves {
        leaves.extend_from_slice(leaf);
    }
    lump(&mut bytes, 10, 1, &leaves);
    let mut models = Vec::new();
    for (mins, maxs, headnode) in &builder.models {
        for value in mins.iter().chain(maxs).chain([0.0f64; 3].iter()) {
            models.extend_from_slice(&(*value as f32).to_le_bytes());
        }
        models.extend_from_slice(&headnode.to_le_bytes());
        models.extend_from_slice(&0i32.to_le_bytes());
        models.extend_from_slice(&0i32.to_le_bytes());
    }
    lump(&mut bytes, 14, 0, &models);
    Map::parse(&bytes).expect("fixture parses")
}

/// World brushes plus any number of brush entities (model index 1..).
fn world(entities: &str, world_boxes: &[Box3], solids: &[(Vec3, Vec<Box3>)]) -> Map {
    let mut builder = Builder::default();
    let mut leaf = Vec::new();
    for box3 in world_boxes {
        leaf.push(builder.brush(*box3));
    }
    let world_leaf = builder.leaf(&leaf);
    builder.model([-4096.0; 3], [4096.0; 3], world_leaf);
    for (origin, boxes) in solids {
        let mut leaves = Vec::new();
        for box3 in boxes {
            leaves.push(builder.brush(*box3));
        }
        let leaf = builder.leaf(&leaves);
        builder.model(
            *origin,
            [origin[0] + 64.0, origin[1] + 64.0, origin[2] + 64.0],
            leaf,
        );
    }
    assemble(entities, builder)
}

fn floor() -> Box3 {
    bx([-500.0, -500.0, -16.0], [500.0, 500.0, 0.0])
}

#[test]
fn brush_entity_transform_is_applied() {
    let map = world(
        r#"{"classname""worldspawn"}{"classname""func_brush""model""*1""origin""100 0 0"}"#,
        &[],
        &[(
            [100.0, 0.0, 0.0],
            vec![bx([-5.0, -5.0, -5.0], [5.0, 5.0, 5.0])],
        )],
    );
    // model index 1 is translated to x=100; a ray from x=90 must hit at x=95.
    let hit = map
        .trace_model(1, Sweep::ray([90.0, 0.0, 0.0], [110.0, 0.0, 0.0], 1))
        .unwrap();
    assert!((hit.fraction - 0.25).abs() < 1e-9, "{hit:?}");
    assert_eq!(hit.plane_normal, Some([-1.0, 0.0, 0.0]));
    // The same sweep against the world model misses entirely.
    assert_eq!(
        map.trace(Sweep::ray([90.0, 0.0, 0.0], [110.0, 0.0, 0.0], 1))
            .unwrap()
            .fraction,
        1.0
    );
    assert!(map
        .trace_model(2, Sweep::ray([0.0; 3], [1.0; 3], 1))
        .is_err());
}

#[test]
fn rotated_brush_entity_uses_yaw() {
    let map = world(
        r#"{"classname""worldspawn"}{"classname""func_brush""model""*1""angles""0 90 0"}"#,
        &[],
        &[(
            [0.0, 0.0, 0.0],
            vec![bx([-10.0, -5.0, -5.0], [10.0, 5.0, 5.0])],
        )],
    );
    // Yaw 90 maps model +x to world +y, so the y-facing extent becomes 10.
    let hit = map
        .trace_model(1, Sweep::ray([0.0, -20.0, 0.0], [0.0, 20.0, 0.0], 1))
        .unwrap();
    assert!((hit.fraction - 0.25).abs() < 1e-9, "{hit:?}");
    let normal = hit.plane_normal.unwrap();
    assert!(normal[0].abs() < 1e-9 && normal[1] < -0.999, "{hit:?}");
}

#[test]
fn asymmetric_brush_entity_keeps_handedness() {
    let plain = world(
        r#"{"classname""worldspawn"}{"classname""func_brush""model""*1"}"#,
        &[],
        &[(
            [0.0, 0.0, 0.0],
            vec![bx([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])],
        )],
    );
    // Unrotated: the model y range is [0, 10], not mirrored to [-10, 0].
    let hit = plain
        .trace_model(1, Sweep::ray([5.0, -30.0, 5.0], [5.0, 30.0, 5.0], 1))
        .unwrap();
    assert!((hit.fraction - 0.5).abs() < 1e-9, "{hit:?}");

    let yawed = world(
        r#"{"classname""worldspawn"}{"classname""func_brush""model""*1""angles""0 90 0"}"#,
        &[],
        &[(
            [0.0, 0.0, 0.0],
            vec![bx([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])],
        )],
    );
    // Yaw 90 maps model +x to world +y and model +y to world -x, so world x
    // is [-10, 0] and world y is [0, 10].
    let miss = yawed
        .trace_model(1, Sweep::ray([5.0, -30.0, 5.0], [5.0, 30.0, 5.0], 1))
        .unwrap();
    assert_eq!(miss.fraction, 1.0, "{miss:?}");
    let hit = yawed
        .trace_model(1, Sweep::ray([-5.0, -30.0, 5.0], [-5.0, 30.0, 5.0], 1))
        .unwrap();
    assert!((hit.fraction - 0.5).abs() < 1e-9, "{hit:?}");
    let hit = yawed
        .trace_model(1, Sweep::ray([-30.0, 5.0, 5.0], [30.0, 5.0, 5.0], 1))
        .unwrap();
    // World x range from rotated model y is [-10, 0].
    assert!((hit.fraction - 1.0 / 3.0).abs() < 1e-9, "{hit:?}");
}

#[test]
fn static_brush_entity_participates_in_solid_traces() {
    let map = world(
        r#"{"classname""worldspawn"}{"classname""func_brush""model""*1"}"#,
        &[floor()],
        &[(
            [0.0, 0.0, 0.0],
            vec![bx([60.0, -20.0, 0.0], [80.0, 20.0, 40.0])],
        )],
    );
    let hit = map
        .trace_solids(Sweep::ray([0.0, 0.0, 10.0], [100.0, 0.0, 10.0], 1))
        .unwrap();
    assert!((hit.fraction - 0.6).abs() < 1e-9, "{hit:?}");
    assert_eq!(map.solid_models().len(), 1);
    assert_eq!(map.solid_models()[0].index, 1);
    assert_eq!(map.solid_models()[0].name, "*1");
}

#[test]
fn spawn_points_are_sorted_and_validated() {
    let entities = format!(
        r#"{{"classname""worldspawn"}}{{"classname""{}""origin""10 0 0"}}{{"classname""{}""origin""0 0 300"}}"#,
        SPAWN_CLASSES[0], SPAWN_CLASSES[1]
    );
    let map = world(&entities, &[floor()], &[]);
    let points = map.spawn_points().unwrap();
    assert_eq!(points.len(), 2);
    // Sorted by classname: counterterrorist... before start? Use explicit checks.
    for point in &points {
        if point.classname == SPAWN_CLASSES[0] {
            assert!(point.valid, "{point:?}");
            assert!(point.on_ground);
            assert_eq!(point.position, [10.0, 0.0, 0.0]);
        } else {
            assert!(!point.valid);
            assert!(!point.on_ground);
            assert_eq!(point.reason, "no ground within search distance");
        }
    }
    let first = map.first_spawn().unwrap().unwrap();
    assert_eq!(first.classname, SPAWN_CLASSES[0]);
    assert_eq!(first.position[0], 10.0);
}

#[test]
fn embedded_spawn_is_rejected() {
    let entities = format!(
        r#"{{"classname""worldspawn"}}{{"classname""{}""origin""0 0 -8"}}"#,
        SPAWN_CLASSES[0]
    );
    let map = world(&entities, &[floor()], &[]);
    let point = map.first_spawn().unwrap();
    assert!(point.is_none());
    let points = map.spawn_points().unwrap();
    assert!(!points[0].valid);
    assert!(points[0].reason.contains("obstructed"));
}

#[test]
fn missing_origin_and_bad_vectors_are_reported() {
    let entities = format!(
        r#"{{"classname""worldspawn"}}{{"classname""{}"}}"#,
        SPAWN_CLASSES[0]
    );
    let map = world(&entities, &[floor()], &[]);
    let points = map.spawn_points().unwrap();
    assert_eq!(points[0].reason, "missing origin");
    assert!(map.first_spawn().unwrap().is_none());

    let bad = format!(
        r#"{{"classname""worldspawn"}}{{"classname""{}""origin""1 2"}}"#,
        SPAWN_CLASSES[0]
    );
    let map = world(&bad, &[floor()], &[]);
    assert!(map.spawn_points().is_err());
}

fn simulate(
    map: &Map,
    state: &mut PlayerState,
    input: MoveInput,
    frames: usize,
    dt: f64,
) -> Vec<Vec3> {
    let cfg = MoveConfig::default();
    let mut trail = Vec::new();
    for _ in 0..frames {
        step(map, &Hull::PLAYER, &cfg, state, &input, dt).unwrap();
        trail.push(state.position);
    }
    trail
}

#[test]
fn falls_and_lands_on_floor() {
    let map = world(r#"{"classname""worldspawn"}"#, &[floor()], &[]);
    let mut state = PlayerState::new([0.0, 0.0, 120.0]);
    let trail = simulate(&map, &mut state, MoveInput::IDLE, 180, 1.0 / 60.0);
    assert!(trail.iter().any(|p| p[2] > 100.0));
    assert!(state.on_ground, "{state:?}");
    assert!(state.position[2].abs() < 1e-6, "{state:?}");
    assert!(state.velocity[2].abs() < 1e-6, "{state:?}");
}

#[test]
fn walking_is_blocked_by_a_tall_wall() {
    let map = world(
        r#"{"classname""worldspawn"}"#,
        &[floor(), bx([30.0, -100.0, 0.0], [44.0, 100.0, 32.0])],
        &[],
    );
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    let input = MoveInput {
        wish_dir: [1.0, 0.0, 0.0],
        wish_speed: 250.0,
        jump: false,
    };
    simulate(&map, &mut state, input, 120, 0.05);
    // Hull half-width 16: contact with the wall face at x=30 is x=14.
    assert!(state.on_ground, "{state:?}");
    assert!(state.position[0] <= 14.001, "{state:?}");
    assert!(state.position[0] > 13.0, "{state:?}");
    assert!(state.position[2].abs() < 1e-6, "{state:?}");
}

#[test]
fn diagonal_walk_slides_along_wall_and_stays_grounded() {
    let map = world(
        r#"{"classname""worldspawn"}"#,
        &[floor(), bx([30.0, -200.0, 0.0], [44.0, 500.0, 32.0])],
        &[],
    );
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    let inv_sqrt2 = std::f64::consts::FRAC_1_SQRT_2;
    let input = MoveInput {
        wish_dir: [inv_sqrt2, inv_sqrt2, 0.0],
        wish_speed: 250.0,
        jump: false,
    };
    simulate(&map, &mut state, input, 60, 1.0 / 60.0);
    assert!(state.on_ground, "{state:?}");
    assert!(state.position[0] <= 14.001, "{state:?}");
    assert!(state.position[0] > 13.5, "{state:?}");
    assert!(state.position[1] > 100.0, "{state:?}");
    assert!(state.position[2].abs() < 1e-6, "{state:?}");
}

#[test]
fn low_step_is_climbed() {
    let map = world(
        r#"{"classname""worldspawn"}"#,
        &[floor(), bx([30.0, -100.0, 0.0], [2000.0, 100.0, 8.0])],
        &[],
    );
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    let input = MoveInput {
        wish_dir: [1.0, 0.0, 0.0],
        wish_speed: 250.0,
        jump: false,
    };
    let mut stepped = false;
    for _ in 0..40 {
        let report = step(
            &map,
            &Hull::PLAYER,
            &MoveConfig::default(),
            &mut state,
            &input,
            0.05,
        )
        .unwrap();
        stepped |= report.stepped;
    }
    assert!(stepped, "no step was reported");
    assert!((state.position[2] - 8.0).abs() < 1e-6, "{state:?}");
    assert!(state.position[0] > 60.0, "{state:?}");
}

#[test]
fn jump_rises_then_lands() {
    let map = world(r#"{"classname""worldspawn"}"#, &[floor()], &[]);
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    let cfg = MoveConfig::default();
    step(
        &map,
        &Hull::PLAYER,
        &cfg,
        &mut state,
        &MoveInput {
            wish_dir: [0.0; 3],
            wish_speed: 0.0,
            jump: true,
        },
        1.0 / 60.0,
    )
    .unwrap();
    let mut peak = state.position[2];
    for _ in 0..180 {
        step(
            &map,
            &Hull::PLAYER,
            &cfg,
            &mut state,
            &MoveInput::IDLE,
            1.0 / 60.0,
        )
        .unwrap();
        peak = peak.max(state.position[2]);
    }
    assert!(peak > 20.0, "peak {peak}");
    assert!(state.on_ground);
    assert!(state.position[2].abs() < 1e-6, "{state:?}");
}

#[test]
fn movement_is_deterministic() {
    let map = world(
        r#"{"classname""worldspawn"}"#,
        &[floor(), bx([30.0, -100.0, 0.0], [400.0, 100.0, 8.0])],
        &[],
    );
    let input = MoveInput {
        wish_dir: [1.0, 0.0, 0.0],
        wish_speed: 250.0,
        jump: false,
    };
    let mut a = PlayerState::new([0.0, 0.0, 0.0]);
    let mut b = PlayerState::new([0.0, 0.0, 0.0]);
    let trail_a = simulate(&map, &mut a, input, 30, 0.05);
    let trail_b = simulate(&map, &mut b, input, 30, 0.05);
    assert_eq!(trail_a, trail_b);
    assert_eq!(a, b);
    // Substep count is bounded and reported.
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    let report = step(
        &map,
        &Hull::PLAYER,
        &MoveConfig::default(),
        &mut state,
        &input,
        0.25,
    )
    .unwrap();
    assert_eq!(report.substeps, 15);
}

#[test]
fn invalid_movement_inputs_are_rejected() {
    let map = world(r#"{"classname""worldspawn"}"#, &[floor()], &[]);
    let cfg = MoveConfig::default();
    let mut state = PlayerState::new([0.0, 0.0, 0.0]);
    assert!(step(&map, &Hull::PLAYER, &cfg, &mut state, &MoveInput::IDLE, 0.3).is_err());
    assert!(step(
        &map,
        &Hull::PLAYER,
        &cfg,
        &mut state,
        &MoveInput::IDLE,
        f64::NAN
    )
    .is_err());
    let bad_dir = MoveInput {
        wish_dir: [2.0, 0.0, 0.0],
        wish_speed: 250.0,
        jump: false,
    };
    assert!(step(&map, &Hull::PLAYER, &cfg, &mut state, &bad_dir, 0.05).is_err());
    let float_hull = Hull {
        mins: [0.0, 0.0, 1.0],
        maxs: [0.0, 0.0, 72.0],
    };
    assert!(step(&map, &float_hull, &cfg, &mut state, &MoveInput::IDLE, 0.05).is_err());
    let mut embedded = PlayerState::new([0.0, 0.0, -100.0]);
    let report = step(
        &map,
        &Hull::PLAYER,
        &cfg,
        &mut embedded,
        &MoveInput::IDLE,
        0.05,
    )
    .unwrap();
    assert!(!report.landed || embedded.on_ground);
}
