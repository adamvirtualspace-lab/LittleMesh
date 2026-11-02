//! The internal mesh representation (see docs/algorithms.md §2).
//!
//! Meshes are indexed triangle lists whose vertices are split at attribute
//! seams: a corner with two UVs or two normals is two vertices. Topology work
//! goes through [`Mesh::position_remap`] or the half-edge view, which see past
//! those splits.

use std::collections::HashMap;

use glam::{Vec2, Vec3, Vec4};

use crate::{Error, Result};

/// Everything loaded from or written to one file.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
}

/// An indexed triangle mesh with split vertices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub positions: Vec<Vec3>,
    pub normals: Option<Vec<Vec3>>,
    /// xyz = tangent direction, w = bitangent sign (glTF convention).
    pub tangents: Option<Vec<Vec4>>,
    /// UV channels; `uvs[0]` is UV0, `uvs[1]` the lightmap channel.
    pub uvs: Vec<Vec<Vec2>>,
    pub colors: Option<Vec<Vec4>>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<Submesh>,
}

/// A run of triangles drawn with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submesh {
    /// First index (not triangle) of the run; a multiple of 3.
    pub index_start: u32,
    /// Number of indices in the run; a multiple of 3.
    pub index_count: u32,
    /// Index into [`Scene::materials`], or `None` for the default material.
    pub material: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

/// A metallic-roughness PBR material, mirroring glTF's core material model.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    pub base_color: Vec4,
    pub base_color_texture: Option<TextureRef>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub occlusion_texture: Option<TextureRef>,
    pub emissive: Vec3,
    pub emissive_texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: Vec4::ONE,
            base_color_texture: None,
            metallic: 1.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive: Vec3::ZERO,
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// A material's reference to an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureRef {
    /// Index into [`Scene::images`].
    pub image: usize,
    /// Which UV channel the texture reads.
    pub uv_channel: u32,
}

/// An encoded image (PNG, JPEG, ...) carried through untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Rebuilds every vertex attribute so new vertex `i` copies old vertex
    /// `sources[i]`. Indices are left for the caller to rewrite.
    pub(crate) fn gather_vertices(&mut self, sources: &[u32]) {
        fn gather<T: Copy>(data: &[T], sources: &[u32]) -> Vec<T> {
            sources.iter().map(|&i| data[i as usize]).collect()
        }
        self.positions = gather(&self.positions, sources);
        self.normals = self.normals.as_deref().map(|n| gather(n, sources));
        self.tangents = self.tangents.as_deref().map(|t| gather(t, sources));
        self.colors = self.colors.as_deref().map(|c| gather(c, sources));
        for uvs in &mut self.uvs {
            *uvs = gather(uvs, sources);
        }
    }

    /// Axis-aligned bounds as `(min, max)`, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p))),
        )
    }

    /// Maps every vertex to the first vertex with a bit-identical position.
    ///
    /// Two split copies of one corner map to the same index, so topology
    /// (adjacency, borders, seams) can be computed on the remapped indices.
    pub fn position_remap(&self) -> Vec<u32> {
        let mut first_seen: HashMap<[u32; 3], u32> = HashMap::with_capacity(self.positions.len());
        self.positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // Normalize -0.0 to 0.0 so it welds with +0.0.
                let key = [p.x, p.y, p.z].map(|c| (c + 0.0).to_bits());
                *first_seen.entry(key).or_insert(i as u32)
            })
            .collect()
    }

    /// Number of distinct positions, i.e. vertices after a position weld.
    pub fn unique_position_count(&self) -> usize {
        let remap = self.position_remap();
        remap
            .iter()
            .enumerate()
            .filter(|&(i, &r)| i as u32 == r)
            .count()
    }

    /// Generates smooth, area-weighted vertex normals.
    ///
    /// Normals are averaged across split vertices that share a position, so
    /// UV seams don't show up as shading seams.
    pub fn generate_normals(&mut self) {
        let remap = self.position_remap();
        let mut accum = vec![Vec3::ZERO; self.positions.len()];
        for tri in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
            // The cross product's length is twice the area: area weighting for free.
            let n = (self.positions[b] - self.positions[a])
                .cross(self.positions[c] - self.positions[a]);
            for i in [a, b, c] {
                accum[remap[i] as usize] += n;
            }
        }
        let normals = remap
            .iter()
            .map(|&r| accum[r as usize].try_normalize().unwrap_or(Vec3::Y))
            .collect();
        self.normals = Some(normals);
    }

    /// Checks index bounds, attribute lengths, and submesh ranges.
    pub fn validate(&self) -> Result<()> {
        let fail = |reason: String| {
            Err(Error::InvalidMesh {
                mesh: self.name.clone(),
                reason,
            })
        };
        let n = self.positions.len();
        if !self.indices.len().is_multiple_of(3) {
            return fail(format!(
                "index count {} is not a multiple of 3",
                self.indices.len()
            ));
        }
        if let Some(&bad) = self.indices.iter().find(|&&i| i as usize >= n) {
            return fail(format!("index {bad} is out of range for {n} vertices"));
        }
        if self.positions.iter().any(|p| !p.is_finite()) {
            return fail("non-finite vertex position".into());
        }
        let lengths = [
            ("normals", self.normals.as_ref().map(Vec::len)),
            ("tangents", self.tangents.as_ref().map(Vec::len)),
            ("colors", self.colors.as_ref().map(Vec::len)),
        ];
        for (attr, len) in lengths {
            if let Some(len) = len.filter(|&len| len != n) {
                return fail(format!("{attr} has {len} entries, expected {n}"));
            }
        }
        for (channel, uv) in self.uvs.iter().enumerate() {
            if uv.len() != n {
                return fail(format!(
                    "UV{channel} has {} entries, expected {n}",
                    uv.len()
                ));
            }
        }
        let mut covered = 0u32;
        for (i, sub) in self.submeshes.iter().enumerate() {
            if sub.index_start % 3 != 0 || sub.index_count % 3 != 0 {
                return fail(format!("submesh {i} is not aligned to whole triangles"));
            }
            if sub.index_start != covered {
                return fail(format!(
                    "submesh {i} starts at {}, expected {covered}",
                    sub.index_start
                ));
            }
            covered += sub.index_count;
        }
        if covered as usize != self.indices.len() {
            return fail(format!(
                "submeshes cover {covered} indices, mesh has {}",
                self.indices.len()
            ));
        }
        Ok(())
    }
}

