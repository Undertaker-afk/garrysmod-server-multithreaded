use crate::{distance_squared, CoreError, CoreResult, Entity, EntityKey, World, MAX_COORD};

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Agent {
    pub actor: EntityKey,
    /// Proximity only, not engine line of sight or PVS.
    pub perception_radius: f64,
    pub attack_radius: f64,
    pub move_speed: f64,
    pub target_factions: Vec<u16>,
    pub current_target: Option<EntityKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Action {
    Idle,
    Chase,
    AttackCandidate,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Intent {
    pub actor: EntityKey,
    pub target: Option<EntityKey>,
    pub action: Action,
    /// Direct steering suggestion only: not collision-aware or authoritative.
    pub desired_velocity: [f64; 3],
    pub distance: Option<f64>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Observer {
    pub actor: EntityKey,
    pub radius: f64,
    pub max_entities: usize,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Interest {
    pub actor: EntityKey,
    pub entities: Vec<EntityKey>,
    pub truncated: bool,
}

fn valid_radius(radius: f64) -> bool {
    radius.is_finite() && (0.0..=MAX_COORD).contains(&radius)
}

impl World {
    pub fn validate_agent(&self, agent: &Agent) -> CoreResult<()> {
        if self.entity(agent.actor).is_none() {
            return Err(CoreError::Invalid("unknown agent identity"));
        }
        if !valid_radius(agent.perception_radius)
            || !valid_radius(agent.attack_radius)
            || agent.attack_radius > agent.perception_radius
            || !agent.move_speed.is_finite()
            || !(0.0..=10000.0).contains(&agent.move_speed)
            || agent.target_factions.is_empty()
            || agent.target_factions.len() > 32
        {
            return Err(CoreError::Invalid("invalid agent configuration"));
        }
        Ok(())
    }

    pub fn think(&self, agent: &Agent) -> CoreResult<Intent> {
        self.validate_agent(agent)?;
        let actor = self.entity(agent.actor).ok_or(CoreError::Unavailable)?;
        let mut intent = Intent {
            actor: actor.key,
            target: None,
            action: Action::Idle,
            desired_velocity: [0.0; 3],
            distance: None,
        };
        if !actor.alive {
            return Ok(intent);
        }
        let eligible = |entity: &&Entity| {
            entity.alive
                && entity.key != actor.key
                && agent.target_factions.contains(&entity.faction)
                && distance_squared(actor.position, entity.position)
                    <= agent.perception_radius * agent.perception_radius
        };
        // Preserve a still-eligible target to prevent unnecessary target thrashing.
        let target = agent
            .current_target
            .and_then(|key| self.entity(key))
            .filter(eligible)
            .or_else(|| {
                self.nearby(actor.position, agent.perception_radius)
                    .filter(eligible)
                    .min_by(|a, b| {
                        distance_squared(actor.position, a.position)
                            .total_cmp(&distance_squared(actor.position, b.position))
                            .then_with(|| a.key.cmp(&b.key))
                    })
            });
        if let Some(target) = target {
            let distance = distance_squared(actor.position, target.position).sqrt();
            intent.target = Some(target.key);
            intent.distance = Some(distance);
            if distance <= agent.attack_radius {
                intent.action = Action::AttackCandidate;
            } else {
                intent.action = Action::Chase;
                for (i, component) in intent.desired_velocity.iter_mut().enumerate() {
                    *component =
                        (target.position[i] - actor.position[i]) / distance * agent.move_speed;
                }
            }
        }
        Ok(intent)
    }

    pub fn validate_observer(&self, observer: &Observer) -> CoreResult<()> {
        if self.entity(observer.actor).is_none()
            || !valid_radius(observer.radius)
            || !(1..=512).contains(&observer.max_entities)
        {
            return Err(CoreError::Invalid(
                "unknown observer or invalid interest limits",
            ));
        }
        Ok(())
    }

    /// Custom addon relevance only. Not Source replication/PVS/visibility.
    pub fn interest(&self, observer: &Observer) -> CoreResult<Interest> {
        self.validate_observer(observer)?;
        let actor = self.entity(observer.actor).ok_or(CoreError::Unavailable)?;
        let mut nearby: Vec<&Entity> = self
            .nearby(actor.position, observer.radius)
            .filter(|e| e.key != actor.key)
            .collect();
        let compare = |a: &&Entity, b: &&Entity| {
            distance_squared(a.position, actor.position)
                .total_cmp(&distance_squared(b.position, actor.position))
                .then_with(|| a.key.cmp(&b.key))
        };
        let truncated = nearby.len() > observer.max_entities;
        if truncated {
            // Partition before sorting: avoid sorting the whole world when the
            // caller wants only a small nearest-first relevance set.
            nearby.select_nth_unstable_by(observer.max_entities, compare);
            nearby.truncate(observer.max_entities);
        }
        nearby.sort_unstable_by(compare);
        Ok(Interest {
            actor: actor.key,
            entities: nearby.into_iter().map(|e| e.key).collect(),
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::tests::{entity, world};
    fn agent() -> Agent {
        Agent {
            actor: EntityKey {
                id: 1,
                generation: 1,
            },
            perception_radius: 100.0,
            attack_radius: 5.0,
            move_speed: 10.0,
            target_factions: vec![2],
            current_target: None,
        }
    }
    #[test]
    fn nearest_enemy_ties_are_deterministic_and_sticky() {
        let world = world(vec![
            entity(1, 0.0, 1),
            entity(2, -20.0, 2),
            entity(3, 20.0, 2),
            entity(4, 1.0, 1),
        ]);
        let mut config = agent();
        let intent = world.think(&config).unwrap();
        assert_eq!(intent.action, Action::Chase);
        assert_eq!(intent.target.unwrap().id, 2);
        assert_eq!(intent.desired_velocity, [-10.0, 0.0, 0.0]);
        config.current_target = Some(EntityKey {
            id: 3,
            generation: 1,
        });
        assert_eq!(world.think(&config).unwrap().target.unwrap().id, 3);
        config.current_target = Some(EntityKey {
            id: 3,
            generation: 9,
        });
        assert_eq!(world.think(&config).unwrap().target.unwrap().id, 2);
    }
    #[test]
    fn attack_is_only_a_candidate_and_death_or_range_produces_idle() {
        let world = world(vec![entity(1, 0.0, 1), entity(2, 5.0, 2)]);
        let mut config = agent();
        let intent = world.think(&config).unwrap();
        assert_eq!(intent.action, Action::AttackCandidate);
        assert_eq!(intent.desired_velocity, [0.0; 3]);
        config.perception_radius = 4.0;
        config.attack_radius = 1.0;
        assert_eq!(world.think(&config).unwrap().action, Action::Idle);
        let mut dead = entity(1, 0.0, 1);
        dead.alive = false;
        let world = crate::world::tests::world(vec![dead, entity(2, 1.0, 2)]);
        assert_eq!(world.think(&agent()).unwrap().action, Action::Idle);
    }
    #[test]
    fn interest_is_nearest_first_and_bounded() {
        let world = world(vec![
            entity(1, 0.0, 1),
            entity(2, 4.0, 2),
            entity(3, 2.0, 2),
        ]);
        let result = world
            .interest(&Observer {
                actor: agent().actor,
                radius: 10.0,
                max_entities: 1,
            })
            .unwrap();
        assert!(result.truncated);
        assert_eq!(result.entities[0].id, 3);
        assert!(world
            .interest(&Observer {
                actor: agent().actor,
                radius: f64::NAN,
                max_entities: 1
            })
            .is_err());
        let mut config = agent();
        config.move_speed = f64::INFINITY;
        assert!(world.think(&config).is_err());
    }
    #[test]
    fn bounded_interest_matches_full_sort_reference() {
        let mut entities: Vec<_> = (1..=1000)
            .map(|id| entity(id, ((id * 17) % 41) as f64, 1))
            .collect();
        let actor = entities[0].clone();
        let world = world(entities.clone());
        entities.retain(|e| e.key != actor.key);
        entities.sort_unstable_by(|a, b| {
            distance_squared(a.position, actor.position)
                .total_cmp(&distance_squared(b.position, actor.position))
                .then_with(|| a.key.cmp(&b.key))
        });
        for max_entities in [1, 10, 150, 512] {
            let result = world
                .interest(&Observer {
                    actor: actor.key,
                    radius: 100.0,
                    max_entities,
                })
                .unwrap();
            assert_eq!(
                result.entities,
                entities[..max_entities]
                    .iter()
                    .map(|e| e.key)
                    .collect::<Vec<_>>()
            );
            assert!(result.truncated);
        }
    }

    #[test]
    fn concurrent_readers_produce_identical_intents() {
        let world = world(
            (1..=150)
                .map(|i| entity(i, i as f64, if i == 1 { 1 } else { 2 }))
                .collect(),
        );
        let expected = world.think(&agent()).unwrap();
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|_| {
                    let world = &world;
                    scope.spawn(move || world.think(&agent()).unwrap())
                })
                .collect();
            for job in jobs {
                assert_eq!(job.join().unwrap(), expected);
            }
        });
    }
}
