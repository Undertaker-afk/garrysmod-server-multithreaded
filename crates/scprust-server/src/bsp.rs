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
#[derive(Debug, Clone)]
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
    pub(crate) headnode: i32,
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
            leaves.push(Leaf { first, count });
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
        let models = header.data(bytes, 14, 0, 48, 1024)?;
        if models.is_empty() {
            return Err(Error("missing world model"));
        }
        for model in models.chunks_exact(48) {
            let mins = vector(model, 0)?;
            let maxs = vector(model, 12)?;
            vector(model, 24)?;
            if (0..3).any(|i| mins[i] > maxs[i])
                || !child_valid(i32_at(model, 36), nodes.len(), leaves.len())
            {
                return Err(Error("invalid model bounds/headnode"));
            }
        }
        let headnode = i32_at(models, 36);
        let mut map = Self {
            model_count: models.len() / 48,
            has_displacements: header.lumps[26].length != 0,
            header,
            entities,
            planes,
            nodes,
            leaves,
            leaf_brushes,
            brushes,
            sides,
            headnode,
            world_brushes: Vec::new(),
        };
        let ids = map.candidates(None);
        map.world_brushes = ids;
        Ok(map)
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
    /// Conservative whole-segment BSP broadphase. Bounds are (start,end,mins,maxs).
    pub(crate) fn candidates(&self, segment: Option<(Vec3, Vec3, Vec3, Vec3)>) -> Vec<usize> {
        let mut stack = vec![self.headnode];
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
