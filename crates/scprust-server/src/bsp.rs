//! Strict little-endian VBSP v20 subset: world brush geometry and entity data.
//! This is not a complete map loader or Source collision parity implementation.
use crate::entities::{self, Entity};
use std::{collections::BTreeSet, fmt, io::Read, path::Path};
pub const HEADER_BYTES: usize = 1036;
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;
pub type Vec3 = [f64; 3];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default)]
pub struct Lump {
    pub offset: usize,
    pub length: usize,
    pub version: i32,
    pub compression: i32,
}
#[derive(Debug)]
pub struct Header {
    pub revision: i32,
    pub lumps: [Lump; 64],
}
fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(
        bytes[at..at + 4]
            .try_into()
            .expect("validated fixed record"),
    )
}
fn u16_at(bytes: &[u8], at: usize) -> usize {
    u16::from_le_bytes(
        bytes[at..at + 2]
            .try_into()
            .expect("validated fixed record"),
    ) as usize
}
fn nonnegative(n: i32) -> Result<usize> {
    usize::try_from(n).map_err(|_| Error("negative index/offset/count"))
}
fn vector(bytes: &[u8], at: usize) -> Result<Vec3> {
    let mut v = [0.0; 3];
    for (i, component) in v.iter_mut().enumerate() {
        *component = f32::from_bits(i32_at(bytes, at + i * 4) as u32) as f64;
    }
    if !valid_vector(v) {
        return Err(Error("nonfinite/out-of-bounds geometry"));
    }
    Ok(v)
}
pub fn valid_vector(v: Vec3) -> bool {
    v.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
}
pub(crate) fn dot(a: Vec3, b: Vec3) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
impl Header {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_BYTES || bytes.len() > MAX_FILE_BYTES {
            return Err(Error("BSP size out of bounds"));
        }
        if &bytes[..4] != b"VBSP" {
            return Err(Error("expected little-endian VBSP"));
        }
        if i32_at(bytes, 4) != 20 {
            return Err(Error("only BSP version 20 is supported"));
        }
        let mut lumps = [Lump::default(); 64];
        let mut ranges = Vec::new();
        for (i, lump) in lumps.iter_mut().enumerate() {
            let at = 8 + i * 16;
            *lump = Lump {
                offset: nonnegative(i32_at(bytes, at))?,
                length: nonnegative(i32_at(bytes, at + 4))?,
                version: i32_at(bytes, at + 8),
                compression: i32_at(bytes, at + 12),
            };
            let end = lump
                .offset
                .checked_add(lump.length)
                .ok_or(Error("lump range overflow"))?;
            if end > bytes.len() {
                return Err(Error("lump extends beyond file"));
            }
            if lump.length > 0 {
                if lump.offset < HEADER_BYTES {
                    return Err(Error("lump overlaps header"));
                }
                ranges.push((lump.offset, end));
            }
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
            return Err(Error("overlapping nonempty lumps"));
        }
        Ok(Self {
            revision: i32_at(bytes, 1032),
            lumps,
        })
    }
    fn data<'a>(
        &self,
        bytes: &'a [u8],
        id: usize,
        version: i32,
        record: usize,
        limit: usize,
    ) -> Result<&'a [u8]> {
        let lump = self.lumps[id];
        if lump.version != version || lump.compression != 0 {
            return Err(Error("unsupported consumed lump version/compression"));
        }
        if !lump.length.is_multiple_of(record) || lump.length / record > limit {
            return Err(Error("invalid lump record length/count"));
        }
        let data = &bytes[lump.offset..lump.offset + lump.length];
        if data.starts_with(b"LZMA") {
            return Err(Error("compressed consumed lump unsupported"));
        }
        Ok(data)
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Plane {
    pub normal: Vec3,
    pub distance: f64,
}
#[derive(Debug)]
pub(crate) struct Node {
    pub plane: usize,
    pub children: [i32; 2],
}
#[derive(Debug)]
pub(crate) struct Brush {
    pub first: usize,
    pub count: usize,
    pub contents: u32,
}
#[derive(Debug)]
pub(crate) struct Leaf {
    pub first: usize,
    pub count: usize,
    pub cluster: i32,
    pub contents: u32,
}
#[derive(Debug)]
pub(crate) struct Model {
    pub mins: Vec3,
    pub maxs: Vec3,
    pub headnode: i32,
    pub brushes: Vec<usize>,
}
/// Brush entity backed by a BSP model, at its entity transform. Static: doors,
/// movers and animated brush entities are not simulated here.
#[derive(Debug, Clone, PartialEq)]
pub struct SolidModel {
    pub index: usize,
    pub name: String,
    pub origin: Vec3,
    pub angles: Vec3,
    pub brush_count: usize,
}
/// Model-space to world-space transform for brush entities.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Transform {
    origin: Vec3,
    rotation: [[f64; 3]; 3],
    pub(crate) identity: bool,
}
fn matrix_vector(m: &[[f64; 3]; 3], v: Vec3) -> Vec3 {
    [0, 1, 2].map(|row| m[row][0] * v[0] + m[row][1] * v[1] + m[row][2] * v[2])
}
impl Transform {
    fn new(origin: Vec3, angles: Vec3) -> Result<Self> {
        if !valid_vector(origin) || !valid_vector(angles) {
            return Err(Error("invalid brush-entity transform"));
        }
        if origin == [0.0; 3] && angles == [0.0; 3] {
            return Ok(Self {
                origin,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                identity: true,
            });
        }
        let (pitch, yaw, roll) = (
            angles[0].to_radians(),
            angles[1].to_radians(),
            angles[2].to_radians(),
        );
        let (sp, cp) = (pitch.sin(), pitch.cos());
        let (sy, cy) = (yaw.sin(), yaw.cos());
        let (sr, cr) = (roll.sin(), roll.cos());
        let forward = [cp * cy, cp * sy, -sp];
        let right = [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp];
        let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
        if [forward, right, up]
            .iter()
            .flatten()
            .any(|v| !v.is_finite())
        {
            return Err(Error("nonfinite brush-entity rotation"));
        }
        // Source `right` points along local -y, so a proper rotation with the
        // identity at zero angles uses columns (forward, -right, up). Yaw then
        // matches Source; pitch/roll follow the same basis vectors.
        let rotation = [
            [forward[0], -right[0], up[0]],
            [forward[1], -right[1], up[1]],
            [forward[2], -right[2], up[2]],
        ];
        Ok(Self {
            origin,
            rotation,
            identity: false,
        })
    }
    /// Plane (n, d) in model space becomes (R n, d + dot(R n, origin)).
    pub(crate) fn plane(&self, base: &Plane) -> Plane {
        let normal = matrix_vector(&self.rotation, base.normal);
        Plane {
            normal,
            distance: base.distance + dot(normal, self.origin),
        }
    }
}
fn parse_model_reference(value: &[u8]) -> Option<usize> {
    let digits = value.strip_prefix(b"*")?;
    if digits.is_empty() || digits.len() > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}
