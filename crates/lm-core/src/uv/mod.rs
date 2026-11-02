//! Auto UVs (docs/algorithms.md §7).
//!
//! v0.1 starts with the xatlas baseline (§7.12): xatlas's own chart
//! generation and packer, one 0–1 sheet per material. The shape-aware
//! strategies arrive on top of it and are measured against it.

pub mod debug;
pub mod metrics;

use std::time::Instant;

use glam::Vec2;
use serde::Serialize;

pub use metrics::{UvMetrics, uv_metrics};

use crate::{Error, Mesh, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct UvSettings {
    /// Target texture size in texels. Sets the default padding and the
    /// resolution overlaps are checked at.
    pub resolution: u32,
    /// Gap around each island in texels; `None` uses `resolution / 256`
    /// (§7.10), which keeps mips clean down to 256 px.
    pub padding: Option<u32>,
}

impl Default for UvSettings {
    fn default() -> Self {
        Self {
            resolution: 1024,
            padding: None,
        }
    }
}

impl UvSettings {
    pub fn padding(&self) -> u32 {
        self.padding.unwrap_or((self.resolution / 256).max(1))
    }
}

/// What kind of part a triangle was unwrapped as (§7.5). The shape classes
/// arrive with segmentation and the strategies; until then every triangle is
/// `Xatlas` after Auto UVs, or `Input` when the UVs came with the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UvClass {
    Input,
    Xatlas,
    Repeated,
    Planar,
    Box,
    QuadGrid,
    Cylinder,
    Cone,
    BentTube,
    Disk,
    Developable,
    Organic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UvMethod {
    /// xatlas's default unwrap and pack.
    Xatlas,
}

#[derive(Debug, Clone, Serialize)]
pub struct UvReport {
    pub method: UvMethod,
    pub charts: usize,
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub elapsed_ms: f64,
    pub metrics: UvMetrics,
    /// Class of each triangle, in the mesh's order right after unwrapping.
    #[serde(skip)]
    pub classes: Vec<UvClass>,
}

/// Replaces UV0 with a fresh unwrap. Other UV channels are kept; tangents
/// are dropped because they depend on UV0. The mesh must pass
/// [`Mesh::validate`], so its submeshes cover every triangle.
pub fn auto_uv(mesh: &mut Mesh, settings: &UvSettings) -> Result<UvReport> {
    let vertices_before = mesh.vertex_count();
    let start = Instant::now();
    let charts = xatlas_baseline(mesh, settings)?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let metrics = uv_metrics(mesh, 0, settings.resolution).expect("UV0 was just written");
    Ok(UvReport {
        method: UvMethod::Xatlas,
        charts,
        vertices_before,
        vertices_after: mesh.vertex_count(),
        elapsed_ms,
        metrics,
        classes: vec![UvClass::Xatlas; mesh.triangle_count()],
    })
}

/// Total surface area each sheet is scaled to before it goes to xatlas.
const XATLAS_SHEET_AREA: f32 = 1.0e4;

/// Unwraps and packs each material's triangles into their own 0–1 sheet.
/// Returns the chart count.
fn xatlas_baseline(mesh: &mut Mesh, settings: &UvSettings) -> Result<usize> {
    let tri_count = mesh.triangle_count();
    // Triangles per material, in submesh order.
    let mut sheets: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
    for sub in &mesh.submeshes {
        let first = sub.index_start as usize / 3;
        let tris = first..first + sub.index_count as usize / 3;
        match sheets.iter_mut().find(|(m, _)| *m == sub.material) {
            Some((_, list)) => list.extend(tris),
            None => sheets.push((sub.material, tris.collect())),
        }
    }

    let pack = lm_xatlas::PackOptions {
        resolution: settings.resolution,
        padding: settings.padding(),
        ..Default::default()
    };
    // Each output vertex copies the attributes of `sources[i]`.
    let mut sources: Vec<u32> = Vec::new();
    let mut new_uvs: Vec<Vec2> = Vec::new();
    let mut indices = vec![0u32; tri_count * 3];
    let mut charts = 0;
    const UNSEEN: u32 = u32::MAX;
    let mut local = vec![UNSEEN; mesh.vertex_count()];
    for (_, tris) in &sheets {
        if tris.is_empty() {
            continue;
        }
        // Compact this sheet's vertices so xatlas only sees what it uses.
        local.fill(UNSEEN);
        let mut originals: Vec<u32> = Vec::new();
        let mut sheet_indices = Vec::with_capacity(tris.len() * 3);
        for &t in tris {
            for &v in &mesh.indices[t * 3..t * 3 + 3] {
                let slot = &mut local[v as usize];
                if *slot == UNSEEN {
                    *slot = originals.len() as u32;
                    originals.push(v);
                }
                sheet_indices.push(*slot);
            }
        }
        // xatlas drops triangles with an absolute area under FLT_EPSILON, which
        // on a small prop is every fine triangle. A uniform scale changes no UV
        // shapes, so give each sheet a fixed total area first.
        let area: f32 = sheet_indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| mesh.positions[originals[v as usize] as usize]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum();
        let scale = if area > 0.0 {
            (XATLAS_SHEET_AREA / area).sqrt()
        } else {
            1.0
        };
        let positions: Vec<[f32; 3]> = originals
            .iter()
            .map(|&v| (mesh.positions[v as usize] * scale).to_array())
            .collect();
        let normals: Option<Vec<[f32; 3]>> = mesh.normals.as_ref().map(|n| {
            originals
                .iter()
                .map(|&v| n[v as usize].to_array())
                .collect()
        });

        let mut atlas = lm_xatlas::Atlas::new();
        atlas
            .add_mesh(&lm_xatlas::MeshInput {
                positions: &positions,
                normals: normals.as_deref(),
                uvs: None,
                indices: &sheet_indices,
                face_materials: None,
            })
            .map_err(|e| Error::Other(format!("{}: {e}", mesh.name)))?;
        atlas.generate(&lm_xatlas::ChartOptions::default(), &pack);
        let info = atlas.info();
        let out = atlas
            .mesh(0)
            .ok_or_else(|| Error::Other(format!("{}: xatlas produced no output", mesh.name)))?;
        charts += out.chart_count as usize;

        // Normalize by the longer side so islands keep their aspect ratio.
        let scale = 1.0 / info.width.max(info.height).max(1) as f32;
        let base = sources.len() as u32;
        for v in &out.vertices {
            sources.push(originals[v.xref as usize]);
            new_uvs.push(Vec2::new(v.uv[0], v.uv[1]) * scale);
        }
        for (k, &t) in tris.iter().enumerate() {
            for c in 0..3 {
                indices[t * 3 + c] = base + out.indices[k * 3 + c];
            }
        }
    }

    mesh.tangents = None;
    mesh.gather_vertices(&sources);
    match mesh.uvs.first_mut() {
        Some(uv0) => *uv0 = new_uvs,
        None => mesh.uvs.push(new_uvs),
    }
    mesh.indices = indices;
    Ok(charts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::{Submesh, optimize};

    fn check_clean(report: &UvReport) {
        let m = &report.metrics;
        assert_eq!(m.flipped_triangles, 0, "{m:?}");
        assert_eq!(m.zero_area_triangles, 0, "{m:?}");
        assert_eq!(m.overlap_texels, 0, "{m:?}");
        assert_eq!(m.out_of_bounds_triangles, 0, "{m:?}");
    }

    #[test]
    fn unwraps_shapes_without_flips_or_overlaps() {
        for mut mesh in [
            primitives::cube(1.0),
            primitives::uv_sphere(1.0, 32, 16),
            primitives::torus(1.0, 0.3, 48, 16),
            primitives::cylinder(0.5, 2.0, 24, 4),
        ] {
            let name = mesh.name.clone();
            let triangles = mesh.triangle_count();
            let report = auto_uv(&mut mesh, &UvSettings::default()).unwrap();
            mesh.validate().unwrap();
            assert_eq!(mesh.triangle_count(), triangles, "{name}");
            assert!(mesh.tangents.is_none());
            check_clean(&report);
            assert!(report.charts >= 1, "{name}");
            assert!(
                report.metrics.angle_distortion_mean < 1.5,
                "{name}: {:?}",
                report.metrics
            );
        }
    }

    #[test]
    fn keeps_positions_per_corner_and_other_channels() {
        let mut mesh = primitives::torus(1.0, 0.3, 24, 12);
        mesh.uvs.push(mesh.uvs[0].clone());
        let before: Vec<_> = mesh
            .indices
            .iter()
            .map(|&i| mesh.positions[i as usize])
            .collect();
        let lightmap_before: Vec<_> = mesh
            .indices
            .iter()
            .map(|&i| mesh.uvs[1][i as usize])
            .collect();
        auto_uv(&mut mesh, &UvSettings::default()).unwrap();
        let after: Vec<_> = mesh
            .indices
            .iter()
            .map(|&i| mesh.positions[i as usize])
            .collect();
        let lightmap_after: Vec<_> = mesh
            .indices
            .iter()
            .map(|&i| mesh.uvs[1][i as usize])
            .collect();
        assert_eq!(before, after);
        assert_eq!(lightmap_before, lightmap_after);
        assert_eq!(mesh.uvs.len(), 2);
    }

    #[test]
    fn meshes_without_uvs_get_uv0() {
        let mut mesh = primitives::uv_sphere(1.0, 16, 8);
        mesh.uvs.clear();
        optimize::cleanup(&mut mesh);
        let report = auto_uv(&mut mesh, &UvSettings::default()).unwrap();
        assert_eq!(mesh.uvs.len(), 1);
        check_clean(&report);
    }

    #[test]
    fn each_material_gets_its_own_sheet() {
        let mut mesh = primitives::cube(1.0);
        // First three faces on material 0, the rest on material 1.
        mesh.submeshes = vec![
            Submesh {
                index_start: 0,
                index_count: 18,
                material: Some(0),
            },
            Submesh {
                index_start: 18,
                index_count: 18,
                material: Some(1),
            },
        ];
        let report = auto_uv(&mut mesh, &UvSettings::default()).unwrap();
        assert_eq!(report.metrics.sheets, 2);
        check_clean(&report);
        // Both sheets start near the origin, so their UV bounds overlap.
        let bounds = |range: std::ops::Range<usize>| {
            mesh.indices[range]
                .iter()
                .map(|&i| mesh.uvs[0][i as usize])
                .fold(Vec2::splat(f32::MAX), Vec2::min)
        };
        assert!(bounds(0..18).max_element() < 0.2);
        assert!(bounds(18..36).max_element() < 0.2);
    }

    #[test]
    fn padding_follows_resolution() {
        assert_eq!(UvSettings::default().padding(), 4);
        let s = UvSettings {
            resolution: 4096,
            padding: None,
        };
        assert_eq!(s.padding(), 16);
        let s = UvSettings {
            resolution: 128,
            padding: None,
        };
        assert_eq!(s.padding(), 1);
    }
}
