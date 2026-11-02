//! Hand-written declarations for shim/shim.cpp. Layouts must match the C structs.

use std::ffi::c_void;

#[repr(C)]
pub struct LmMeshInput {
    pub positions: *const f32,
    pub normals: *const f32,
    pub uvs: *const f32,
    pub indices: *const u32,
    pub face_materials: *const u32,
    pub vertex_count: u32,
    pub index_count: u32,
}

#[repr(C)]
pub struct LmUvInput {
    pub uvs: *const f32,
    pub indices: *const u32,
    pub face_materials: *const u32,
    pub vertex_count: u32,
    pub index_count: u32,
}

#[repr(C)]
pub struct LmChartOptions {
    pub max_chart_area: f32,
    pub max_boundary_length: f32,
    pub normal_deviation_weight: f32,
    pub roundness_weight: f32,
    pub straightness_weight: f32,
    pub normal_seam_weight: f32,
    pub texture_seam_weight: f32,
    pub max_cost: f32,
    pub max_iterations: u32,
    pub use_input_mesh_uvs: u32,
    pub fix_winding: u32,
}

#[repr(C)]
pub struct LmPackOptions {
    pub max_chart_size: u32,
    pub padding: u32,
    pub texels_per_unit: f32,
    pub resolution: u32,
    pub bilinear: u32,
    pub block_align: u32,
    pub brute_force: u32,
    pub rotate_charts_to_axis: u32,
    pub rotate_charts: u32,
    pub keep_chart_aspect: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct LmAtlasInfo {
    pub width: u32,
    pub height: u32,
    pub atlas_count: u32,
    pub chart_count: u32,
    pub mesh_count: u32,
    pub texels_per_unit: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct LmMeshInfo {
    pub vertex_count: u32,
    pub index_count: u32,
    pub chart_count: u32,
}

unsafe extern "C" {
    pub fn lm_xatlas_create() -> *mut c_void;
    pub fn lm_xatlas_destroy(atlas: *mut c_void);
    pub fn lm_xatlas_add_mesh(atlas: *mut c_void, input: *const LmMeshInput) -> i32;
    pub fn lm_xatlas_add_uv_mesh(atlas: *mut c_void, input: *const LmUvInput) -> i32;
    pub fn lm_xatlas_compute_charts(atlas: *mut c_void, options: *const LmChartOptions);
    pub fn lm_xatlas_pack_charts(atlas: *mut c_void, options: *const LmPackOptions);
    pub fn lm_xatlas_info(atlas: *const c_void, out: *mut LmAtlasInfo);
    pub fn lm_xatlas_mesh_info(atlas: *const c_void, mesh: u32, out: *mut LmMeshInfo) -> i32;
    pub fn lm_xatlas_mesh_copy(
        atlas: *const c_void,
        mesh: u32,
        xrefs: *mut u32,
        uvs: *mut f32,
        charts: *mut i32,
        atlases: *mut i32,
        indices: *mut u32,
    ) -> i32;
}
