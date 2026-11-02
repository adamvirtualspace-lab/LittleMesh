//! Mesh and scene statistics for the CLI and the stats panel.

use serde::Serialize;

use crate::{HalfEdgeMesh, Mesh, Scene};

#[derive(Debug, Clone, Serialize)]
pub struct MeshStats {
    pub name: String,
    pub vertices: usize,
    /// Vertices after welding split copies that share a position.
    pub unique_positions: usize,
    pub triangles: usize,
    pub submeshes: usize,
    pub uv_channels: usize,
    pub has_normals: bool,
    pub has_tangents: bool,
    pub has_colors: bool,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub components: usize,
    pub boundary_loops: usize,
    pub non_manifold_edges: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneStats {
    pub meshes: Vec<MeshStats>,
    pub materials: usize,
    pub images: usize,
    pub total_vertices: usize,
    pub total_triangles: usize,
}

pub fn mesh_stats(mesh: &Mesh) -> MeshStats {
    let topology = HalfEdgeMesh::from_mesh(mesh);
    let (lo, hi) = mesh.bounds().unwrap_or_default();
    MeshStats {
        name: mesh.name.clone(),
        vertices: mesh.vertex_count(),
        unique_positions: mesh.unique_position_count(),
        triangles: mesh.triangle_count(),
        submeshes: mesh.submeshes.len(),
        uv_channels: mesh.uvs.len(),
        has_normals: mesh.normals.is_some(),
        has_tangents: mesh.tangents.is_some(),
        has_colors: mesh.colors.is_some(),
        bounds_min: lo.to_array(),
        bounds_max: hi.to_array(),
        components: topology.components().len(),
        boundary_loops: topology.boundary_loops().len(),
        non_manifold_edges: topology.non_manifold_edges(),
    }
}

pub fn scene_stats(scene: &Scene) -> SceneStats {
    let meshes: Vec<MeshStats> = scene.meshes.iter().map(mesh_stats).collect();
    SceneStats {
        total_vertices: meshes.iter().map(|m| m.vertices).sum(),
        total_triangles: meshes.iter().map(|m| m.triangles).sum(),
        materials: scene.materials.len(),
        images: scene.images.len(),
        meshes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn cylinder_stats() {
        let s = mesh_stats(&primitives::cylinder(1.0, 2.0, 8, 2));
        assert_eq!((s.vertices, s.unique_positions, s.triangles), (27, 24, 32));
        assert_eq!(
            (s.components, s.boundary_loops, s.non_manifold_edges),
            (1, 2, 0)
        );
        assert_eq!(s.bounds_min[1], -1.0);
        assert_eq!(s.bounds_max[1], 1.0);
    }
}
