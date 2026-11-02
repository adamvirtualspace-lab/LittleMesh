//! Cleanup, mesh warnings, and render optimization (docs/algorithms.md §3–4).

use std::collections::HashMap;

use glam::Vec3;
use serde::Serialize;

use crate::{HalfEdgeMesh, Mesh, Scene};

/// What a cleanup pass removed from one mesh.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CleanupReport {
    pub welded_vertices: usize,
    pub degenerate_triangles: usize,
    pub unused_vertices: usize,
}

/// What a cleanup pass removed from the whole scene.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SceneCleanupReport {
    pub meshes: Vec<CleanupReport>,
    pub unused_materials: usize,
    pub unused_images: usize,
}

/// Cleans every mesh, then drops materials and images nothing uses.
pub fn cleanup_scene(scene: &mut Scene) -> SceneCleanupReport {
    let meshes = scene.meshes.iter_mut().map(cleanup).collect();
    let (unused_materials, unused_images) = remove_unused_materials(scene);
    SceneCleanupReport {
        meshes,
        unused_materials,
        unused_images,
    }
}

/// Exact weld, degenerate-triangle removal, and vertex compaction.
pub fn cleanup(mesh: &mut Mesh) -> CleanupReport {
    let before = mesh.vertex_count();
    let welded = weld_exact(mesh);
    let degenerate_triangles = remove_degenerates(mesh);
    let referenced = compact_vertices(mesh);
    CleanupReport {
        welded_vertices: before - welded,
        degenerate_triangles,
        unused_vertices: welded - referenced,
    }
}

/// Bit pattern of a float, with -0.0 folded into 0.0.
fn bits(f: f32) -> u32 {
    (f + 0.0).to_bits()
}

/// Merges vertices whose every attribute is bit-identical. Returns the new
/// vertex count.
fn weld_exact(mesh: &mut Mesh) -> usize {
    let n = mesh.vertex_count();
    let key = |i: usize| {
        let mut k: Vec<u32> = mesh.positions[i].to_array().map(bits).to_vec();
        if let Some(normals) = &mesh.normals {
            k.extend(normals[i].to_array().map(bits));
        }
        if let Some(tangents) = &mesh.tangents {
            k.extend(tangents[i].to_array().map(bits));
        }
        for uv in &mesh.uvs {
            k.extend(uv[i].to_array().map(bits));
        }
        if let Some(colors) = &mesh.colors {
            k.extend(colors[i].to_array().map(bits));
        }
        k
    };
    let mut first: HashMap<Vec<u32>, u32> = HashMap::with_capacity(n);
    let remap: Vec<u32> = (0..n)
        .map(|i| *first.entry(key(i)).or_insert(i as u32))
        .collect();
    for index in &mut mesh.indices {
        *index = remap[*index as usize];
    }
    first.len()
}

/// Drops triangles that repeat a vertex or have exactly zero area, keeping
/// submesh ranges consistent. Returns how many were removed.
fn remove_degenerates(mesh: &mut Mesh) -> usize {
    let mut kept = Vec::with_capacity(mesh.indices.len());
    let mut removed = 0;
    for sub in &mut mesh.submeshes {
        let start = kept.len() as u32;
        let range = sub.index_start as usize..(sub.index_start + sub.index_count) as usize;
        for tri in mesh.indices[range].as_chunks::<3>().0 {
            let [a, b, c] = [tri[0], tri[1], tri[2]];
            let p = |i: u32| mesh.positions[i as usize];
            let area = (p(b) - p(a)).cross(p(c) - p(a));
            if a == b || b == c || a == c || area == Vec3::ZERO {
                removed += 1;
            } else {
                kept.extend_from_slice(tri);
            }
        }
        sub.index_start = start;
        sub.index_count = kept.len() as u32 - start;
    }
    mesh.indices = kept;
    mesh.submeshes.retain(|s| s.index_count > 0);
    removed
}

