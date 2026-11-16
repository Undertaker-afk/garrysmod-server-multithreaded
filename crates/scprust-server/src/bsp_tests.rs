use crate::{bsp::*, collision::Sweep};
fn int(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn float(out: &mut Vec<u8>, n: f32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn vec3(out: &mut Vec<u8>, v: [f32; 3]) {
    for n in v {
        float(out, n);
    }
}
fn set_i(bytes: &mut [u8], offset: usize, n: i32) {
    bytes[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
}
fn lump(bytes: &mut Vec<u8>, id: usize, version: i32, data: &[u8]) {
    let offset = bytes.len() as i32;
    set_i(bytes, 8 + id * 16, offset);
    set_i(bytes, 12 + id * 16, data.len() as i32);
    set_i(bytes, 16 + id * 16, version);
    bytes.extend_from_slice(data);
}
fn fixture(second_min: f32, shared_world: bool) -> Vec<u8> {
    let mut bytes = vec![0; HEADER_BYTES];
    bytes[..4].copy_from_slice(b"VBSP");
    set_i(&mut bytes, 4, 20);
    set_i(&mut bytes, 1032, 42);
    lump(
        &mut bytes,
        0,
        0,
        b"{\"classname\"\"worldspawn\"}\n{\"classname\"\"info_player_start\"}\0",
    );
    let mut planes = Vec::new();
    for (lo, hi) in [(0.0, 10.0), (second_min, second_min + 10.0)] {
        for (normal, distance) in [
            ([1.0, 0.0, 0.0], hi),
            ([-1.0, 0.0, 0.0], -lo),
            ([0.0, 1.0, 0.0], 5.0),
            ([0.0, -1.0, 0.0], 5.0),
            ([0.0, 0.0, 1.0], 5.0),
            ([0.0, 0.0, -1.0], 5.0),
        ] {
            vec3(&mut planes, normal);
            float(&mut planes, distance);
            int(&mut planes, 0);
        }
    }
    vec3(&mut planes, [1.0, 0.0, 0.0]);
    float(&mut planes, 0.0);
    int(&mut planes, 0);
    lump(&mut bytes, 1, 0, &planes);
    let mut nodes = vec![0; 32];
    set_i(&mut nodes, 0, 12);
    set_i(&mut nodes, 4, -1);
    set_i(&mut nodes, 8, -2);
    lump(&mut bytes, 5, 0, &nodes);
    let mut sides = Vec::new();
    for i in 0..12u16 {
        sides.extend_from_slice(&i.to_le_bytes());
        sides.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0, 0]);
    }
    lump(&mut bytes, 19, 0, &sides);
    let mut brushes = Vec::new();
    for first in [0, 6] {
        int(&mut brushes, first);
        int(&mut brushes, 6);
        int(&mut brushes, 1);
    }
    lump(&mut bytes, 18, 0, &brushes);
    lump(&mut bytes, 17, 0, &[0, 0, 1, 0]);
    let mut leaves = vec![0; 96];
    leaves[26..28].copy_from_slice(&(if shared_world { 2u16 } else { 1 }).to_le_bytes());
    leaves[88..90].copy_from_slice(&1u16.to_le_bytes());
    leaves[90..92].copy_from_slice(&1u16.to_le_bytes());
    lump(&mut bytes, 10, 1, &leaves);
    let mut models = Vec::new();
    for head in [0, -3] {
        vec3(&mut models, [-1000.0; 3]);
        vec3(&mut models, [1000.0; 3]);
        vec3(&mut models, [0.0; 3]);
        int(&mut models, head);
        int(&mut models, 0);
        int(&mut models, 0);
    }
    lump(&mut bytes, 14, 0, &models);
    bytes
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
#[test]
fn reads_world_models_entities_and_ignores_submodel_brushes() {
    let map = Map::parse(&fixture(100.0, false)).unwrap();
    assert_eq!(map.header.revision, 42);
    assert_eq!(map.entities.len(), 2);
    assert_eq!(map.world_brush_count(), 1);
    assert_eq!(map.model_count, 2);
    assert_eq!(map.plane_count(), 13);
    assert_eq!(map.node_count(), 1);
    assert_eq!(map.leaf_count(), 3);
    let hit = map
        .trace(Sweep::ray([90.0, 0.0, 0.0], [120.0, 0.0, 0.0], 1))
        .unwrap();
    assert_eq!(hit.fraction, 1.0);
    assert_eq!(hit.brush, None);
}
#[test]
fn exact_point_hull_misses_masks_and_hit_planes() {
    let map = Map::parse(&fixture(100.0, false)).unwrap();
    let ray = Sweep::ray([-10.0, 0.0, 0.0], [20.0, 0.0, 0.0], 1);
    let trace = map.trace(ray).unwrap();
    close(trace.fraction, 1.0 / 3.0);
    assert_eq!(trace.end_position, [0.0; 3]);
    assert_eq!(trace.plane_normal, Some([-1.0, 0.0, 0.0]));
    assert!(!trace.start_solid);
    let trace = map
        .trace(Sweep {
            mins: [-1.0; 3],
            maxs: [1.0; 3],
            ..ray
        })
        .unwrap();
    close(trace.fraction, 0.3);
    close(trace.end_position[0], -1.0);
    let reverse = map.trace(Sweep::ray(ray.end, ray.start, 1)).unwrap();
    close(reverse.fraction, 1.0 / 3.0);
    assert_eq!(reverse.plane_normal, Some([1.0, 0.0, 0.0]));
    assert_eq!(
        map.trace(Sweep::ray([-10.0, 10.0, 0.0], [20.0, 10.0, 0.0], 1))
            .unwrap()
            .brush,
        None
    );
    assert_eq!(
        map.trace(Sweep {
            contents_mask: 2,
            ..ray
        })
        .unwrap()
        .tested_brushes,
        0
    );
    assert!(map
        .trace(Sweep {
            start: [f64::NAN; 3],
            ..ray
        })
        .is_err());
    assert!(map
        .trace(Sweep {
            mins: [2.0; 3],
            maxs: [1.0; 3],
            ..ray
        })
        .is_err());
}
#[test]
fn start_solid_union_coverage_and_empty_space() {
    let map = Map::parse(&fixture(8.0, true)).unwrap();
    let trace = map
        .trace(Sweep::ray([1.0, 0.0, 0.0], [17.0, 0.0, 0.0], 1))
        .unwrap();
    assert!(trace.start_solid && trace.all_solid);
    assert_eq!(trace.fraction, 0.0);
    let gap = Map::parse(&fixture(12.0, true)).unwrap();
    assert!(
        !gap.trace(Sweep::ray([1.0, 0.0, 0.0], [17.0, 0.0, 0.0], 1))
            .unwrap()
            .all_solid
    );
    let trace = map
        .trace(Sweep::ray([1.0, 0.0, 0.0], [30.0, 0.0, 0.0], 1))
        .unwrap();
    assert!(trace.start_solid && !trace.all_solid);
    assert!(
        map.trace(Sweep::ray([5.0, 0.0, 0.0], [5.0, 0.0, 0.0], 1))
            .unwrap()
            .all_solid
    );
    assert_eq!(
        map.trace(Sweep::ray([-5.0, 0.0, 0.0], [-5.0, 0.0, 0.0], 1))
            .unwrap()
            .brush,
        None
    );
}
#[test]
fn rejects_versions_ranges_compression_cycles_and_nonfinite_planes() {
    let source = fixture(100.0, false);
    for version in [19, 21, i32::MAX] {
        let mut b = source.clone();
        set_i(&mut b, 4, version);
        assert!(Map::parse(&b).is_err());
    }
    let mut b = source.clone();
    set_i(&mut b, 8, -1);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    set_i(&mut b, 12, i32::MAX);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    set_i(&mut b, 8, 1);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    let first = i32::from_le_bytes(b[8..12].try_into().unwrap());
    set_i(&mut b, 24, first);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    set_i(&mut b, 36, 123);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    set_i(&mut b, 16 + 10 * 16, 0);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    let at = Header::parse(&b).unwrap().lumps[5].offset;
    set_i(&mut b, at + 4, 0);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    set_i(&mut b, at + 4, i32::MIN);
    assert!(Map::parse(&b).is_err());
    let mut b = source.clone();
    let at = Header::parse(&b).unwrap().lumps[1].offset;
    set_i(&mut b, at, 0x7fc00000);
    assert!(Map::parse(&b).is_err());
    for n in 0..HEADER_BYTES {
        assert!(Map::parse(&source[..n]).is_err());
    }
}
#[test]
fn mutated_maps_do_not_panic() {
    let source = fixture(100.0, false);
    let mut random = 0x12345678u32;
    for _ in 0..4000 {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        let mut bytes = source.clone();
        let at = random as usize % bytes.len();
        bytes[at] ^= (random >> 8) as u8;
        if let Ok(map) = Map::parse(&bytes) {
            let _ = map.trace(Sweep::ray([-20.0; 3], [20.0; 3], 1));
        }
    }
}
#[test]
fn broadphase_agrees_with_reference_single_axis_box() {
    let map = Map::parse(&fixture(100.0, false)).unwrap();
    for i in 0..200 {
        for j in 0..20 {
            let start = [i as f64 * 0.2 - 15.0, j as f64 - 10.0, 0.0];
            let end = [30.0, start[1], 0.0];
            let trace = map.trace(Sweep::ray(start, end, 1)).unwrap();
            let expected = if start[1].abs() <= 5.0 && start[0] <= 10.0 {
                Some(if start[0] <= 0.0 {
                    -start[0] / (30.0 - start[0])
                } else {
                    0.0
                })
            } else {
                None
            };
            if let Some(fraction) = expected {
                close(trace.fraction, fraction);
                assert_eq!(trace.brush, Some(0));
            } else {
                assert_eq!(trace.brush, None);
            }
        }
    }
}
#[test]
fn parallel_trace_batches_preserve_order_and_match_serial() {
    let map = Map::parse(&fixture(100.0, false)).unwrap();
    let sweeps: Vec<_> = (0..100)
        .map(|i| {
            Sweep::ray(
                [-10.0, i as f64 - 50.0, 0.0],
                [20.0, i as f64 - 50.0, 0.0],
                1,
            )
        })
        .collect();
    let serial: Vec<_> = sweeps.iter().map(|s| map.trace(*s).unwrap()).collect();
    assert_eq!(map.trace_batch(&sweeps, 4).unwrap(), serial);
    assert_eq!(map.trace_batch(&sweeps, 1).unwrap(), serial);
    assert!(map.trace_batch(&sweeps, 0).is_err());
    assert!(map.trace_batch(&vec![sweeps[0]; 1025], 4).is_err());
    assert!(map
        .trace_batch(&[Sweep::ray([f64::NAN; 3], [0.0; 3], 1)], 4)
        .is_err());
    assert!(map.trace_batch(&[], 4).unwrap().is_empty());
}
#[test]
fn non_axial_plane_clipping_and_asymmetric_hull_offsets() {
    let mut bytes = fixture(100.0, false);
    let at = Header::parse(&bytes).unwrap().lumps[1].offset;
    // Replace x<=10 with x+y<=10. The BSP split is still x=0.
    let n = std::f32::consts::FRAC_1_SQRT_2;
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&n.to_le_bytes());
    bytes[at + 12..at + 16].copy_from_slice(&(n * 10.0).to_le_bytes());
    let map = Map::parse(&bytes).unwrap();
    let ray = Sweep::ray([20.0, 2.0, 0.0], [-10.0, 2.0, 0.0], 1);
    let point = map.trace(ray).unwrap();
    assert!((point.end_position[0] - 8.0).abs() < 1e-6);
    let hull = map
        .trace(Sweep {
            mins: [-2.0, -1.0, 0.0],
            maxs: [1.0, 3.0, 0.0],
            ..ray
        })
        .unwrap();
    assert!((hull.end_position[0] - 11.0).abs() < 1e-6);
}
