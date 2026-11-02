//! Half-edge view over a triangle mesh's position-welded topology.
//!
//! Half-edge `h` is corner `h` of the source index buffer, running from that
//! corner to the next one in its triangle, so `next`, `prev`, and `face` are
//! plain arithmetic and per-corner data maps straight back to the mesh.
//!
//! The view is always manifold. Edges shared by more than two faces, or by two
//! faces with conflicting winding, are cut open into boundaries. Vertices whose
//! faces form more than one fan (bowties) are split into one vertex per fan.

use std::collections::HashMap;

use crate::mesh::Mesh;

const NONE: u32 = u32::MAX;

#[derive(Debug, Clone)]
pub struct HalfEdgeMesh {
    /// Origin vertex of each half-edge.
    origin: Vec<u32>,
    /// Opposite half-edge, or `NONE` on a boundary.
    twin: Vec<u32>,
    /// For each half-edge vertex, the mesh vertex it was welded from.
    vertex_source: Vec<u32>,
    /// One outgoing half-edge per vertex; the boundary one if there is one.
    vertex_halfedge: Vec<u32>,
    non_manifold_edges: usize,
    split_vertices: usize,
}

/// Topology of one connected component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentTopology {
    pub faces: Vec<u32>,
    pub vertex_count: usize,
    pub edge_count: usize,
    pub boundary_loops: usize,
    /// `V − E + F`.
    pub euler_characteristic: i64,
    /// `(2 − χ − b) / 2`.
    pub genus: i64,
}

impl ComponentTopology {
    /// Disk-like: no handles and exactly one boundary loop.
    pub fn is_disk(&self) -> bool {
        self.genus == 0 && self.boundary_loops == 1
    }

    /// Closed surface with no boundary.
    pub fn is_closed(&self) -> bool {
        self.boundary_loops == 0
    }
}

impl HalfEdgeMesh {
    pub fn from_mesh(mesh: &Mesh) -> Self {
        let remap = mesh.position_remap();
        let corner: Vec<u32> = mesh.indices.iter().map(|&i| remap[i as usize]).collect();
        let n = corner.len();

        // Directed edge -> (first half-edge, how many half-edges run that way).
        let mut directed: HashMap<(u32, u32), (u32, u32)> = HashMap::with_capacity(n);
        for h in 0..n as u32 {
            let key = (corner[h as usize], corner[next(h) as usize]);
            directed
                .entry(key)
                .and_modify(|e| e.1 += 1)
                .or_insert((h, 1));
        }

        let mut twin = vec![NONE; n];
        let mut non_manifold_edges = 0;
        for (&(a, b), &(h, count)) in &directed {
            if a == b {
                continue; // Degenerate edge inside a collapsed triangle.
            }
            let reverse = directed.get(&(b, a));
            match reverse {
                Some(&(t, 1)) if count == 1 => twin[h as usize] = t,
                _ => {
                    // Count each bad undirected edge once.
                    let reverse_count = reverse.map_or(0, |r| r.1);
                    let bad = count + reverse_count > 2 || count > 1;
                    if bad && (a < b || reverse_count == 0) {
                        non_manifold_edges += 1;
                    }
                }
            }
        }

        let mut mesh = Self {
            origin: vec![NONE; n],
            twin,
            vertex_source: Vec::new(),
            vertex_halfedge: Vec::new(),
            non_manifold_edges,
            split_vertices: 0,
        };
        mesh.assign_vertices(&corner);
        mesh
    }

