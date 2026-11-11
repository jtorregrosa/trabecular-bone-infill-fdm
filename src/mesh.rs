//! Indexed triangle mesh and the handful of mesh operations the pipeline needs:
//! welding, orientation, hole capping, topology checks and component splitting.

use crate::util::FxHashMap;
use rayon::prelude::*;

#[derive(Clone, Debug, Default)]
pub struct TriMesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Result of [`TriMesh::edge_report`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct EdgeReport {
    /// Undirected edges used by exactly one triangle (holes).
    pub boundary: usize,
    /// Undirected edges used by three or more triangles.
    pub non_manifold: usize,
    /// Edges used by two triangles that traverse them in the same direction
    /// (inconsistent winding).
    pub misoriented: usize,
}

impl EdgeReport {
    /// Closed, 2-manifold and consistently oriented.
    pub fn is_watertight(&self) -> bool {
        self.boundary == 0 && self.non_manifold == 0 && self.misoriented == 0
    }
}

/// One connected component of a mesh.
#[derive(Clone, Copy, Debug)]
pub struct Component {
    pub triangles: usize,
    /// Signed volume in mm³: positive for an outward-oriented closed shell,
    /// negative for a cavity.
    pub volume: f64,
}

impl TriMesh {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Builds an indexed mesh from a triangle soup, merging vertices that
    /// coincide after rounding to `1e-5` mm and dropping degenerate triangles.
    pub fn from_soup(soup: &[[[f32; 3]; 3]]) -> Self {
        const Q: f64 = 1e5;
        let mut map: FxHashMap<[i64; 3], u32> = FxHashMap::default();
        map.reserve(soup.len() / 2 + 16);
        let mut vertices = Vec::with_capacity(soup.len() / 2 + 16);
        let mut triangles = Vec::with_capacity(soup.len());
        for tri in soup {
            let mut idx = [0u32; 3];
            for (c, p) in tri.iter().enumerate() {
                let key = [
                    (p[0] as f64 * Q).round() as i64,
                    (p[1] as f64 * Q).round() as i64,
                    (p[2] as f64 * Q).round() as i64,
                ];
                idx[c] = *map.entry(key).or_insert_with(|| {
                    vertices.push(*p);
                    (vertices.len() - 1) as u32
                });
            }
            if idx[0] != idx[1] && idx[1] != idx[2] && idx[0] != idx[2] {
                triangles.push(idx);
            }
        }
        Self { vertices, triangles }
    }

    /// Axis-aligned bounding box as `(min, max)`.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for v in &self.vertices {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a] as f64);
                hi[a] = hi[a].max(v[a] as f64);
            }
        }
        (lo, hi)
    }

    /// Signed enclosed volume (mm³); meaningful for closed meshes.
    pub fn signed_volume(&self) -> f64 {
        self.triangles.par_iter().map(|t| signed_tet_volume(&self.vertices, t)).sum()
    }

    /// Total surface area (mm²).
    pub fn area(&self) -> f64 {
        self.triangles
            .par_iter()
            .map(|t| {
                let n = area_normal(&self.vertices, t);
                0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
            })
            .sum()
    }

    /// Reverses the winding of every triangle.
    pub fn flip(&mut self) {
        for t in &mut self.triangles {
            t.swap(1, 2);
        }
    }

    /// Appends another mesh (no welding between the two).
    pub fn append(&mut self, other: &TriMesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.triangles.extend(other.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }

    /// Counts boundary, non-manifold and misoriented edges.
    pub fn edge_report(&self) -> EdgeReport {
        // Key: (min << 32 | max) << 1 | direction bit, so both directions of
        // an undirected edge sort next to each other.
        let mut keys: Vec<u64> = Vec::with_capacity(self.triangles.len() * 3);
        for t in &self.triangles {
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                let (lo, hi, dir) = if a < b { (a, b, 0u64) } else { (b, a, 1u64) };
                keys.push((((lo as u64) << 32 | hi as u64) << 1) | dir);
            }
        }
        keys.par_sort_unstable();
        let mut report = EdgeReport::default();
        let mut i = 0;
        while i < keys.len() {
            let edge = keys[i] >> 1;
            let mut forward = 0usize;
            let mut backward = 0usize;
            while i < keys.len() && keys[i] >> 1 == edge {
                if keys[i] & 1 == 0 {
                    forward += 1;
                } else {
                    backward += 1;
                }
                i += 1;
            }
            match forward + backward {
                1 => report.boundary += 1,
                2 => {
                    if forward != 1 {
                        report.misoriented += 1;
                    }
                }
                _ => report.non_manifold += 1,
            }
        }
        report
    }

    /// Caps simple holes (loops of boundary edges) with a triangle fan around
    /// the loop centroid. Returns the number of holes closed, or 0 if the
    /// boundary is not a set of simple loops (in which case nothing changes).
    pub fn fill_holes(&mut self) -> usize {
        let mut count: FxHashMap<(u32, u32), (u32, bool)> = FxHashMap::default();
        for t in &self.triangles {
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                let entry = count.entry(key).or_insert((0, a < b));
                entry.0 += 1;
            }
        }
        // Directed boundary edges a -> b as they appear in their only triangle.
        let mut next: FxHashMap<u32, u32> = FxHashMap::default();
        for (&(lo, hi), &(n, forward)) in &count {
            if n == 1 {
                let (a, b) = if forward { (lo, hi) } else { (hi, lo) };
                if next.insert(a, b).is_some() {
                    return 0; // two holes touching at one vertex
                }
            }
        }
        if next.is_empty() {
            return 0;
        }
        let mut starts: Vec<u32> = next.keys().copied().collect();
        starts.sort_unstable();
        let mut seen: FxHashMap<u32, ()> = FxHashMap::default();
        let mut new_vertices = Vec::new();
        let mut new_triangles = Vec::new();
        let mut holes = 0usize;
        for start in starts {
            if seen.contains_key(&start) {
                continue;
            }
            let mut ring = Vec::new();
            let mut cur = start;
            loop {
                if seen.insert(cur, ()).is_some() {
                    break;
                }
                ring.push(cur);
                match next.get(&cur) {
                    Some(&n) => cur = n,
                    None => return 0, // open chain, not a loop
                }
            }
            if cur != start || ring.len() < 3 {
                return 0;
            }
            let mut c = [0f64; 3];
            for &v in &ring {
                for a in 0..3 {
                    c[a] += self.vertices[v as usize][a] as f64;
                }
            }
            let inv = 1.0 / ring.len() as f64;
            let centre = (self.vertices.len() + new_vertices.len()) as u32;
            new_vertices.push([(c[0] * inv) as f32, (c[1] * inv) as f32, (c[2] * inv) as f32]);
            // The existing face runs a -> b along the rim; the cap runs b -> a.
            for i in 0..ring.len() {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                new_triangles.push([b, a, centre]);
            }
            holes += 1;
        }
        self.vertices.extend(new_vertices);
        self.triangles.extend(new_triangles);
        holes
    }

    /// Labels connected components (triangles sharing a vertex). Returns the
    /// per-triangle component id and a summary per component.
    pub fn components(&self) -> (Vec<u32>, Vec<Component>) {
        let n = self.vertices.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for t in &self.triangles {
            let a = find(&mut parent, t[0]);
            for &v in &t[1..] {
                let b = find(&mut parent, v);
                if a != b {
                    parent[b as usize] = a;
                }
            }
        }
        let mut id_of_root: FxHashMap<u32, u32> = FxHashMap::default();
        let mut comps: Vec<Component> = Vec::new();
        let mut labels = Vec::with_capacity(self.triangles.len());
        for t in &self.triangles {
            let root = find(&mut parent, t[0]);
            let id = *id_of_root.entry(root).or_insert_with(|| {
                comps.push(Component { triangles: 0, volume: 0.0 });
                (comps.len() - 1) as u32
            });
            comps[id as usize].triangles += 1;
            comps[id as usize].volume += signed_tet_volume(&self.vertices, t);
            labels.push(id);
        }
        (labels, comps)
    }

    /// Keeps only the triangles for which `keep` returns true and drops
    /// vertices that are no longer referenced.
    pub fn retain_triangles(&mut self, mut keep: impl FnMut(usize) -> bool) {
        let mut idx = 0usize;
        self.triangles.retain(|_| {
            let k = keep(idx);
            idx += 1;
            k
        });
        self.compact();
    }

    /// Removes unreferenced vertices.
    pub fn compact(&mut self) {
        let mut remap = vec![u32::MAX; self.vertices.len()];
        let mut vertices = Vec::with_capacity(self.vertices.len());
        for t in &mut self.triangles {
            for v in t.iter_mut() {
                let slot = &mut remap[*v as usize];
                if *slot == u32::MAX {
                    *slot = vertices.len() as u32;
                    vertices.push(self.vertices[*v as usize]);
                }
                *v = *slot;
            }
        }
        self.vertices = vertices;
    }
}

