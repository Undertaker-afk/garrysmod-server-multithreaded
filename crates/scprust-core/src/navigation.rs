use crate::{valid_position, CoreError, CoreResult};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

pub const MAX_NODES: usize = 4096;
pub const MAX_EDGES: usize = 32768;
pub const MAX_PATH: usize = 256;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Edge {
    pub to: u32,
    pub cost: f64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Node {
    pub id: u32,
    pub position: [f64; 3],
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct RouteQuery {
    pub start: u32,
    pub goal: u32,
    pub max_visited: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RouteStatus {
    Found,
    Unreachable,
    BudgetExceeded,
    PathTooLong,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Route {
    pub status: RouteStatus,
    pub nodes: Vec<u32>,
    pub cost: Option<f64>,
    pub visited: usize,
}

#[derive(Debug, Default)]
pub struct Navigation {
    nodes: Vec<Node>,
    index: HashMap<u32, usize>,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    cost: f64,
    node: usize,
}
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.node == other.node
    }
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Navigation {
    pub fn build(mut nodes: Vec<Node>) -> CoreResult<Self> {
        if nodes.len() > MAX_NODES {
            return Err(CoreError::Invalid("too many navigation nodes"));
        }
        nodes.sort_unstable_by_key(|n| n.id);
        let mut index = HashMap::with_capacity(nodes.len());
        let mut edge_count = 0;
        for (i, node) in nodes.iter_mut().enumerate() {
            if node.id == 0 || index.insert(node.id, i).is_some() || !valid_position(&node.position)
            {
                return Err(CoreError::Invalid("invalid or duplicate navigation node"));
            }
            edge_count += node.edges.len();
            if edge_count > MAX_EDGES {
                return Err(CoreError::Invalid("too many navigation edges"));
            }
            node.edges.sort_unstable_by_key(|e| e.to);
            let mut seen = HashSet::new();
            for edge in &node.edges {
                if !edge.cost.is_finite()
                    || edge.cost <= 0.0
                    || edge.cost > 1_000_000_000.0
                    || !seen.insert(edge.to)
                {
                    return Err(CoreError::Invalid("invalid or duplicate navigation edge"));
                }
            }
        }
        if nodes
            .iter()
            .flat_map(|n| &n.edges)
            .any(|e| !index.contains_key(&e.to))
        {
            return Err(CoreError::Invalid(
                "navigation edge references an unknown node",
            ));
        }
        Ok(Self { nodes, index })
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn validate_query(&self, query: &RouteQuery) -> CoreResult<()> {
        if !(1..=MAX_NODES).contains(&query.max_visited)
            || !self.index.contains_key(&query.start)
            || !self.index.contains_key(&query.goal)
        {
            return Err(CoreError::Invalid(
                "unknown route endpoint or invalid visit budget",
            ));
        }
        Ok(())
    }

    /// Dijkstra, not geometric A*: arbitrary positive edge costs need not be
    /// bounded below by Euclidean distance. Validate the query before calling.
    pub fn route(&self, query: &RouteQuery) -> CoreResult<Route> {
        self.validate_query(query)?;
        let start = self.index[&query.start];
        let goal = self.index[&query.goal];
        let mut costs = vec![f64::INFINITY; self.nodes.len()];
        let mut previous = vec![None; self.nodes.len()];
        let mut queue = BinaryHeap::new();
        costs[start] = 0.0;
        queue.push(Entry {
            cost: 0.0,
            node: start,
        });
        let mut visited = 0;
        while let Some(Entry { cost, node }) = queue.pop() {
            if cost > costs[node] {
                continue;
            }
            if visited >= query.max_visited {
                return Ok(Route {
                    status: RouteStatus::BudgetExceeded,
                    nodes: vec![],
                    cost: None,
                    visited,
                });
            }
            visited += 1;
            if node == goal {
                let mut path = vec![self.nodes[goal].id];
                let mut cursor = goal;
                while cursor != start {
                    if path.len() >= MAX_PATH {
                        return Ok(Route {
                            status: RouteStatus::PathTooLong,
                            nodes: vec![],
                            cost: None,
                            visited,
                        });
                    }
                    // A settled reachable node always has a predecessor except start.
                    cursor = previous[cursor].ok_or(CoreError::Unavailable)?;
                    path.push(self.nodes[cursor].id);
                }
                path.reverse();
                return Ok(Route {
                    status: RouteStatus::Found,
                    nodes: path,
                    cost: Some(cost),
                    visited,
                });
            }
            for edge in &self.nodes[node].edges {
                let next = self.index[&edge.to];
                let next_cost = cost + edge.cost;
                if next_cost < costs[next] {
                    costs[next] = next_cost;
                    previous[next] = Some(node);
                    queue.push(Entry {
                        cost: next_cost,
                        node: next,
                    });
                }
            }
        }
        Ok(Route {
            status: RouteStatus::Unreachable,
            nodes: vec![],
            cost: None,
            visited,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: u32, edges: &[(u32, f64)]) -> Node {
        Node {
            id,
            position: [0.0; 3],
            edges: edges.iter().map(|&(to, cost)| Edge { to, cost }).collect(),
        }
    }
    fn query(start: u32, goal: u32, max_visited: usize) -> RouteQuery {
        RouteQuery {
            start,
            goal,
            max_visited,
        }
    }
    #[test]
    fn weighted_directed_shortest_path_and_budget() {
        let nav = Navigation::build(vec![
            node(1, &[(3, 10.0), (2, 1.0)]),
            node(2, &[(3, 2.0)]),
            node(3, &[]),
        ])
        .unwrap();
        let route = nav.route(&query(1, 3, 3)).unwrap();
        assert_eq!(route.nodes, vec![1, 2, 3]);
        assert_eq!(route.cost, Some(3.0));
        assert_eq!(
            nav.route(&query(3, 1, 3)).unwrap().status,
            RouteStatus::Unreachable
        );
        assert_eq!(
            nav.route(&query(1, 3, 1)).unwrap().status,
            RouteStatus::BudgetExceeded
        );
        assert_eq!(nav.route(&query(1, 1, 1)).unwrap().nodes, vec![1]);
    }
    #[test]
    fn rejects_invalid_graphs_and_queries() {
        assert!(Navigation::build(vec![node(1, &[(2, 1.0)])]).is_err());
        assert!(Navigation::build(vec![node(1, &[]), node(1, &[])]).is_err());
        assert!(Navigation::build(vec![node(1, &[(1, f64::NAN)])]).is_err());
        assert!(Navigation::build(vec![node(1, &[(1, 0.0)])]).is_err());
        assert!(Navigation::build(vec![node(1, &[(1, 1.0), (1, 2.0)])]).is_err());
        let nav = Navigation::build(vec![node(1, &[])]).unwrap();
        assert!(nav.route(&query(1, 2, 5)).is_err());
        assert!(nav.route(&query(1, 1, 0)).is_err());
    }
    #[test]
    fn bounds_returned_paths() {
        let nodes = (1..=258)
            .map(|i| {
                if i == 258 {
                    node(i, &[])
                } else {
                    node(i, &[(i + 1, 1.0)])
                }
            })
            .collect();
        let nav = Navigation::build(nodes).unwrap();
        assert_eq!(
            nav.route(&query(1, 258, 300)).unwrap().status,
            RouteStatus::PathTooLong
        );
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn all_pairs_match_floyd_warshall_reference() {
        const N: usize = 16;
        let mut distances = [[f64::INFINITY; N]; N];
        let mut nodes = Vec::new();
        for (i, row) in distances.iter_mut().enumerate() {
            row[i] = 0.0;
            let mut edges = Vec::new();
            for (j, distance) in row.iter_mut().enumerate() {
                if i != j && (i * 17 + j * 31) % 5 < 2 {
                    let cost = ((i * 11 + j * 7) % 13 + 1) as f64;
                    edges.push(Edge {
                        to: j as u32 + 1,
                        cost,
                    });
                    *distance = cost;
                }
            }
            nodes.push(Node {
                id: i as u32 + 1,
                position: [0.0; 3],
                edges,
            });
        }
        for k in 0..N {
            for i in 0..N {
                for j in 0..N {
                    distances[i][j] = distances[i][j].min(distances[i][k] + distances[k][j]);
                }
            }
        }
        let graph = Navigation::build(nodes).unwrap();
        for (i, row) in distances.iter().enumerate() {
            for (j, distance) in row.iter().enumerate() {
                let route = graph
                    .route(&RouteQuery {
                        start: i as u32 + 1,
                        goal: j as u32 + 1,
                        max_visited: N,
                    })
                    .unwrap();
                if distance.is_finite() {
                    assert_eq!(route.cost, Some(*distance));
                } else {
                    assert_eq!(route.status, RouteStatus::Unreachable);
                }
            }
        }
    }
}
