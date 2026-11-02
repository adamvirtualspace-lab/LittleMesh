//! Safe wrapper over the vendored [xatlas](https://github.com/jpcy/xatlas)
//! (MIT, see vendor/xatlas/LICENSE): chart generation, parameterization, and
//! packing. LittleMesh uses it as the Auto UV baseline and as the packer for
//! its own charts (docs/algorithms.md §7.10).
//!
//! This is the only crate in the workspace that allows `unsafe`, and only for
//! the FFI calls below. Every input is validated before it crosses into C++.

#![allow(unsafe_code)]

mod ffi;

use std::ffi::c_void;
use std::fmt;
use std::ptr::{self, NonNull};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Rejected before reaching xatlas.
    InvalidInput(&'static str),
    /// An index is not smaller than the vertex count.
    IndexOutOfRange,
    /// The index count is not a multiple of 3.
    InvalidIndexCount,
    /// `add_mesh` and `add_uv_mesh` can't be mixed in one atlas.
    MixedInputs,
    /// Any other xatlas failure.
    Other,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidInput(why) => write!(f, "invalid xatlas input: {why}"),
            Error::IndexOutOfRange => f.write_str("xatlas: index out of range"),
            Error::InvalidIndexCount => f.write_str("xatlas: index count is not a multiple of 3"),
            Error::MixedInputs => f.write_str("xatlas: can't mix 3D meshes and UV meshes"),
            Error::Other => f.write_str("xatlas: failed to add mesh"),
        }
    }
}

impl std::error::Error for Error {}

/// A 3D triangle mesh to unwrap.
#[derive(Debug, Clone, Copy)]
pub struct MeshInput<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: Option<&'a [[f32; 3]]>,
    /// Hint for chart generation, used only with `ChartOptions::use_input_mesh_uvs`.
    pub uvs: Option<&'a [[f32; 2]]>,
    pub indices: &'a [u32],
    /// One per triangle. Charts never mix materials.
    pub face_materials: Option<&'a [u32]>,
}

/// An already-unwrapped mesh: each UV island becomes one chart, kept as is
/// and only moved, rotated, and uniformly scaled by the packer.
#[derive(Debug, Clone, Copy)]
pub struct UvMeshInput<'a> {
    pub uvs: &'a [[f32; 2]],
    pub indices: &'a [u32],
    /// One per triangle. Overlapping (stacked) islands need different materials.
    pub face_materials: Option<&'a [u32]>,
}

/// Mirrors `xatlas::ChartOptions`; the defaults are xatlas's.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartOptions {
    pub max_chart_area: f32,
    pub max_boundary_length: f32,
    pub normal_deviation_weight: f32,
    pub roundness_weight: f32,
    pub straightness_weight: f32,
    pub normal_seam_weight: f32,
    pub texture_seam_weight: f32,
    pub max_cost: f32,
    pub max_iterations: u32,
    pub use_input_mesh_uvs: bool,
    pub fix_winding: bool,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            max_chart_area: 0.0,
            max_boundary_length: 0.0,
            normal_deviation_weight: 2.0,
            roundness_weight: 0.01,
            straightness_weight: 6.0,
            normal_seam_weight: 4.0,
            texture_seam_weight: 0.5,
            max_cost: 2.0,
            max_iterations: 1,
            use_input_mesh_uvs: false,
            fix_winding: false,
        }
    }
}