#[inline]
fn signed_tet_volume(vertices: &[[f32; 3]], t: &[u32; 3]) -> f64 {
    let a = to_f64(vertices[t[0] as usize]);
    let b = to_f64(vertices[t[1] as usize]);
    let c = to_f64(vertices[t[2] as usize]);
    (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
}

/// Cross product of two triangle edges: direction = normal, length = 2·area.
#[inline]
pub fn area_normal(vertices: &[[f32; 3]], t: &[u32; 3]) -> [f64; 3] {
    let a = to_f64(vertices[t[0] as usize]);
    let b = to_f64(vertices[t[1] as usize]);
    let c = to_f64(vertices[t[2] as usize]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

#[inline]
pub fn to_f64(p: [f32; 3]) -> [f64; 3] {
    [p[0] as f64, p[1] as f64, p[2] as f64]
}

/// Axis-aligned box as a closed, outward-oriented mesh (12 triangles).
pub fn box_mesh(lo: [f32; 3], hi: [f32; 3]) -> TriMesh {
    let v = |x: usize, y: usize, z: usize| {
        [if x == 0 { lo[0] } else { hi[0] }, if y == 0 { lo[1] } else { hi[1] }, if z == 0 { lo[2] } else { hi[2] }]
    };
    let vertices = vec![v(0, 0, 0), v(1, 0, 0), v(0, 1, 0), v(1, 1, 0), v(0, 0, 1), v(1, 0, 1), v(0, 1, 1), v(1, 1, 1)];
    // Corner index = x | y << 1 | z << 2; quads listed counter-clockwise when
    // seen from outside.
    let quads: [[u32; 4]; 6] = [
        [0, 4, 6, 2], // -x
        [1, 3, 7, 5], // +x
        [0, 1, 5, 4], // -y
        [2, 6, 7, 3], // +y
        [0, 2, 3, 1], // -z
        [4, 5, 7, 6], // +z
    ];
    let mut triangles = Vec::with_capacity(12);
    for q in quads {
        triangles.push([q[0], q[1], q[2]]);
        triangles.push([q[0], q[2], q[3]]);
    }
    TriMesh { vertices, triangles }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_is_watertight_with_right_volume() {
        let m = box_mesh([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]);
        assert!(m.edge_report().is_watertight());
        assert!((m.signed_volume() - 24.0).abs() < 1e-9);
        assert!((m.area() - 52.0).abs() < 1e-9);
    }

    #[test]
    fn flipping_negates_volume() {
        let mut m = box_mesh([0.0; 3], [1.0; 3]);
        m.flip();
        assert!((m.signed_volume() + 1.0).abs() < 1e-9);
        assert!(m.edge_report().is_watertight());
    }

    #[test]
    fn hole_is_detected_and_filled() {
        let mut m = box_mesh([0.0; 3], [1.0; 3]);
        m.triangles.truncate(10); // remove the +z face
        let r = m.edge_report();
        assert_eq!(r.boundary, 4);
        assert_eq!(m.fill_holes(), 1);
        assert!(m.edge_report().is_watertight());
        assert!((m.signed_volume() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn components_are_separated() {
        let mut m = box_mesh([0.0; 3], [1.0; 3]);
        let mut cavity = box_mesh([0.25; 3], [0.75; 3]);
        cavity.flip();
        m.append(&cavity);
        let (labels, comps) = m.components();
        assert_eq!(comps.len(), 2);
        assert_eq!(labels.len(), 24);
        assert!((comps[0].volume - 1.0).abs() < 1e-9);
        assert!((comps[1].volume + 0.125).abs() < 1e-9);
    }

    #[test]
    fn soup_welding_merges_shared_vertices() {
        let b = box_mesh([0.0; 3], [1.0; 3]);
        let soup: Vec<[[f32; 3]; 3]> = b
            .triangles
            .iter()
            .map(|t| [b.vertices[t[0] as usize], b.vertices[t[1] as usize], b.vertices[t[2] as usize]])
            .collect();
        let m = TriMesh::from_soup(&soup);
        assert_eq!(m.vertices.len(), 8);
        assert_eq!(m.triangles.len(), 12);
        assert!(m.edge_report().is_watertight());
    }
}
