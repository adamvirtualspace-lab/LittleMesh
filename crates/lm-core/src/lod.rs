//! LOD chain generation and screen-size suggestions (docs/algorithms.md §5–6).

use glam::Vec3;
use meshopt::{SimplifyOptions, SimplifyVertex, VertexDataAdapter};
use serde::Serialize;

use crate::Mesh;
use crate::optimize::compact_vertices;

/// How far one LOD level reduces LOD0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LodTarget {
    /// Fraction of LOD0's triangles, e.g. `0.25`.
    Ratio(f32),
    /// Absolute triangle count.
    Triangles(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LodSettings {
    /// One target per level after LOD0.
    pub targets: Vec<LodTarget>,
    /// Collapse-cost weight for normal changes.
    pub normal_weight: f32,
    /// Collapse-cost weight for UV changes.
    pub uv_weight: f32,
    /// Keep open borders in place (for tiles that must stitch).
    pub lock_border: bool,
    /// Stop the chain when a level saves less than this fraction of the previous level's triangles.
    pub min_savings: f32,
    /// Screen height in pixels for screen-size suggestions.
    pub screen_height: f32,
    /// Geometric error allowed on screen, in pixels.
    pub pixel_error: f32,
}

impl Default for LodSettings {
    fn default() -> Self {
        Self {
            targets: vec![
                LodTarget::Ratio(0.5),
                LodTarget::Ratio(0.25),
                LodTarget::Ratio(0.125),
            ],
            normal_weight: 0.5,
            uv_weight: 1.0,
            lock_border: false,
            min_savings: 0.15,
            screen_height: 1080.0,
            pixel_error: 1.0,
        }
    }
}

/// One level of a LOD chain.
#[derive(Debug, Clone)]
pub struct Lod {
    pub level: usize,
    pub mesh: Mesh,
    pub target_triangles: usize,
    /// World-space geometric error versus LOD0.
    pub error: f32,
    /// Suggested switch point: bounding-sphere diameter as a fraction of screen height.
    pub screen_size: f32,
    /// Which simplifier produced this level (the worst one across submeshes).
    pub method: SimplifyMethod,
}

/// The simplifiers, tried in order; each is used only when the previous one
/// stalls well above its triangle target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimplifyMethod {
    /// Attribute-aware; keeps UV and normal seams intact.
    #[default]
    Standard,
    /// Lets collapses cross seams; topology stays intact but UVs may stretch near seams.
    Permissive,
    /// Ignores topology; always reaches its target but can merge separate parts.
    Sloppy,
}

/// Screen size at which a LOD with world error `error` should become active:
/// `s = 2 · r · p_max / (e · H)`, capped at 1 (docs/algorithms.md §6).
pub fn switch_screen_size(radius: f32, error: f32, pixel_error: f32, screen_height: f32) -> f32 {
    if error <= 0.0 {
        return 1.0;
    }
    (2.0 * radius * pixel_error / (error * screen_height)).min(1.0)
}

/// Builds LOD0 (the input) plus one simplified level per target.
///
/// Every level is simplified from LOD0, never from the previous level, so
/// error doesn't compound. Submeshes are simplified separately with the
/// vertices they share locked, so material borders can't crack.
pub fn generate_chain(mesh: &Mesh, settings: &LodSettings) -> Vec<Lod> {
    let mut chain = vec![Lod {
        level: 0,
        mesh: mesh.clone(),
        target_triangles: mesh.triangle_count(),
        error: 0.0,
        screen_size: 1.0,
        method: SimplifyMethod::Standard,
    }];
    let lod0_triangles = mesh.triangle_count();
    if lod0_triangles == 0 {
        return chain;
    }
    let simplifier = Simplifier::new(mesh, settings);
    let radius = mesh
        .bounds()
        .map_or(0.0, |(lo, hi)| (hi - lo).length() * 0.5);

    for (k, target) in settings.targets.iter().enumerate() {
        let target_triangles = match *target {
            LodTarget::Ratio(r) => (lod0_triangles as f32 * r).round() as usize,
            LodTarget::Triangles(t) => t,
        }
        .clamp(1, lod0_triangles);
        let (mut lod_mesh, error, method) = simplifier.simplify(target_triangles);
        let previous = chain.last().unwrap();
        let savings =
            1.0 - lod_mesh.triangle_count() as f32 / previous.mesh.triangle_count() as f32;
        if savings < settings.min_savings {
            break;
        }
        compact_vertices(&mut lod_mesh);
        let screen_size =
            switch_screen_size(radius, error, settings.pixel_error, settings.screen_height)
                .min(previous.screen_size);
        chain.push(Lod {
            level: k + 1,
            mesh: lod_mesh,
            target_triangles,
            error,
            screen_size,
            method,
        });
    }
    chain
}