/// Reorders vertices by first use and drops unreferenced ones. Returns the
/// new vertex count.
///
/// Ordering by first use is also what meshoptimizer's vertex fetch
/// optimization does, so this doubles as that pass.
pub fn compact_vertices(mesh: &mut Mesh) -> usize {
    const UNUSED: u32 = u32::MAX;
    let mut remap = vec![UNUSED; mesh.vertex_count()];
    let mut order = Vec::new();
    for index in &mut mesh.indices {
        let slot = &mut remap[*index as usize];
        if *slot == UNUSED {
            *slot = order.len() as u32;
            order.push(*index as usize);
        }
        *index = *slot;
    }
    fn gather<T: Copy>(data: &[T], order: &[usize]) -> Vec<T> {
        order.iter().map(|&i| data[i]).collect()
    }
    mesh.positions = gather(&mesh.positions, &order);
    if let Some(normals) = &mut mesh.normals {
        *normals = gather(normals, &order);
    }
    if let Some(tangents) = &mut mesh.tangents {
        *tangents = gather(tangents, &order);
    }
    for uv in &mut mesh.uvs {
        *uv = gather(uv, &order);
    }
    if let Some(colors) = &mut mesh.colors {
        *colors = gather(colors, &order);
    }
    order.len()
}

/// Removes materials no submesh uses, then images no material uses.
fn remove_unused_materials(scene: &mut Scene) -> (usize, usize) {
    let mut used = vec![false; scene.materials.len()];
    for sub in scene.meshes.iter().flat_map(|m| &m.submeshes) {
        if let Some(m) = sub.material {
            used[m] = true;
        }
    }
    let material_remap = keep_marked(&mut scene.materials, &used);
    let unused_materials = material_remap.len() - scene.materials.len();
    for sub in scene.meshes.iter_mut().flat_map(|m| &mut m.submeshes) {
        sub.material = sub.material.map(|m| material_remap[m]);
    }

    let mut used = vec![false; scene.images.len()];
    for material in &scene.materials {
        for t in texture_refs(material).into_iter().flatten() {
            used[t.image] = true;
        }
    }
    let image_remap = keep_marked(&mut scene.images, &used);
    let unused_images = image_remap.len() - scene.images.len();
    for material in &mut scene.materials {
        for t in texture_refs_mut(material).into_iter().flatten() {
            t.image = image_remap[t.image];
        }
    }
    (unused_materials, unused_images)
}

/// Keeps the marked items and returns an old-index → new-index table.
fn keep_marked<T>(items: &mut Vec<T>, keep: &[bool]) -> Vec<usize> {
    let mut remap = vec![usize::MAX; items.len()];
    let mut next = 0;
    for (i, &k) in keep.iter().enumerate() {
        if k {
            remap[i] = next;
            next += 1;
        }
    }
    let mut i = 0;
    items.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    remap
}

fn texture_refs(m: &crate::Material) -> [Option<crate::TextureRef>; 5] {
    [
        m.base_color_texture,
        m.metallic_roughness_texture,
        m.normal_texture,
        m.occlusion_texture,
        m.emissive_texture,
    ]
}

fn texture_refs_mut(m: &mut crate::Material) -> [Option<&mut crate::TextureRef>; 5] {
    [
        m.base_color_texture.as_mut(),
        m.metallic_roughness_texture.as_mut(),
        m.normal_texture.as_mut(),
        m.occlusion_texture.as_mut(),
        m.emissive_texture.as_mut(),
    ]
}

/// Something worth telling the user about a mesh. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub mesh: String,
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    NonManifold,
    MissingUvs,
    MissingNormals,
    TinyScale,
    HugeScale,
}

/// Checks one mesh for problems worth reporting (docs/algorithms.md §3).
pub fn mesh_warnings(mesh: &Mesh) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let mut warn = |kind, message: String| {
        warnings.push(Warning {
            mesh: mesh.name.clone(),
            kind,
            message,
        })
    };
    let non_manifold = HalfEdgeMesh::from_mesh(mesh).non_manifold_edges();
    if non_manifold > 0 {
        warn(
            WarningKind::NonManifold,
            format!(
                "{non_manifold} non-manifold edges (shared by more than two faces or with flipped winding); they're treated as open edges"
            ),
        );
    }
    if mesh.uvs.is_empty() {
        warn(
            WarningKind::MissingUvs,
            "no UVs; run Auto UVs before texturing".into(),
        );
    }
    if mesh.normals.is_none() {
        warn(
            WarningKind::MissingNormals,
            "no normals; smooth normals will be generated".into(),
        );
    }
    if let Some((lo, hi)) = mesh.bounds() {
        let size = (hi - lo).max_element();
        if size < 0.01 {
            warn(
                WarningKind::TinyScale,
                format!(
                    "only {:.4} m across; was it exported in the wrong units?",
                    size
                ),
            );
        } else if size > 1000.0 {
            warn(
                WarningKind::HugeScale,
                format!("{:.0} m across; was it exported in the wrong units?", size),
            );
        }
    }
    warnings
}

