//! Pure snapshot computation: no Lua state, engine pointers, or entity mutation.
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_POINTS: usize = 8192;
pub const MAX_QUERIES: usize = 128;
pub const MAX_HITS: usize = 512;
const MAX_COORD: f64 = 1_000_000.0;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub id: u32,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub center: [f64; 3],
    pub radius: f64,
    pub max_hits: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub points: Vec<Point>,
    pub queries: Vec<Query>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct QueryResult {
    pub ids: Vec<u32>,
    /// True means some matching IDs were omitted. IDs are NOT nearest-first.
    pub truncated: bool,
}

fn valid_position(p: &[f64; 3]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

pub fn validate(batch: &Batch) -> Result<(), &'static str> {
    if batch.points.len() > MAX_POINTS || batch.queries.len() > MAX_QUERIES {
        return Err("too many points or queries");
    }
    let mut ids = HashSet::with_capacity(batch.points.len());
    for point in &batch.points {
        if point.id == 0 || !ids.insert(point.id) || !valid_position(&point.position) {
            return Err(
                "point IDs must be unique and positive; coordinates must be finite and bounded",
            );
        }
    }
    for query in &batch.queries {
        if !valid_position(&query.center)
            || !query.radius.is_finite()
            || !(0.0..=MAX_COORD).contains(&query.radius)
            || !(1..=MAX_HITS).contains(&query.max_hits)
        {
            return Err("invalid query center, radius, or max_hits");
        }
    }
    Ok(())
}

/// Call only on validated input. Parallelism is over queries in the caller's Rayon pool.
pub fn compute(mut batch: Batch) -> Vec<QueryResult> {
    batch
        .points
        .sort_unstable_by(|a, b| a.position[0].total_cmp(&b.position[0]));
    batch
        .queries
        .par_iter()
        .map(|query| {
            let lo = query.center[0] - query.radius;
            let hi = query.center[0] + query.radius;
            let start = batch.points.partition_point(|p| p.position[0] < lo);
            let end = batch.points.partition_point(|p| p.position[0] <= hi);
            let radius_squared = query.radius * query.radius;
            let mut ids: Vec<u32> = batch.points[start..end]
                .iter()
                .filter_map(|point| {
                    let distance_squared: f64 = point
                        .position
                        .iter()
                        .zip(query.center.iter())
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum();
                    (distance_squared <= radius_squared).then_some(point.id)
                })
                .collect();
            ids.sort_unstable();
            let truncated = ids.len() > query.max_hits;
            ids.truncate(query.max_hits);
            QueryResult { ids, truncated }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(id: u32, x: f64) -> Point {
        Point {
            id,
            position: [x, 0.0, 0.0],
        }
    }
    fn query(radius: f64, max_hits: usize) -> Query {
        Query {
            center: [0.0; 3],
            radius,
            max_hits,
        }
    }
    #[test]
    fn includes_boundary_and_preserves_query_order() {
        let batch = Batch {
            points: vec![point(3, 5.1), point(2, -5.0), point(1, 0.0)],
            queries: vec![query(5.0, 10), query(0.0, 10)],
        };
        validate(&batch).unwrap();
        assert_eq!(
            compute(batch),
            vec![
                QueryResult {
                    ids: vec![1, 2],
                    truncated: false
                },
                QueryResult {
                    ids: vec![1],
                    truncated: false
                },
            ]
        );
    }
    #[test]
    fn deterministic_truncation() {
        let batch = Batch {
            points: vec![point(9, 0.0), point(2, 1.0)],
            queries: vec![query(10.0, 1)],
        };
        assert_eq!(
            compute(batch),
            vec![QueryResult {
                ids: vec![2],
                truncated: true
            }]
        );
    }
    #[test]
    fn rejects_invalid_inputs() {
        let mut batch = Batch {
            points: vec![point(1, 0.0), point(1, 1.0)],
            queries: vec![],
        };
        assert!(validate(&batch).is_err());
        batch.points = vec![point(1, f64::NAN)];
        assert!(validate(&batch).is_err());
        batch.points.clear();
        batch.queries = vec![query(-1.0, 1)];
        assert!(validate(&batch).is_err());
        batch.queries = vec![query(1.0, MAX_HITS + 1)];
        assert!(validate(&batch).is_err());
        batch.queries = vec![query(1.0, 1); MAX_QUERIES + 1];
        assert!(validate(&batch).is_err());
        batch.queries.clear();
        batch.points = (1..=MAX_POINTS as u32 + 1)
            .map(|id| point(id, 0.0))
            .collect();
        assert!(validate(&batch).is_err());
    }
    #[test]
    fn matches_brute_force_in_three_dimensions() {
        let points: Vec<Point> = (1..=1000)
            .map(|id| Point {
                id,
                position: [
                    ((id * 17) % 101) as f64 - 50.0,
                    ((id * 23) % 89) as f64 - 44.0,
                    ((id * 31) % 79) as f64 - 39.0,
                ],
            })
            .collect();
        let queries: Vec<Query> = (0..32)
            .map(|i| Query {
                center: [i as f64 - 16.0, 3.0, -2.0],
                radius: i as f64,
                max_hits: 512,
            })
            .collect();
        let expected: Vec<QueryResult> = queries
            .iter()
            .map(|q| {
                let mut ids: Vec<u32> = points
                    .iter()
                    .filter(|p| {
                        p.position
                            .iter()
                            .zip(q.center.iter())
                            .map(|(a, b)| (a - b) * (a - b))
                            .sum::<f64>()
                            <= q.radius * q.radius
                    })
                    .map(|p| p.id)
                    .collect();
                ids.sort_unstable();
                let truncated = ids.len() > q.max_hits;
                ids.truncate(q.max_hits);
                QueryResult { ids, truncated }
            })
            .collect();
        let batch = Batch { points, queries };
        validate(&batch).unwrap();
        assert_eq!(compute(batch), expected);
    }
}