/// Everything the simplifier needs that doesn't change between levels.
struct Simplifier<'a> {
    mesh: &'a Mesh,
    attributes: Vec<f32>,
    weights: Vec<f32>,
    locks: Vec<SimplifyVertex>,
    options: SimplifyOptions,
    /// Relative-to-absolute error factor for the sloppy fallback.
    scale: f32,
}

impl<'a> Simplifier<'a> {
    fn new(mesh: &'a Mesh, settings: &LodSettings) -> Self {
        let mut weights = Vec::new();
        if mesh.normals.is_some() {
            weights.extend([settings.normal_weight; 3]);
        }
        if !mesh.uvs.is_empty() {
            weights.extend([settings.uv_weight; 2]);
        }
        let mut attributes = Vec::with_capacity(mesh.vertex_count() * weights.len());
        for i in 0..mesh.vertex_count() {
            if let Some(normals) = &mesh.normals {
                attributes.extend(normals[i].to_array());
            }
            if let Some(uv) = mesh.uvs.first() {
                attributes.extend(uv[i].to_array());
            }
        }

        let mut options = SimplifyOptions::ErrorAbsolute;
        if mesh.submeshes.len() > 1 {
            options |= SimplifyOptions::Sparse;
        }
        if settings.lock_border {
            options |= SimplifyOptions::LockBorder;
        }

        let adapter = positions_adapter(mesh);
        Self {
            mesh,
            attributes,
            weights,
            locks: shared_submesh_locks(mesh),
            options,
            scale: meshopt::simplify_scale(&adapter),
        }
    }

    /// Returns the simplified mesh (not yet compacted), its world error, and
    /// the method that produced it.
    fn simplify(&self, target_triangles: usize) -> (Mesh, f32, SimplifyMethod) {
        let mesh = self.mesh;
        let adapter = positions_adapter(mesh);
        let ratio = target_triangles as f32 / mesh.triangle_count() as f32;
        let mut out = mesh.clone();
        out.indices.clear();
        let mut max_error = 0.0f32;
        let mut method = SimplifyMethod::Standard;

        for (sub_out, sub) in out.submeshes.iter_mut().zip(&mesh.submeshes) {
            let range = sub.index_start as usize..(sub.index_start + sub.index_count) as usize;
            let indices = &mesh.indices[range];
            let sub_target = ((indices.len() / 3) as f32 * ratio).round().max(1.0) as usize;
            let stalled = |result: &[u32]| result.len() / 3 > sub_target + sub_target / 10 + 2;
            let (mut result, mut error) =
                self.attribute_aware(indices, &adapter, sub_target, self.options);
            let mut sub_method = SimplifyMethod::Standard;
            // Scans and heavily split meshes stall on their seams: let collapses cross them.
            if stalled(&result) {
                (result, error) = self.attribute_aware(
                    indices,
                    &adapter,
                    sub_target,
                    self.options | SimplifyOptions::Permissive,
                );
                sub_method = SimplifyMethod::Permissive;
            }
            // Thin parts and many locks can still stall: the sloppy simplifier
            // ignores topology but always reaches its target.
            if stalled(&result) {
                let mut relative = 0.0;
                result = meshopt::simplify_sloppy_with_locks(
                    indices,
                    &adapter,
                    &self.locks,
                    sub_target * 3,
                    1.0,
                    Some(&mut relative),
                );
                error = relative * self.scale;
                sub_method = SimplifyMethod::Sloppy;
            }
            method = method.max(sub_method);
            max_error = max_error.max(error);
            sub_out.index_start = out.indices.len() as u32;
            sub_out.index_count = result.len() as u32;
            out.indices.extend(result);
        }
        out.submeshes.retain(|s| s.index_count > 0);
        (out, max_error, method)
    }

    /// One attribute-aware simplification of a submesh's indices down to
    /// `target` triangles. Returns the indices and the world error.
    fn attribute_aware(
        &self,
        indices: &[u32],
        adapter: &VertexDataAdapter<'_>,
        target: usize,
        options: SimplifyOptions,
    ) -> (Vec<u32>, f32) {
        let mut error = 0.0;
        let result = meshopt::simplify_with_attributes_and_locks(
            indices,
            adapter,
            &self.attributes,
            &self.weights,
            self.weights.len() * size_of::<f32>(),
            &self.locks,
            target * 3,
            // Unbounded error: the triangle target decides when to stop.
            self.scale,
            options,
            Some(&mut error),
        );
        (result, error)
    }
}

fn positions_adapter(mesh: &Mesh) -> VertexDataAdapter<'_> {
    VertexDataAdapter::new(
        meshopt::typed_to_bytes(&mesh.positions),
        size_of::<Vec3>(),
        0,
    )
    .expect("positions are tightly packed Vec3s")
}