    /// Gives every fan of half-edges around a welded vertex its own vertex id.
    fn assign_vertices(&mut self, corner: &[u32]) {
        let n = corner.len();
        // Outgoing half-edges grouped by welded vertex (CSR layout).
        let vertex_limit = corner.iter().map(|&v| v as usize + 1).max().unwrap_or(0);
        let mut start = vec![0u32; vertex_limit + 1];
        for &v in corner {
            start[v as usize + 1] += 1;
        }
        for i in 0..vertex_limit {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut outgoing = vec![0u32; n];
        for (h, &v) in corner.iter().enumerate() {
            outgoing[fill[v as usize] as usize] = h as u32;
            fill[v as usize] += 1;
        }

        let mut fans_per_vertex = vec![0u32; vertex_limit];
        for v in 0..vertex_limit {
            for k in start[v]..start[v + 1] {
                let h = outgoing[k as usize];
                if self.origin[h as usize] != NONE {
                    continue;
                }
                let id = self.vertex_source.len() as u32;
                self.vertex_source.push(v as u32);
                self.vertex_halfedge.push(h);
                fans_per_vertex[v] += 1;
                self.walk_fan(h, id);
            }
        }
        self.split_vertices = fans_per_vertex.iter().filter(|&&f| f > 1).count();
    }

    /// Labels the fan of outgoing half-edges containing `h` with vertex `id`.
    fn walk_fan(&mut self, h: u32, id: u32) {
        let claim = |mesh: &mut Self, g: u32| {
            mesh.origin[g as usize] = id;
            if mesh.twin[g as usize] == NONE {
                mesh.vertex_halfedge[id as usize] = g;
            }
        };
        claim(self, h);
        // Clockwise: twin of the previous half-edge leaves the same vertex.
        let mut g = h;
        while let Some(t) = self.twin(prev(g)) {
            if self.origin[t as usize] != NONE {
                break;
            }
            claim(self, t);
            g = t;
        }
        // Counter-clockwise: the half-edge after our twin leaves the same vertex.
        let mut g = h;
        while let Some(t) = self.twin(g) {
            let n = next(t);
            if self.origin[n as usize] != NONE {
                break;
            }
            claim(self, n);
            g = n;
        }
    }

    pub fn halfedge_count(&self) -> usize {
        self.origin.len()
    }

    pub fn face_count(&self) -> usize {
        self.origin.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.vertex_source.len()
    }

    /// Undirected edges: twin pairs plus boundary half-edges.
    pub fn edge_count(&self) -> usize {
        self.twin
            .iter()
            .enumerate()
            .filter(|&(h, &t)| t == NONE || (h as u32) < t)
            .count()
    }

    /// Edges that were cut open because more than two faces, or two faces with
    /// conflicting winding, shared them.
    pub fn non_manifold_edges(&self) -> usize {
        self.non_manifold_edges
    }

    /// Welded vertices that were split because their faces formed several fans.
    pub fn split_vertices(&self) -> usize {
        self.split_vertices
    }

    pub fn origin(&self, h: u32) -> u32 {
        self.origin[h as usize]
    }

    pub fn dest(&self, h: u32) -> u32 {
        self.origin[next(h) as usize]
    }

    pub fn twin(&self, h: u32) -> Option<u32> {
        let t = self.twin[h as usize];
        (t != NONE).then_some(t)
    }

    pub fn is_boundary(&self, h: u32) -> bool {
        self.twin[h as usize] == NONE
    }

    pub fn next(&self, h: u32) -> u32 {
        next(h)
    }

    pub fn prev(&self, h: u32) -> u32 {
        prev(h)
    }

    pub fn face(&self, h: u32) -> u32 {
        h / 3
    }

    /// The mesh vertex a half-edge vertex was welded from.
    pub fn vertex_source(&self, v: u32) -> u32 {
        self.vertex_source[v as usize]
    }

    /// An outgoing half-edge of `v`; on a boundary vertex, the boundary one.
    pub fn vertex_halfedge(&self, v: u32) -> u32 {
        self.vertex_halfedge[v as usize]
    }

    /// The boundary half-edge leaving the end of boundary half-edge `h`.
    fn next_boundary(&self, h: u32) -> u32 {
        let mut g = next(h);
        while let Some(t) = self.twin(g) {
            g = next(t);
        }
        g
    }

    /// Every boundary loop as an ordered list of boundary half-edges.
    pub fn boundary_loops(&self) -> Vec<Vec<u32>> {
        let mut visited = vec![false; self.halfedge_count()];
        let mut loops = Vec::new();
        for h in 0..self.halfedge_count() as u32 {
            if !self.is_boundary(h) || visited[h as usize] {
                continue;
            }
            let mut boundary = Vec::new();
            let mut g = h;
            while !visited[g as usize] {
                visited[g as usize] = true;
                boundary.push(g);
                g = self.next_boundary(g);
            }
            loops.push(boundary);
        }
        loops
    }

    /// Connected components (through shared edges) with their topology.
    pub fn components(&self) -> Vec<ComponentTopology> {
        let faces = self.face_count();
        let mut parent: Vec<u32> = (0..faces as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for h in 0..self.halfedge_count() as u32 {
            if let Some(t) = self.twin(h) {
                let (a, b) = (find(&mut parent, h / 3), find(&mut parent, t / 3));
                if a != b {
                    parent[a.max(b) as usize] = a.min(b);
                }
            }
        }

        let mut component_of_root: HashMap<u32, usize> = HashMap::new();
        let mut components: Vec<ComponentTopology> = Vec::new();
        let mut face_component = vec![0usize; faces];
        for f in 0..faces as u32 {
            let root = find(&mut parent, f);
            let c = *component_of_root.entry(root).or_insert_with(|| {
                components.push(ComponentTopology {
                    faces: Vec::new(),
                    vertex_count: 0,
                    edge_count: 0,
                    boundary_loops: 0,
                    euler_characteristic: 0,
                    genus: 0,
                });
                components.len() - 1
            });
            components[c].faces.push(f);
            face_component[f as usize] = c;
        }
        for v in 0..self.vertex_count() {
            let c = face_component[(self.vertex_halfedge[v] / 3) as usize];
            components[c].vertex_count += 1;
        }
        for (h, &t) in self.twin.iter().enumerate() {
            if t == NONE || (h as u32) < t {
                components[face_component[h / 3]].edge_count += 1;
            }
        }
        for boundary in self.boundary_loops() {
            components[face_component[(boundary[0] / 3) as usize]].boundary_loops += 1;
        }
        for c in &mut components {
            c.euler_characteristic =
                c.vertex_count as i64 - c.edge_count as i64 + c.faces.len() as i64;
            c.genus = (2 - c.euler_characteristic - c.boundary_loops as i64) / 2;
        }
        components
    }
}

fn next(h: u32) -> u32 {
    if h % 3 == 2 { h - 2 } else { h + 1 }
}

fn prev(h: u32) -> u32 {
    if h.is_multiple_of(3) { h + 2 } else { h - 1 }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::mesh::Submesh;
    use crate::primitives;

    fn only_component(mesh: &Mesh) -> ComponentTopology {
        let he = HalfEdgeMesh::from_mesh(mesh);
        let mut components = he.components();
        assert_eq!(components.len(), 1, "{} should be one piece", mesh.name);
        components.remove(0)
    }

    fn raw_mesh(positions: Vec<Vec3>, indices: Vec<u32>) -> Mesh {
        let count = indices.len() as u32;
        Mesh {
            name: "raw".into(),
            positions,
            indices,
            submeshes: vec![Submesh {
                index_start: 0,
                index_count: count,
                material: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn cube_is_a_closed_sphere() {
        let t = only_component(&primitives::cube(1.0));
        assert_eq!((t.vertex_count, t.edge_count, t.faces.len()), (8, 18, 12));
        assert_eq!(
            (t.euler_characteristic, t.boundary_loops, t.genus),
            (2, 0, 0)
        );
        assert!(t.is_closed());
    }

    #[test]
    fn open_cylinder_has_two_loops() {
        let mesh = primitives::cylinder(1.0, 2.0, 16, 4);
        let t = only_component(&mesh);
        assert_eq!(t.vertex_count, 16 * 5, "the UV seam must weld");
        assert_eq!(
            (t.euler_characteristic, t.boundary_loops, t.genus),
            (0, 2, 0)
        );
        let loops = HalfEdgeMesh::from_mesh(&mesh).boundary_loops();
        assert!(loops.iter().all(|l| l.len() == 16));
    }

    #[test]
    fn torus_has_genus_one() {
        let t = only_component(&primitives::torus(1.0, 0.3, 24, 12));
        assert_eq!(
            (t.euler_characteristic, t.boundary_loops, t.genus),
            (0, 0, 1)
        );
    }

    #[test]
    fn sphere_poles_weld() {
        let t = only_component(&primitives::uv_sphere(1.0, 16, 8));
        assert!(t.is_closed());
        assert_eq!(t.genus, 0);
    }

    #[test]
    fn grid_is_a_disk() {
        let t = only_component(&primitives::grid(1.0, 1.0, 5, 3));
        assert!(t.is_disk());
        let loops = HalfEdgeMesh::from_mesh(&primitives::grid(1.0, 1.0, 5, 3)).boundary_loops();
        assert_eq!(loops[0].len(), 2 * (5 + 3));
    }

    #[test]
    fn boundary_loops_are_connected_chains() {
        let mesh = primitives::cylinder(1.0, 1.0, 8, 3);
        let he = HalfEdgeMesh::from_mesh(&mesh);
        for boundary in he.boundary_loops() {
            for (k, &h) in boundary.iter().enumerate() {
                let following = boundary[(k + 1) % boundary.len()];
                assert_eq!(he.dest(h), he.origin(following));
            }
        }
    }

    #[test]
    fn twins_are_symmetric_and_reversed() {
        let he = HalfEdgeMesh::from_mesh(&primitives::torus(1.0, 0.3, 12, 8));
        for h in 0..he.halfedge_count() as u32 {
            let t = he.twin(h).expect("torus has no boundary");
            assert_eq!(he.twin(t), Some(h));
            assert_eq!((he.origin(t), he.dest(t)), (he.dest(h), he.origin(h)));
        }
    }

    #[test]
    fn fin_edge_is_cut_open() {
        // Three triangles hinged on the edge 0–1.
        let positions = vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::NEG_X, Vec3::Z];
        let mesh = raw_mesh(positions, vec![0, 1, 2, 1, 0, 3, 0, 1, 4]);
        let he = HalfEdgeMesh::from_mesh(&mesh);
        assert_eq!(he.non_manifold_edges(), 1);
        assert!(
            (0..9).all(|h| he.is_boundary(h)),
            "every edge should be open"
        );
        assert_eq!(he.components().len(), 3);
    }

    #[test]
    fn bowtie_vertex_is_split() {
        // Two triangles touching only at vertex 0.
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::NEG_X, Vec3::NEG_Y];
        let mesh = raw_mesh(positions, vec![0, 1, 2, 0, 3, 4]);
        let he = HalfEdgeMesh::from_mesh(&mesh);
        assert_eq!(he.split_vertices(), 1);
        assert_eq!(he.vertex_count(), 6);
        assert_eq!(he.boundary_loops().len(), 2);
        assert!(he.boundary_loops().iter().all(|l| l.len() == 3));
    }

    #[test]
    fn conflicting_winding_is_cut_open() {
        // Two triangles sharing edge 0–1, both running it the same direction.
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::NEG_Y];
        let mesh = raw_mesh(positions, vec![0, 1, 2, 0, 1, 3]);
        let he = HalfEdgeMesh::from_mesh(&mesh);
        assert_eq!(he.non_manifold_edges(), 1);
        assert_eq!(he.components().len(), 2);
    }
}
