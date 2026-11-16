use crate::{
    distance_squared, valid_position, CoreError, CoreResult, Navigation, Node, MAX_ENTITIES,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct EntityKey {
    pub id: u32,
    pub generation: u32,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Entity {
    pub key: EntityKey,
    pub position: [f64; 3],
    pub faction: u16,
    pub alive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Version {
    pub epoch: String,
    pub revision: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Publish {
    /// Compare-and-swap against this version. No last-writer-wins updates.
    pub base: Version,
    pub entities: Vec<Entity>,
    /// Omitted/null: retain graph. []: explicitly clear graph.
    pub navigation: Option<Vec<Node>>,
}

#[derive(Debug)]
pub struct World {
    pub version: Version,
    pub navigation: Arc<Navigation>,
    entities: Vec<Entity>,
    by_id: HashMap<u32, usize>,
    by_x: Vec<usize>,
}

impl World {
    fn build(
        version: Version,
        mut entities: Vec<Entity>,
        navigation: Arc<Navigation>,
    ) -> CoreResult<Self> {
        if entities.len() > MAX_ENTITIES {
            return Err(CoreError::Invalid("too many world entities"));
        }
        entities.sort_unstable_by_key(|e| e.key);
        let mut by_id = HashMap::with_capacity(entities.len());
        for (i, entity) in entities.iter().enumerate() {
            if entity.key.id == 0
                || by_id.insert(entity.key.id, i).is_some()
                || !valid_position(&entity.position)
            {
                return Err(CoreError::Invalid(
                    "invalid coordinates or duplicate/zero entity slot",
                ));
            }
        }
        let mut by_x: Vec<usize> = (0..entities.len()).collect();
        by_x.sort_unstable_by(|&a, &b| {
            entities[a].position[0]
                .total_cmp(&entities[b].position[0])
                .then_with(|| entities[a].key.cmp(&entities[b].key))
        });
        Ok(Self {
            version,
            navigation,
            entities,
            by_id,
            by_x,
        })
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn entity(&self, key: EntityKey) -> Option<&Entity> {
        let entity = &self.entities[*self.by_id.get(&key.id)?];
        (entity.key == key).then_some(entity)
    }

    /// Exact inclusive sphere candidates over a reusable immutable X index.
    /// Callers validate center/radius before querying. Iteration order is stable.
    pub fn nearby(&self, center: [f64; 3], radius: f64) -> impl Iterator<Item = &Entity> {
        let start = self
            .by_x
            .partition_point(|&i| self.entities[i].position[0] < center[0] - radius);
        let end = self
            .by_x
            .partition_point(|&i| self.entities[i].position[0] <= center[0] + radius);
        self.by_x[start..end]
            .iter()
            .map(|&i| &self.entities[i])
            .filter(move |e| distance_squared(e.position, center) <= radius * radius)
    }
}

#[derive(Debug)]
pub struct WorldStore {
    current: RwLock<Arc<World>>,
}

impl WorldStore {
    /// Epoch must change on process restart. It is an identity, not a credential.
    pub fn new(epoch: String) -> Self {
        let world = World::build(
            Version { epoch, revision: 0 },
            vec![],
            Arc::new(Navigation::default()),
        )
        .expect("empty world is valid");
        Self {
            current: RwLock::new(Arc::new(world)),
        }
    }

    pub fn latest(&self) -> CoreResult<Arc<World>> {
        self.current
            .read()
            .map(|w| Arc::clone(&w))
            .map_err(|_| CoreError::Unavailable)
    }

    pub fn snapshot(&self, version: &Version) -> CoreResult<Arc<World>> {
        let world = self.latest()?;
        if &world.version != version {
            return Err(CoreError::Conflict);
        }
        Ok(world)
    }

    pub fn publish(&self, update: Publish) -> CoreResult<Arc<World>> {
        let old = self.snapshot(&update.base)?;
        let revision = old
            .version
            .revision
            .checked_add(1)
            .ok_or(CoreError::Invalid(
                "world revision exhausted; restart required",
            ))?;
        let navigation = match update.navigation {
            Some(nodes) => Arc::new(Navigation::build(nodes)?),
            None => old.navigation.clone(),
        };
        // Expensive construction happens without holding a read/write lock.
        let next = Arc::new(World::build(
            Version {
                epoch: old.version.epoch.clone(),
                revision,
            },
            update.entities,
            navigation,
        )?);
        let mut current = self.current.write().map_err(|_| CoreError::Unavailable)?;
        if current.version != update.base {
            return Err(CoreError::Conflict);
        }
        *current = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn entity(id: u32, x: f64, faction: u16) -> Entity {
        Entity {
            key: EntityKey { id, generation: 1 },
            position: [x, 0.0, 0.0],
            faction,
            alive: true,
        }
    }
    pub fn world(entities: Vec<Entity>) -> Arc<World> {
        let store = WorldStore::new("test".into());
        store
            .publish(Publish {
                base: store.latest().unwrap().version.clone(),
                entities,
                navigation: None,
            })
            .unwrap()
    }
    #[test]
    fn immutable_snapshots_cas_and_entity_generation() {
        let store = WorldStore::new("test".into());
        let initial = store.latest().unwrap();
        let next = store
            .publish(Publish {
                base: initial.version.clone(),
                entities: vec![entity(1, 0.0, 1)],
                navigation: None,
            })
            .unwrap();
        assert_eq!(initial.entity_count(), 0);
        assert_eq!(next.entity_count(), 1);
        assert_eq!(next.version.revision, 1);
        assert!(next
            .entity(EntityKey {
                id: 1,
                generation: 2
            })
            .is_none());
        assert_eq!(
            store.snapshot(&initial.version).unwrap_err(),
            CoreError::Conflict
        );
        assert_eq!(
            store
                .publish(Publish {
                    base: initial.version.clone(),
                    entities: vec![],
                    navigation: None
                })
                .unwrap_err(),
            CoreError::Conflict
        );
    }
    #[test]
    fn invalid_publish_is_atomic() {
        let store = WorldStore::new("test".into());
        let base = store.latest().unwrap().version.clone();
        assert!(store
            .publish(Publish {
                base: base.clone(),
                entities: vec![entity(1, 0.0, 1), entity(1, 1.0, 2)],
                navigation: None
            })
            .is_err());
        assert_eq!(store.latest().unwrap().version, base);
        let mut invalid = entity(1, 0.0, 1);
        invalid.position[0] = f64::INFINITY;
        assert!(store
            .publish(Publish {
                base: base.clone(),
                entities: vec![invalid],
                navigation: None
            })
            .is_err());
        assert_eq!(store.latest().unwrap().version, base);
    }
    #[test]
    fn spatial_index_matches_brute_force() {
        let entities: Vec<Entity> = (1..=1000)
            .map(|id| Entity {
                position: [
                    (id * 7 % 101) as f64,
                    (id * 13 % 89) as f64,
                    (id * 19 % 71) as f64,
                ],
                ..entity(id, 0.0, 1)
            })
            .collect();
        let world = world(entities.clone());
        for radius in [0.0, 5.0, 50.0, 100.0] {
            let mut indexed: Vec<_> = world.nearby([30.0; 3], radius).map(|e| e.key).collect();
            let expected: Vec<_> = entities
                .iter()
                .filter(|e| distance_squared(e.position, [30.0; 3]) <= radius * radius)
                .map(|e| e.key)
                .collect();
            indexed.sort_unstable();
            assert_eq!(indexed, expected);
        }
    }
    #[test]
    fn concurrent_publish_has_one_winner() {
        let store = WorldStore::new("test".into());
        let base = store.latest().unwrap().version.clone();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (1..=8)
                .map(|id| {
                    let base = base.clone();
                    let store = &store;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        store
                            .publish(Publish {
                                base,
                                entities: vec![entity(id, 0.0, 1)],
                                navigation: None,
                            })
                            .is_ok()
                    })
                })
                .collect();
            assert_eq!(
                jobs.into_iter()
                    .map(|job| usize::from(job.join().unwrap()))
                    .sum::<usize>(),
                1
            );
        });
        assert_eq!(store.latest().unwrap().version.revision, 1);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::tests::entity;
    use super::*;
    #[test]
    fn retain_or_clear_graph_and_reject_old_process_epochs() {
        let store = WorldStore::new("old-process".into());
        let first = store
            .publish(Publish {
                base: store.latest().unwrap().version.clone(),
                entities: vec![],
                navigation: Some(vec![Node {
                    id: 1,
                    position: [0.0; 3],
                    edges: vec![],
                }]),
            })
            .unwrap();
        let second = store
            .publish(Publish {
                base: first.version.clone(),
                entities: vec![],
                navigation: None,
            })
            .unwrap();
        assert!(Arc::ptr_eq(&first.navigation, &second.navigation));
        let third = store
            .publish(Publish {
                base: second.version.clone(),
                entities: vec![],
                navigation: Some(vec![]),
            })
            .unwrap();
        assert_eq!(third.navigation.node_count(), 0);
        assert_eq!(first.navigation.node_count(), 1);
        let restarted = WorldStore::new("new-process".into());
        assert_eq!(
            restarted.snapshot(&third.version).unwrap_err(),
            CoreError::Conflict
        );
    }
    #[test]
    fn one_generation_per_slot_and_entity_limit() {
        let store = WorldStore::new("test".into());
        let base = store.latest().unwrap().version.clone();
        let mut recycled = entity(1, 0.0, 1);
        recycled.key.generation = 2;
        assert!(store
            .publish(Publish {
                base: base.clone(),
                entities: vec![entity(1, 0.0, 1), recycled],
                navigation: None
            })
            .is_err());
        assert!(store
            .publish(Publish {
                base: base.clone(),
                entities: (1..=MAX_ENTITIES as u32 + 1)
                    .map(|id| entity(id, 0.0, 1))
                    .collect(),
                navigation: None
            })
            .is_err());
        assert_eq!(store.latest().unwrap().version, base);
    }
}