/// Locks every vertex whose position is used by more than one submesh.
fn shared_submesh_locks(mesh: &Mesh) -> Vec<SimplifyVertex> {
    let mut locks = vec![SimplifyVertex::None; mesh.vertex_count()];
    if mesh.submeshes.len() < 2 {
        return locks;
    }
    let remap = mesh.position_remap();
    // Last submesh that used each position, and whether a second one did.
    let mut owner = vec![usize::MAX; mesh.vertex_count()];
    let mut shared = vec![false; mesh.vertex_count()];
    for (s, sub) in mesh.submeshes.iter().enumerate() {
        let range = sub.index_start as usize..(sub.index_start + sub.index_count) as usize;
        for &i in &mesh.indices[range] {
            let p = remap[i as usize] as usize;
            if owner[p] != usize::MAX && owner[p] != s {
                shared[p] = true;
            }
            owner[p] = s;
        }
    }
    for (i, lock) in locks.iter_mut().enumerate() {
        if shared[remap[i] as usize] {
            *lock = SimplifyVertex::Lock;
        }
    }
    locks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Submesh;
    use crate::primitives;

    #[test]
    fn chain_hits_targets() {
        let mesh = primitives::uv_sphere(1.0, 96, 48);
        let chain = generate_chain(&mesh, &LodSettings::default());
        assert_eq!(chain.len(), 4);
        for lod in &chain[1..] {
            let tris = lod.mesh.triangle_count() as f32;
            let target = lod.target_triangles as f32;
            assert!(
                (tris - target).abs() <= target * 0.05 + 2.0,
                "level {}: {tris} vs {target}",
                lod.level
            );
            assert_eq!(lod.method, SimplifyMethod::Standard);
            lod.mesh.validate().unwrap();
        }
    }

    #[test]
    fn error_grows_and_screen_size_shrinks() {
        let mesh = primitives::torus(1.0, 0.3, 96, 48);
        let chain = generate_chain(&mesh, &LodSettings::default());
        for pair in chain.windows(2) {
            assert!(pair[1].error >= pair[0].error);
            assert!(pair[1].screen_size <= pair[0].screen_size);
        }
        assert!(chain.last().unwrap().error > 0.0);
    }

    #[test]
    fn lods_drop_unused_vertices() {
        let mesh = primitives::uv_sphere(1.0, 64, 32);
        let chain = generate_chain(&mesh, &LodSettings::default());
        let lod = chain.last().unwrap();
        assert!(lod.mesh.vertex_count() < mesh.vertex_count() / 4);
        let mut used = vec![false; lod.mesh.vertex_count()];
        for &i in &lod.mesh.indices {
            used[i as usize] = true;
        }
        assert!(used.iter().all(|&u| u));
    }

    #[test]
    fn stops_when_savings_are_too_small() {
        // A single quad can't lose a triangle without vanishing.
        let mesh = primitives::grid(1.0, 1.0, 1, 1);
        let chain = generate_chain(&mesh, &LodSettings::default());
        assert!(chain.len() <= 2);
    }

    #[test]
    fn material_borders_stay_closed() {
        // Two materials on one grid, sharing the vertices along their border.
        let mut mesh = primitives::grid(2.0, 2.0, 32, 32);
        let half = (mesh.indices.len() / 6 * 3) as u32;
        let total = mesh.indices.len() as u32;
        mesh.submeshes = vec![
            Submesh {
                index_start: 0,
                index_count: half,
                material: None,
            },
            Submesh {
                index_start: half,
                index_count: total - half,
                material: None,
            },
        ];
        let locks = shared_submesh_locks(&mesh);
        let locked: Vec<usize> = (0..locks.len())
            .filter(|&i| locks[i] == SimplifyVertex::Lock)
            .collect();
        assert!(!locked.is_empty());

        let chain = generate_chain(&mesh, &LodSettings::default());
        let lod = chain.last().unwrap();
        // Every locked border position must still be used by both submeshes.
        let uses = |s: usize| -> std::collections::HashSet<[u32; 3]> {
            let sub = lod.mesh.submeshes[s];
            lod.mesh.indices[sub.index_start as usize..(sub.index_start + sub.index_count) as usize]
                .iter()
                .map(|&i| lod.mesh.positions[i as usize].to_array().map(f32::to_bits))
                .collect()
        };
        let (a, b) = (uses(0), uses(1));
        for &i in &locked {
            let key = mesh.positions[i].to_array().map(f32::to_bits);
            assert!(
                a.contains(&key) && b.contains(&key),
                "border vertex {i} was moved"
            );
        }
    }

    #[test]
    fn screen_size_matches_hand_calculation() {
        // r = 1 m, e = 1 cm, 1 px at 1080p: 2 · 1 · 1 / (0.01 · 1080) ≈ 0.185.
        let s = switch_screen_size(1.0, 0.01, 1.0, 1080.0);
        assert!((s - 0.185_185).abs() < 1e-4);
        assert_eq!(switch_screen_size(1.0, 0.0, 1.0, 1080.0), 1.0);
        assert_eq!(
            switch_screen_size(1.0, 1e-6, 1.0, 1080.0),
            1.0,
            "capped at full screen"
        );
    }
}