/// Vertex cache efficiency before and after render optimization.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RenderReport {
    /// Average cache miss ratio: transformed vertices per triangle (lower is better, 0.5 is ideal).
    pub acmr_before: f32,
    pub acmr_after: f32,
}

/// Cache size used for ACMR reporting, a typical post-transform cache.
const CACHE_SIZE: u32 = 16;

pub fn acmr(mesh: &Mesh) -> f32 {
    if mesh.indices.is_empty() {
        return 0.0;
    }
    meshopt::analyze_vertex_cache(&mesh.indices, mesh.vertex_count(), CACHE_SIZE, 0, 0).acmr
}

/// Reorders triangles for the vertex cache and overdraw, then vertices for
/// fetch locality (docs/algorithms.md §4). Geometry is unchanged.
pub fn optimize_render(mesh: &mut Mesh) -> RenderReport {
    let acmr_before = acmr(mesh);
    let vertex_count = mesh.vertex_count();
    let positions = meshopt::typed_to_bytes(&mesh.positions);
    let adapter = meshopt::VertexDataAdapter::new(positions, size_of::<Vec3>(), 0)
        .expect("positions are tightly packed Vec3s");
    for sub in &mesh.submeshes {
        let range = sub.index_start as usize..(sub.index_start + sub.index_count) as usize;
        let slice = &mut mesh.indices[range];
        meshopt::optimize_vertex_cache_in_place(slice, vertex_count);
        meshopt::optimize_overdraw_in_place(slice, &adapter, 1.05);
    }
    compact_vertices(mesh);
    RenderReport {
        acmr_before,
        acmr_after: acmr(mesh),
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec4};

    use super::*;
    use crate::mesh::{Image, Material, Submesh, TextureRef};
    use crate::primitives;

    /// The cube with every vertex duplicated and triangles pointing at either copy.
    fn duplicated_cube() -> Mesh {
        let mut mesh = primitives::cube(1.0);
        let n = mesh.vertex_count() as u32;
        mesh.positions.extend_from_within(..);
        mesh.normals.as_mut().unwrap().extend_from_within(..);
        mesh.uvs[0].extend_from_within(..);
        for (k, index) in mesh.indices.iter_mut().enumerate() {
            if k % 2 == 0 {
                *index += n;
            }
        }
        mesh
    }

    #[test]
    fn weld_merges_identical_vertices() {
        let mut mesh = duplicated_cube();
        let report = cleanup(&mut mesh);
        assert_eq!(report.welded_vertices, 24);
        assert_eq!(mesh.vertex_count(), 24);
        assert_eq!(mesh.triangle_count(), 12);
        mesh.validate().unwrap();
    }

    #[test]
    fn weld_keeps_uv_seams() {
        // The cube's 24 vertices share 8 positions but differ in normal/UV.
        let mut mesh = primitives::cube(1.0);
        let report = cleanup(&mut mesh);
        assert_eq!(report.welded_vertices, 0);
        assert_eq!(mesh.vertex_count(), 24);
    }

    #[test]
    fn weld_folds_negative_zero() {
        // A copy of vertex 0 that differs only by the sign of a zero.
        let mut mesh = primitives::grid(1.0, 1.0, 1, 1);
        mesh.positions[0].y = 0.0;
        mesh.positions
            .push(Vec3::new(mesh.positions[0].x, -0.0, mesh.positions[0].z));
        mesh.normals.as_mut().unwrap().push(Vec3::Y);
        let uv = mesh.uvs[0][0];
        mesh.uvs[0].push(uv);
        mesh.indices[0] = 4;
        let report = cleanup(&mut mesh);
        assert_eq!(report.welded_vertices, 1);
    }

    #[test]
    fn removes_degenerate_triangles_per_submesh() {
        let mut mesh = primitives::cube(1.0);
        mesh.submeshes = vec![
            Submesh {
                index_start: 0,
                index_count: 18,
                material: None,
            },
            Submesh {
                index_start: 18,
                index_count: 18,
                material: None,
            },
        ];
        // A repeated index in the first submesh, a zero-area triangle in the second.
        mesh.indices[1] = mesh.indices[0];
        mesh.positions.push(Vec3::ZERO);
        mesh.positions.push(Vec3::X);
        mesh.positions.push(Vec3::X * 2.0);
        mesh.normals.as_mut().unwrap().extend([Vec3::Y; 3]);
        mesh.uvs[0].extend([Vec2::ZERO, Vec2::X, Vec2::ONE]);
        mesh.indices[18..21].copy_from_slice(&[24, 25, 26]);

        let report = cleanup(&mut mesh);
        assert_eq!(report.degenerate_triangles, 2);
        // The three zero-area vertices, plus one corner from each removed triangle.
        assert_eq!(report.unused_vertices, 5);
        assert_eq!(mesh.triangle_count(), 10);
        assert_eq!(mesh.submeshes[0].index_count, 15);
        assert_eq!(mesh.submeshes[1].index_start, 15);
        mesh.validate().unwrap();
    }

    #[test]
    fn drops_unused_materials_and_images() {
        let image = |name: &str| Image {
            name: name.into(),
            mime_type: "image/png".into(),
            data: vec![1],
        };
        let mut scene = Scene {
            meshes: vec![primitives::cube(1.0)],
            materials: vec![
                Material {
                    name: "unused".into(),
                    base_color_texture: Some(TextureRef {
                        image: 0,
                        uv_channel: 0,
                    }),
                    ..Default::default()
                },
                Material {
                    name: "used".into(),
                    normal_texture: Some(TextureRef {
                        image: 1,
                        uv_channel: 0,
                    }),
                    ..Default::default()
                },
            ],
            images: vec![image("a"), image("b"), image("c")],
        };
        scene.meshes[0].submeshes[0].material = Some(1);
        let report = cleanup_scene(&mut scene);
        assert_eq!((report.unused_materials, report.unused_images), (1, 2));
        assert_eq!(scene.materials[0].name, "used");
        assert_eq!(scene.meshes[0].submeshes[0].material, Some(0));
        assert_eq!(scene.materials[0].normal_texture.unwrap().image, 0);
        assert_eq!(scene.images[0].name, "b");
        scene.validate().unwrap();
    }

    #[test]
    fn warnings_trigger() {
        let kinds = |mesh: &Mesh| {
            mesh_warnings(mesh)
                .into_iter()
                .map(|w| w.kind)
                .collect::<Vec<_>>()
        };
        assert!(kinds(&primitives::cube(1.0)).is_empty());

        let mut no_uvs = primitives::cube(1.0);
        no_uvs.uvs.clear();
        no_uvs.normals = None;
        assert_eq!(
            kinds(&no_uvs),
            [WarningKind::MissingUvs, WarningKind::MissingNormals]
        );

        assert_eq!(kinds(&primitives::cube(0.001)), [WarningKind::TinyScale]);
        assert_eq!(kinds(&primitives::cube(5000.0)), [WarningKind::HugeScale]);

        let mut fin = primitives::grid(1.0, 1.0, 1, 1);
        let extra = fin.indices[0..3].to_vec();
        fin.indices.extend(extra); // The same triangle twice: same winding on shared edges.
        fin.submeshes[0].index_count += 3;
        assert!(kinds(&fin).contains(&WarningKind::NonManifold));
    }

    #[test]
    fn render_optimization_improves_cache_and_keeps_geometry() {
        let mut mesh = primitives::uv_sphere(1.0, 64, 32);
        mesh.colors = Some(vec![Vec4::ONE; mesh.vertex_count()]);
        // Shuffle triangles so the cache order is poor.
        let mut tris: Vec<[u32; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        let len = tris.len();
        for i in 0..len {
            tris.swap(i, (i * 7919) % len);
        }
        mesh.indices = tris.concat();

        let triangle_set = |m: &Mesh| {
            let mut set: Vec<[[u32; 3]; 3]> = m
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let mut corners = t
                        .iter()
                        .map(|&i| m.positions[i as usize].to_array().map(f32::to_bits))
                        .collect::<Vec<_>>();
                    corners.sort();
                    [corners[0], corners[1], corners[2]]
                })
                .collect();
            set.sort();
            set
        };
        let before = triangle_set(&mesh);
        let report = optimize_render(&mut mesh);
        assert!(report.acmr_after < report.acmr_before * 0.8, "{report:?}");
        assert_eq!(triangle_set(&mesh), before);
        mesh.validate().unwrap();
    }
}