fn entity_vector(entity: &entities::Entity, key: &[u8]) -> Result<Option<Vec3>> {
    let Some(value) = entity.get(key) else {
        return Ok(None);
    };
    let parts: Vec<&[u8]> = value
        .split(|b| b.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return Err(Error("entity origin/angles is not a three-number vector"));
    }
    let mut out = [0.0; 3];
    for (slot, part) in out.iter_mut().zip(parts) {
        *slot = std::str::from_utf8(part)
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or(Error("invalid entity vector number"))?;
    }
    if !valid_vector(out) {
        return Err(Error("out-of-bounds entity vector"));
    }
    Ok(Some(out))
}
#[derive(Debug)]
pub struct Map {
    pub header: Header,
    pub entities: Vec<Entity>,
    pub model_count: usize,
    pub has_displacements: bool,
    pub(crate) planes: Vec<Plane>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) leaves: Vec<Leaf>,
    pub(crate) leaf_brushes: Vec<usize>,
    pub(crate) brushes: Vec<Brush>,
    pub(crate) sides: Vec<usize>,
    pub(crate) models: Vec<Model>,
    pub(crate) solid_models: Vec<SolidModel>,
    pub(crate) solid_transforms: Vec<(usize, Transform)>,
    pub(crate) world_brushes: Vec<usize>,
}
fn leaf_index(child: i32) -> usize {
    (-i64::from(child) - 1) as usize
}
fn child_valid(child: i32, nodes: usize, leaves: usize) -> bool {
    if child < 0 {
        leaf_index(child) < leaves
    } else {
        (child as usize) < nodes
    }
}
impl Map {
    pub fn from_file(
        path: impl AsRef<Path>,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > MAX_FILE_BYTES as u64 {
            return Err(Box::new(Error("BSP exceeds file size limit")));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Ok(Self::parse(&bytes)?)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = Header::parse(bytes)?;
        let entity_bytes = header.data(bytes, 0, 0, 1, entities::MAX_ENTITY_BYTES)?;
        let entities = entities::parse(entity_bytes)?;
        if entities.first().and_then(|e| e.get(b"classname")) != Some(&b"worldspawn"[..])
            || entities
                .iter()
                .filter(|e| e.get(b"classname") == Some(&b"worldspawn"[..]))
                .count()
                != 1
        {
            return Err(Error("expected exactly one leading worldspawn"));
        }
        let mut planes = Vec::new();
        for p in header.data(bytes, 1, 0, 20, 65_536)?.chunks_exact(20) {
            let normal = vector(p, 0)?;
            let distance = f32::from_bits(i32_at(p, 12) as u32) as f64;
            if !(0.99..=1.01).contains(&dot(normal, normal))
                || !distance.is_finite()
                || distance.abs() > 1_000_000.0
            {
                return Err(Error("invalid plane normal/distance"));
            }
            planes.push(Plane { normal, distance });
        }
        let mut nodes = Vec::new();
        for n in header.data(bytes, 5, 0, 32, 131_072)?.chunks_exact(32) {
            let plane = nonnegative(i32_at(n, 0))?;
            if plane >= planes.len() {
                return Err(Error("node references unknown plane"));
            }
            nodes.push(Node {
                plane,
                children: [i32_at(n, 4), i32_at(n, 8)],
            });
        }
        let mut sides = Vec::new();
        for s in header.data(bytes, 19, 0, 8, 262_144)?.chunks_exact(8) {
            let plane = u16_at(s, 0);
            if plane >= planes.len() {
                return Err(Error("brush side references unknown plane"));
            }
            sides.push(plane);
        }
        let mut brushes = Vec::new();
        for b in header.data(bytes, 18, 0, 12, 65_536)?.chunks_exact(12) {
            let first = nonnegative(i32_at(b, 0))?;
            let count = nonnegative(i32_at(b, 4))?;
            if !(4..=128).contains(&count)
                || first.checked_add(count).is_none_or(|end| end > sides.len())
            {
                return Err(Error("brush side range/count invalid"));
            }
            brushes.push(Brush {
                first,
                count,
                contents: i32_at(b, 8) as u32,
            });
        }
        let mut leaf_brushes = Vec::new();
        for b in header.data(bytes, 17, 0, 2, 262_144)?.chunks_exact(2) {
            let brush = u16_at(b, 0);
            if brush >= brushes.len() {
                return Err(Error("leaf references unknown brush"));
            }
            leaf_brushes.push(brush);
        }
        let mut leaves = Vec::new();
        for l in header.data(bytes, 10, 1, 32, 65_536)?.chunks_exact(32) {
            let first = u16_at(l, 24);
            let count = u16_at(l, 26);
            if first + count > leaf_brushes.len() {
                return Err(Error("leaf brush range invalid"));
            }
            leaves.push(Leaf {
                first,
                count,
                cluster: i16::from_le_bytes(l[4..6].try_into().expect("validated leaf")) as i32,
                contents: i32_at(l, 0) as u32,
            });
        }
        for n in &nodes {
            for child in n.children {
                if !child_valid(child, nodes.len(), leaves.len()) {
                    return Err(Error("invalid BSP child reference"));
                }
            }
        }
        // Iterative three-color DFS validates all nodes, including unreachable
        // submodel nodes. No attacker-controlled recursion or traversal cycles.
        let mut colors = vec![0u8; nodes.len()];
        for root in 0..nodes.len() {
            let mut stack = vec![(root, false)];
            while let Some((node, leaving)) = stack.pop() {
                if leaving {
                    colors[node] = 2;
                    continue;
                }
                if colors[node] == 1 {
                    return Err(Error("cyclic BSP nodes"));
                }
                if colors[node] == 2 {
                    continue;
                }
                colors[node] = 1;
                stack.push((node, true));
                for child in nodes[node].children.into_iter().rev() {
                    if child >= 0 {
                        stack.push((child as usize, false));
                    }
                }
            }
        }
        let model_bytes = header.data(bytes, 14, 0, 48, 1024)?;
        if model_bytes.is_empty() {
            return Err(Error("missing world model"));
        }
        let mut models = Vec::with_capacity(model_bytes.len() / 48);
        for record in model_bytes.chunks_exact(48) {
            let mins = vector(record, 0)?;
            let maxs = vector(record, 12)?;
            vector(record, 24)?; // model origin (sound/light placement), unused
            let headnode = i32_at(record, 36);
            if (0..3).any(|i| mins[i] > maxs[i])
                || !child_valid(headnode, nodes.len(), leaves.len())
            {
                return Err(Error("invalid model bounds/headnode"));
            }
            let model_brushes = collect_brushes(headnode, &nodes, &leaves, &leaf_brushes)?;
            models.push(Model {
                mins,
                maxs,
                headnode,
                brushes: model_brushes,
            });
        }
        let (solid_models, solid_transforms) = resolve_solid_models(&entities, &models)?;
        let mut map = Self {
            model_count: models.len(),
            has_displacements: header.lumps[26].length != 0,
            header,
            entities,
            planes,
            nodes,
            leaves,
            leaf_brushes,
            brushes,
            sides,
            models,
            solid_models,
            solid_transforms,
            world_brushes: Vec::new(),
        };
        map.world_brushes = map.candidate_brushes(map.models[0].headnode, None);
        Ok(map)
    }
    pub fn solid_models(&self) -> &[SolidModel] {
        &self.solid_models
    }
    pub fn world_model_bounds(&self) -> (Vec3, Vec3) {
        (self.models[0].mins, self.models[0].maxs)
    }
    /// Number of distinct non-negative leaf clusters declared by the leaf lump.
    /// Visibility/PVS data itself is not decoded yet.
    pub fn cluster_count(&self) -> usize {
        self.leaves
            .iter()
            .filter(|leaf| leaf.cluster >= 0)
            .map(|leaf| leaf.cluster as usize + 1)
            .max()
            .unwrap_or(0)
    }
    /// Union of leaf contents flags (for diagnostics, not a trace result).
    pub fn leaf_contents(&self) -> u32 {
        self.leaves.iter().fold(0, |all, leaf| all | leaf.contents)
    }
    pub(crate) fn transform_for(&self, model: usize) -> Transform {
        self.solid_transforms
            .iter()
            .find(|(index, _)| *index == model)
            .map(|(_, transform)| *transform)
            .unwrap_or(Transform {
                origin: [0.0; 3],
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                identity: true,
            })
    }
    pub fn plane_count(&self) -> usize {
        self.planes.len()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }
    pub fn world_brush_count(&self) -> usize {
        self.world_brushes.len()
    }
    /// Conservative whole-segment BSP broadphase within one model's subtree.
    /// Bounds are (start,end,mins,maxs) in that model's coordinate space.
    pub(crate) fn candidate_brushes(
        &self,
        root: i32,
        segment: Option<(Vec3, Vec3, Vec3, Vec3)>,
    ) -> Vec<usize> {
        let mut stack = vec![root];
        let mut nodes = vec![false; self.nodes.len()];
        let mut leaves = vec![false; self.leaves.len()];
        let mut brushes = BTreeSet::new();
        while let Some(child) = stack.pop() {
            if child < 0 {
                let index = leaf_index(child);
                if leaves[index] {
                    continue;
                }
                leaves[index] = true;
                let leaf = &self.leaves[index];
                brushes.extend(
                    self.leaf_brushes[leaf.first..leaf.first + leaf.count]
                        .iter()
                        .copied(),
                );
            } else {
                let index = child as usize;
                if nodes[index] {
                    continue;
                }
                nodes[index] = true;
                let node = &self.nodes[index];
                if let Some((start, end, mins, maxs)) = segment {
                    let plane = &self.planes[node.plane];
                    let a = dot(start, plane.normal) - plane.distance;
                    let b = dot(end, plane.normal) - plane.distance;
                    let (lo, hi) = projections(plane.normal, mins, maxs);
                    if a + lo > 0.0 && b + lo > 0.0 {
                        stack.push(node.children[0]);
                        continue;
                    }
                    if a + hi < 0.0 && b + hi < 0.0 {
                        stack.push(node.children[1]);
                        continue;
                    }
                }
                stack.extend(node.children);
            }
        }
        brushes.into_iter().collect()
    }
}
fn collect_brushes(
    root: i32,
    nodes: &[Node],
    leaves: &[Leaf],
    leaf_brushes: &[usize],
) -> Result<Vec<usize>> {
    let mut stack = vec![root];
    let mut seen_nodes = vec![false; nodes.len()];
    let mut seen_leaves = vec![false; leaves.len()];
    let mut found = BTreeSet::new();
    while let Some(child) = stack.pop() {
        if child < 0 {
            let index = leaf_index(child);
            if index >= leaves.len() {
                return Err(Error("invalid model leaf reference"));
            }
            if std::mem::replace(&mut seen_leaves[index], true) {
                continue;
            }
            let leaf = &leaves[index];
            found.extend(
                leaf_brushes[leaf.first..leaf.first + leaf.count]
                    .iter()
                    .copied(),
            );
        } else {
            let index = child as usize;
            if index >= nodes.len() {
                return Err(Error("invalid model node reference"));
            }
            if std::mem::replace(&mut seen_nodes[index], true) {
                continue;
            }
            stack.extend(nodes[index].children);
        }
    }
    Ok(found.into_iter().collect())
}
type SolidModels = (Vec<SolidModel>, Vec<(usize, Transform)>);

fn resolve_solid_models(entities: &[entities::Entity], models: &[Model]) -> Result<SolidModels> {
    let mut info = Vec::new();
    let mut transforms = Vec::new();
    for entity in entities {
        let Some(value) = entity.get(b"model") else {
            continue;
        };
        let Some(index) = parse_model_reference(value) else {
            continue; // static/dynamic .mdl props are outside BSP brush collision
        };
        if index == 0 || index >= models.len() {
            return Err(Error("brush entity model reference out of range"));
        }
        if transforms.iter().any(|(existing, _)| *existing == index) {
            return Err(Error("multiple entities share one brush model"));
        }
        let origin = entity_vector(entity, b"origin")?.unwrap_or([0.0; 3]);
        let angles = entity_vector(entity, b"angles")?.unwrap_or([0.0; 3]);
        let transform = Transform::new(origin, angles)?;
        info.push(SolidModel {
            index,
            name: String::from_utf8_lossy(value).into_owned(),
            origin,
            angles,
            brush_count: models[index].brushes.len(),
        });
        transforms.push((index, transform));
    }
    info.sort_by_key(|model| model.index);
    transforms.sort_by_key(|(index, _)| *index);
    Ok((info, transforms))
}
pub(crate) fn projections(normal: Vec3, mins: Vec3, maxs: Vec3) -> (f64, f64) {
    let mut lo = 0.0;
    let mut hi = 0.0;
    for i in 0..3 {
        let a = normal[i] * mins[i];
        let b = normal[i] * maxs[i];
        lo += a.min(b);
        hi += a.max(b);
    }
    (lo, hi)
}