/// Mirrors `xatlas::PackOptions`; the defaults are xatlas's.
#[derive(Debug, Clone, PartialEq)]
pub struct PackOptions {
    pub max_chart_size: u32,
    /// Texels of padding around each chart.
    pub padding: u32,
    /// 0 estimates it from `resolution`.
    pub texels_per_unit: f32,
    /// Target atlas size in texels. With `texels_per_unit` 0 this is a
    /// target, not a limit: the atlas grows rather than splitting.
    pub resolution: u32,
    pub bilinear: bool,
    pub block_align: bool,
    pub brute_force: bool,
    pub rotate_charts_to_axis: bool,
    pub rotate_charts: bool,
    /// Keep each chart's exact shape. Stock xatlas stretches every chart per
    /// axis to whole-texel extents (vendor/xatlas/PATCHES.md).
    pub keep_chart_aspect: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            max_chart_size: 0,
            padding: 0,
            texels_per_unit: 0.0,
            resolution: 0,
            bilinear: true,
            block_align: false,
            brute_force: false,
            rotate_charts_to_axis: true,
            rotate_charts: true,
            keep_chart_aspect: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AtlasInfo {
    /// Atlas size in texels; output UVs are in `[0, width] × [0, height]`.
    pub width: u32,
    pub height: u32,
    /// More than 1 only when packing to a fixed `texels_per_unit` and `resolution`.
    pub atlas_count: u32,
    pub chart_count: u32,
    pub mesh_count: u32,
    pub texels_per_unit: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// The input vertex this one was split from.
    pub xref: u32,
    /// In texels, not normalized.
    pub uv: [f32; 2],
    /// `None` for vertices of ignored faces.
    pub chart: Option<u32>,
    pub atlas: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputMesh {
    pub vertices: Vec<Vertex>,
    /// Same triangle order as the input.
    pub indices: Vec<u32>,
    pub chart_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    None,
    Mesh,
    Uv,
}

/// One xatlas atlas: add meshes, compute charts, pack, then read results.
pub struct Atlas {
    ptr: NonNull<c_void>,
    kind: InputKind,
}

// The atlas is owned and only touched through `&mut self` or `&self` reads
// after packing, so moving it between threads is fine.
unsafe impl Send for Atlas {}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

impl Atlas {
    pub fn new() -> Self {
        let ptr = unsafe { ffi::lm_xatlas_create() };
        Self {
            ptr: NonNull::new(ptr).expect("xatlas::Create returned null"),
            kind: InputKind::None,
        }
    }

    pub fn add_mesh(&mut self, mesh: &MeshInput) -> Result<(), Error> {
        self.set_kind(InputKind::Mesh)?;
        let vertex_count = check_counts(mesh.positions.len(), mesh.indices)?;
        if mesh
            .normals
            .is_some_and(|n| n.len() != mesh.positions.len())
        {
            return Err(Error::InvalidInput(
                "normal count differs from position count",
            ));
        }
        if mesh.uvs.is_some_and(|uv| uv.len() != mesh.positions.len()) {
            return Err(Error::InvalidInput("uv count differs from position count"));
        }
        check_face_materials(mesh.face_materials, mesh.indices)?;
        let input = ffi::LmMeshInput {
            positions: mesh.positions.as_ptr().cast(),
            normals: mesh.normals.map_or(ptr::null(), |n| n.as_ptr().cast()),
            uvs: mesh.uvs.map_or(ptr::null(), |uv| uv.as_ptr().cast()),
            indices: mesh.indices.as_ptr(),
            face_materials: mesh.face_materials.map_or(ptr::null(), <[u32]>::as_ptr),
            vertex_count,
            index_count: mesh.indices.len() as u32,
        };
        // xatlas copies the data before returning.
        add_result(unsafe { ffi::lm_xatlas_add_mesh(self.ptr.as_ptr(), &input) })
    }

    pub fn add_uv_mesh(&mut self, mesh: &UvMeshInput) -> Result<(), Error> {
        self.set_kind(InputKind::Uv)?;
        let vertex_count = check_counts(mesh.uvs.len(), mesh.indices)?;
        check_face_materials(mesh.face_materials, mesh.indices)?;
        let input = ffi::LmUvInput {
            uvs: mesh.uvs.as_ptr().cast(),
            indices: mesh.indices.as_ptr(),
            face_materials: mesh.face_materials.map_or(ptr::null(), <[u32]>::as_ptr),
            vertex_count,
            index_count: mesh.indices.len() as u32,
        };
        add_result(unsafe { ffi::lm_xatlas_add_uv_mesh(self.ptr.as_ptr(), &input) })
    }

    pub fn compute_charts(&mut self, options: &ChartOptions) {
        let o = ffi::LmChartOptions {
            max_chart_area: options.max_chart_area,
            max_boundary_length: options.max_boundary_length,
            normal_deviation_weight: options.normal_deviation_weight,
            roundness_weight: options.roundness_weight,
            straightness_weight: options.straightness_weight,
            normal_seam_weight: options.normal_seam_weight,
            texture_seam_weight: options.texture_seam_weight,
            max_cost: options.max_cost,
            max_iterations: options.max_iterations,
            use_input_mesh_uvs: options.use_input_mesh_uvs.into(),
            fix_winding: options.fix_winding.into(),
        };
        unsafe { ffi::lm_xatlas_compute_charts(self.ptr.as_ptr(), &o) }
    }

    pub fn pack_charts(&mut self, options: &PackOptions) {
        let o = ffi::LmPackOptions {
            max_chart_size: options.max_chart_size,
            padding: options.padding,
            texels_per_unit: options.texels_per_unit,
            resolution: options.resolution,
            bilinear: options.bilinear.into(),
            block_align: options.block_align.into(),
            brute_force: options.brute_force.into(),
            rotate_charts_to_axis: options.rotate_charts_to_axis.into(),
            rotate_charts: options.rotate_charts.into(),
            keep_chart_aspect: options.keep_chart_aspect.into(),
        };
        unsafe { ffi::lm_xatlas_pack_charts(self.ptr.as_ptr(), &o) }
    }

    /// `compute_charts` followed by `pack_charts`.
    pub fn generate(&mut self, charts: &ChartOptions, pack: &PackOptions) {
        self.compute_charts(charts);
        self.pack_charts(pack);
    }

    pub fn info(&self) -> AtlasInfo {
        let mut out = ffi::LmAtlasInfo::default();
        unsafe { ffi::lm_xatlas_info(self.ptr.as_ptr(), &mut out) };
        AtlasInfo {
            width: out.width,
            height: out.height,
            atlas_count: out.atlas_count,
            chart_count: out.chart_count,
            mesh_count: out.mesh_count,
            texels_per_unit: out.texels_per_unit,
        }
    }

    /// Output mesh `index`, in the order meshes were added. `None` before packing.
    pub fn mesh(&self, index: u32) -> Option<OutputMesh> {
        let mut info = ffi::LmMeshInfo::default();
        if unsafe { ffi::lm_xatlas_mesh_info(self.ptr.as_ptr(), index, &mut info) } != 0 {
            return None;
        }
        let n = info.vertex_count as usize;
        let mut xrefs = vec![0u32; n];
        let mut uvs = vec![0f32; n * 2];
        let mut charts = vec![0i32; n];
        let mut atlases = vec![0i32; n];
        let mut indices = vec![0u32; info.index_count as usize];
        // The buffers are sized from the same mesh's info above.
        let status = unsafe {
            ffi::lm_xatlas_mesh_copy(
                self.ptr.as_ptr(),
                index,
                xrefs.as_mut_ptr(),
                uvs.as_mut_ptr(),
                charts.as_mut_ptr(),
                atlases.as_mut_ptr(),
                indices.as_mut_ptr(),
            )
        };
        if status != 0 {
            return None;
        }
        let vertices = (0..n)
            .map(|i| Vertex {
                xref: xrefs[i],
                uv: [uvs[i * 2], uvs[i * 2 + 1]],
                chart: u32::try_from(charts[i]).ok(),
                atlas: u32::try_from(atlases[i]).ok(),
            })
            .collect();
        Some(OutputMesh {
            vertices,
            indices,
            chart_count: info.chart_count,
        })
    }

    fn set_kind(&mut self, kind: InputKind) -> Result<(), Error> {
        match self.kind {
            InputKind::None => {
                self.kind = kind;
                Ok(())
            }
            k if k == kind => Ok(()),
            _ => Err(Error::MixedInputs),
        }
    }
}

impl Drop for Atlas {
    fn drop(&mut self) {
        unsafe { ffi::lm_xatlas_destroy(self.ptr.as_ptr()) }
    }
}

fn check_counts(vertex_count: usize, indices: &[u32]) -> Result<u32, Error> {
    let vertex_count =
        u32::try_from(vertex_count).map_err(|_| Error::InvalidInput("too many vertices"))?;
    if u32::try_from(indices.len()).is_err() {
        return Err(Error::InvalidInput("too many indices"));
    }
    if indices.is_empty() {
        return Err(Error::InvalidInput("no triangles"));
    }
    if !indices.len().is_multiple_of(3) {
        return Err(Error::InvalidIndexCount);
    }
    if indices.iter().any(|&i| i >= vertex_count) {
        return Err(Error::IndexOutOfRange);
    }
    Ok(vertex_count)
}

fn check_face_materials(materials: Option<&[u32]>, indices: &[u32]) -> Result<(), Error> {
    match materials {
        Some(m) if m.len() != indices.len() / 3 => Err(Error::InvalidInput(
            "face material count differs from triangle count",
        )),
        _ => Ok(()),
    }
}

fn add_result(code: i32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        2 => Err(Error::IndexOutOfRange),
        4 => Err(Error::InvalidIndexCount),
        _ => Err(Error::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube with split vertices per face.
    fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for axis in 0..3 {
            for sign in [-0.5f32, 0.5] {
                let base = positions.len() as u32;
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                for (a, b) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                    let mut p = [0.0; 3];
                    p[axis] = sign;
                    p[u] = a;
                    p[v] = b;
                    positions.push(p);
                }
                let quad = if sign > 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                indices.extend(quad.map(|i| base + i));
            }
        }
        (positions, indices)
    }

    #[test]
    fn generates_charts_for_a_cube() {
        let (positions, indices) = cube();
        let mut atlas = Atlas::new();
        atlas
            .add_mesh(&MeshInput {
                positions: &positions,
                normals: None,
                uvs: None,
                indices: &indices,
                face_materials: None,
            })
            .unwrap();
        atlas.generate(
            &ChartOptions::default(),
            &PackOptions {
                resolution: 256,
                padding: 2,
                ..Default::default()
            },
        );
        let info = atlas.info();
        assert_eq!(info.mesh_count, 1);
        assert!(info.width > 0 && info.height > 0);
        let out = atlas.mesh(0).unwrap();
        assert_eq!(out.indices.len(), indices.len());
        assert!(out.chart_count >= 1);
        for v in &out.vertices {
            assert!((v.xref as usize) < positions.len());
            assert!(v.chart.is_some());
            assert!(v.uv[0] >= 0.0 && v.uv[0] <= info.width as f32);
            assert!(v.uv[1] >= 0.0 && v.uv[1] <= info.height as f32);
        }
        // Every output triangle still refers to the same input positions.
        for (o, i) in out.indices.iter().zip(&indices) {
            let a = positions[out.vertices[*o as usize].xref as usize];
            assert_eq!(a, positions[*i as usize]);
        }
    }

    /// The UV-mesh path must keep each island's shape: only a rigid motion
    /// (with 90° rotations) and one uniform scale per island are allowed.
    #[test]
    fn packing_keeps_hand_made_charts_intact() {
        // Island A: a 2×1 rectangle split into two triangles.
        // Island B: a right triangle with legs 1 and 3, far away.
        let uvs = vec![
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [0.0, 1.0],
            [10.0, 10.0],
            [11.0, 10.0],
            [10.0, 13.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6];
        let mut atlas = Atlas::new();
        atlas
            .add_uv_mesh(&UvMeshInput {
                uvs: &uvs,
                indices: &indices,
                face_materials: None,
            })
            .unwrap();
        atlas.generate(
            &ChartOptions::default(),
            &PackOptions {
                resolution: 512,
                padding: 4,
                keep_chart_aspect: true,
                ..Default::default()
            },
        );
        let out = atlas.mesh(0).unwrap();
        assert_eq!(out.chart_count, 2);
        let uv_of = |corner: usize| out.vertices[out.indices[corner] as usize].uv;
        let dist =
            |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();

        for tri in 0..indices.len() / 3 {
            let c = tri * 3;
            let before = [
                uvs[indices[c] as usize],
                uvs[indices[c + 1] as usize],
                uvs[indices[c + 2] as usize],
            ];
            let after = [uv_of(c), uv_of(c + 1), uv_of(c + 2)];
            // Same edge-length ratios means same shape up to similarity.
            let lb = [
                dist(before[0], before[1]),
                dist(before[1], before[2]),
                dist(before[2], before[0]),
            ];
            let la = [
                dist(after[0], after[1]),
                dist(after[1], after[2]),
                dist(after[2], after[0]),
            ];
            let scale = la[0] / lb[0];
            assert!(scale > 0.0);
            for k in 1..3 {
                assert!(
                    (la[k] / lb[k] - scale).abs() < 1e-3 * scale,
                    "triangle {tri} was reshaped"
                );
            }
        }
        // Both islands keep one shared scale, so relative texel density holds.
        let s_a = dist(uv_of(0), uv_of(1)) / 2.0;
        let s_b = dist(uv_of(6), uv_of(7)) / 1.0;
        assert!((s_a - s_b).abs() < 1e-3 * s_a, "{s_a} vs {s_b}");
    }

    #[test]
    fn rejects_bad_input_before_ffi() {
        let mut atlas = Atlas::new();
        let positions = [[0.0f32; 3]; 3];
        let bad = MeshInput {
            positions: &positions,
            normals: None,
            uvs: None,
            indices: &[0, 1, 3],
            face_materials: None,
        };
        assert_eq!(atlas.add_mesh(&bad), Err(Error::IndexOutOfRange));
        let uneven = MeshInput {
            indices: &[0, 1],
            ..bad
        };
        assert_eq!(atlas.add_mesh(&uneven), Err(Error::InvalidIndexCount));
        let uv = UvMeshInput {
            uvs: &[[0.0; 2]; 3],
            indices: &[0, 1, 2],
            face_materials: None,
        };
        assert_eq!(atlas.add_uv_mesh(&uv), Err(Error::MixedInputs));
    }

    #[test]
    fn mesh_is_none_before_packing() {
        assert!(Atlas::new().mesh(0).is_none());
    }
}