impl Scene {
    /// Validates every mesh plus the material and image references.
    pub fn validate(&self) -> Result<()> {
        for mesh in &self.meshes {
            mesh.validate()?;
            for sub in &mesh.submeshes {
                if let Some(m) = sub.material.filter(|&m| m >= self.materials.len()) {
                    return Err(Error::InvalidMesh {
                        mesh: mesh.name.clone(),
                        reason: format!("material {m} does not exist"),
                    });
                }
            }
        }
        for material in &self.materials {
            let textures = [
                material.base_color_texture,
                material.metallic_roughness_texture,
                material.normal_texture,
                material.occlusion_texture,
                material.emissive_texture,
            ];
            if let Some(t) = textures
                .into_iter()
                .flatten()
                .find(|t| t.image >= self.images.len())
            {
                return Err(Error::Other(format!(
                    "material '{}' references missing image {}",
                    material.name, t.image
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn primitives_are_valid() {
        for mesh in [
            primitives::cube(1.0),
            primitives::cylinder(0.5, 2.0, 16, 4),
            primitives::torus(1.0, 0.3, 24, 12),
            primitives::uv_sphere(1.0, 16, 8),
            primitives::grid(2.0, 2.0, 4, 4),
        ] {
            mesh.validate().unwrap();
        }
    }

    #[test]
    fn rejects_out_of_range_index() {
        let mut mesh = primitives::cube(1.0);
        mesh.indices[5] = 999;
        assert!(mesh.validate().is_err());
    }

    #[test]
    fn rejects_partial_triangle() {
        let mut mesh = primitives::cube(1.0);
        mesh.indices.pop();
        assert!(mesh.validate().is_err());
    }

    #[test]
    fn rejects_attribute_length_mismatch() {
        let mut mesh = primitives::cube(1.0);
        mesh.uvs[0].pop();
        assert!(mesh.validate().is_err());

        let mut mesh = primitives::cube(1.0);
        mesh.normals.as_mut().unwrap().push(Vec3::Y);
        assert!(mesh.validate().is_err());
    }

    #[test]
    fn rejects_bad_submeshes() {
        let mut mesh = primitives::cube(1.0);
        mesh.submeshes[0].index_count -= 3;
        assert!(mesh.validate().is_err(), "uncovered triangles");

        let mut mesh = primitives::cube(1.0);
        mesh.submeshes[0].index_start = 1;
        assert!(mesh.validate().is_err(), "misaligned start");
    }

    #[test]
    fn rejects_non_finite_position() {
        let mut mesh = primitives::cube(1.0);
        mesh.positions[0].x = f32::NAN;
        assert!(mesh.validate().is_err());
    }

    #[test]
    fn rejects_missing_material() {
        let scene = Scene {
            meshes: vec![primitives::cube(1.0)],
            ..Default::default()
        };
        let mut scene_bad = scene.clone();
        scene_bad.meshes[0].submeshes[0].material = Some(0);
        scene.validate().unwrap();
        assert!(scene_bad.validate().is_err());
    }

    #[test]
    fn position_remap_sees_through_splits() {
        // The cube has 24 split vertices (4 per face) but only 8 corners.
        let cube = primitives::cube(1.0);
        assert_eq!(cube.vertex_count(), 24);
        assert_eq!(cube.unique_position_count(), 8);
    }

    #[test]
    fn generated_normals_are_unit_and_smooth_across_splits() {
        let mut cube = primitives::cube(1.0);
        cube.generate_normals();
        let remap = cube.position_remap();
        let normals = cube.normals.unwrap();
        for (i, n) in normals.iter().enumerate() {
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!(n.abs_diff_eq(normals[remap[i] as usize], 1e-6));
        }
    }
}
