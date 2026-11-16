//! Mathematical convex half-space clipping, not Source movement/trace parity.
//! Static world brushes only. Hull sweeps rely on compiled bevel planes and may
//! be conservative when bevels are absent. Displacements/props are not queried.
use crate::bsp::{dot, projections, valid_vector, Error, Map, Result, Vec3};
#[derive(Clone, Copy, Debug)]
pub struct Sweep {
    pub start: Vec3,
    pub end: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub contents_mask: u32,
}
impl Sweep {
    pub fn ray(start: Vec3, end: Vec3, contents_mask: u32) -> Self {
        Self {
            start,
            end,
            mins: [0.0; 3],
            maxs: [0.0; 3],
            contents_mask,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !valid_vector(self.start)
            || !valid_vector(self.end)
            || !valid_vector(self.mins)
            || !valid_vector(self.maxs)
            || (0..3).any(|i| {
                self.mins[i] > self.maxs[i]
                    || self.mins[i].abs() > 65536.0
                    || self.maxs[i].abs() > 65536.0
            })
        {
            return Err(Error("invalid ray/hull coordinates"));
        }
        Ok(())
    }
}
#[derive(Debug, PartialEq)]
pub struct Trace {
    pub fraction: f64,
    pub end_position: Vec3,
    pub start_solid: bool,
    pub all_solid: bool,
    pub plane_normal: Option<Vec3>,
    pub contents: u32,
    pub brush: Option<usize>,
    pub tested_brushes: usize,
}
struct Interval {
    enter: f64,
    exit: f64,
    normal: Option<Vec3>,
    brush: usize,
}
impl Map {
    pub fn trace(&self, sweep: Sweep) -> Result<Trace> {
        sweep.validate()?;
        let mut intervals = Vec::new();
        let mut tested = 0;
        for id in self.candidates(Some((sweep.start, sweep.end, sweep.mins, sweep.maxs))) {
            let brush = &self.brushes[id];
            if brush.contents & sweep.contents_mask == 0 {
                continue;
            }
            tested += 1;
            let mut interval = Interval {
                enter: 0.0,
                exit: 1.0,
                normal: None,
                brush: id,
            };
            let mut intersects = true;
            for side in &self.sides[brush.first..brush.first + brush.count] {
                let plane = &self.planes[*side];
                let (offset, _) = projections(plane.normal, sweep.mins, sweep.maxs);
                let distance = plane.distance - offset;
                let start = dot(plane.normal, sweep.start) - distance;
                let end = dot(plane.normal, sweep.end) - distance;
                if start > 0.0 && end > 0.0 {
                    intersects = false;
                    break;
                }
                if start <= 0.0 && end <= 0.0 {
                    continue;
                }
                let t = start / (start - end);
                if start > end {
                    if t > interval.enter {
                        interval.enter = t;
                        interval.normal = Some(plane.normal);
                    }
                } else {
                    interval.exit = interval.exit.min(t);
                }
                if interval.enter > interval.exit {
                    intersects = false;
                    break;
                }
            }
            if intersects {
                intervals.push(interval);
            }
        }
        intervals.sort_unstable_by(|a, b| {
            a.enter
                .total_cmp(&b.enter)
                .then_with(|| a.brush.cmp(&b.brush))
        });
        let mut trace = Trace {
            fraction: 1.0,
            end_position: sweep.end,
            start_solid: false,
            all_solid: false,
            plane_normal: None,
            contents: 0,
            brush: None,
            tested_brushes: tested,
        };
        if let Some(first) = intervals.first() {
            trace.fraction = first.enter;
            trace.brush = Some(first.brush);
            trace.contents = self.brushes[first.brush].contents;
            trace.plane_normal = first.normal;
            trace.start_solid = first.enter == 0.0;
            // Merge occupied intervals: separate overlapping brushes can together
            // cover the complete segment even when neither covers it alone.
            if trace.start_solid {
                let mut covered = 0.0f64;
                for interval in &intervals {
                    if interval.enter > covered {
                        break;
                    }
                    covered = covered.max(interval.exit);
                }
                trace.all_solid = covered >= 1.0;
            }
            for i in 0..3 {
                trace.end_position[i] =
                    sweep.start[i] + (sweep.end[i] - sweep.start[i]) * trace.fraction;
            }
        }
        Ok(trace)
    }
}

impl Map {
    /// Independent immutable queries, ordered output. Workers are explicit to
    /// avoid accidentally claiming all engine operations can run in parallel.
    pub fn trace_batch(&self, sweeps: &[Sweep], workers: usize) -> Result<Vec<Trace>> {
        if sweeps.len() > 1024 || !(1..=16).contains(&workers) {
            return Err(Error("trace batch/worker limit exceeded"));
        }
        for sweep in sweeps {
            sweep.validate()?;
        }
        if sweeps.is_empty() {
            return Ok(Vec::new());
        }
        if workers == 1 {
            return sweeps.iter().map(|sweep| self.trace(*sweep)).collect();
        }
        let chunk_size = sweeps.len().div_ceil(workers);
        std::thread::scope(|scope| {
            let mut jobs = Vec::new();
            for chunk in sweeps.chunks(chunk_size) {
                jobs.push(
                    std::thread::Builder::new()
                        .name("bsp-trace".into())
                        .spawn_scoped(scope, move || {
                            chunk
                                .iter()
                                .map(|sweep| self.trace(*sweep))
                                .collect::<Result<Vec<_>>>()
                        })
                        .map_err(|_| Error("could not start trace worker"))?,
                );
            }
            let mut output = Vec::with_capacity(sweeps.len());
            for job in jobs {
                output.extend(job.join().map_err(|_| Error("trace worker panicked"))??);
            }
            Ok(output)
        })
    }
}
