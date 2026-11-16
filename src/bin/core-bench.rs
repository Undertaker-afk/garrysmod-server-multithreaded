//! Compute-only benchmark. Does not measure GMod, JSON, sockets, or engine work.
use rayon::prelude::*;
use scprust_core::{Agent, Edge, Entity, EntityKey, Node, Publish, RouteQuery, WorldStore};
use std::{hint::black_box, time::Instant};

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    samples[(samples.len() - 1) * percentile / 100]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = WorldStore::new("benchmark".into());
    let entities: Vec<_> = (1..=662)
        .map(|id| Entity {
            key: EntityKey { id, generation: 1 },
            position: [(id % 32) as f64 * 64.0, (id / 32) as f64 * 64.0, 0.0],
            faction: if id <= 150 { 1 } else { 2 },
            alive: true,
        })
        .collect();
    let navigation = (1u32..=512)
        .map(|id| {
            let mut edges = Vec::new();
            for to in [id - 1, id + 1, id.saturating_sub(32), id + 32] {
                if (1..=512).contains(&to) {
                    edges.push(Edge { to, cost: 1.0 });
                }
            }
            Node {
                id,
                position: [(id % 32) as f64, (id / 32) as f64, 0.0],
                edges,
            }
        })
        .collect();
    let started = Instant::now();
    let world = store
        .publish(Publish {
            base: store.latest().unwrap().version.clone(),
            entities,
            navigation: Some(navigation),
        })
        .map_err(|e| format!("publish: {e:?}"))?;
    println!("Synthetic scene: 150 players + 512 actors; 128 thinkers + 64 routes/batch");
    println!(
        "One-time world/index/graph construction: {:.1} us",
        started.elapsed().as_secs_f64() * 1e6
    );
    let agents: Vec<_> = (151..=278)
        .map(|id| Agent {
            actor: EntityKey { id, generation: 1 },
            perception_radius: 2048.0,
            attack_radius: 64.0,
            move_speed: 160.0,
            target_factions: vec![1],
            current_target: None,
        })
        .collect();
    let routes: Vec<_> = (1..=64)
        .map(|start| RouteQuery {
            start,
            goal: 513 - start,
            max_visited: 512,
        })
        .collect();
    let mut reference = None;
    for threads in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        let compute = || {
            pool.install(|| {
                let intents: Vec<_> = agents.par_iter().map(|a| world.think(a).unwrap()).collect();
                let paths: Vec<_> = routes
                    .par_iter()
                    .map(|q| world.navigation.route(q).unwrap())
                    .collect();
                (intents, paths)
            })
        };
        let result = compute();
        if let Some(ref expected) = reference {
            assert_eq!(&result, expected);
        }
        reference = Some(result);
        for _ in 0..10 {
            black_box(compute());
        }
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let started = Instant::now();
            black_box(compute());
            samples.push(started.elapsed().as_secs_f64() * 1e6);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{threads} compute threads: p50={:.1} us p95={:.1} us p99={:.1} us",
            percentile(&samples, 50),
            percentile(&samples, 95),
            percentile(&samples, 99)
        );
    }
    println!("Not a capacity estimate: compare end-to-end GMod tick time on your actual schema.");
    Ok(())
}
