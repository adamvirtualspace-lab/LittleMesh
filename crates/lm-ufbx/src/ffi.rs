//! Hand-written declarations for shim/shim.c. Layouts must match the C structs.

use std::ffi::{c_char, c_void};

#[repr(C)]
pub struct LmStr {
    pub data: *const c_char,
    pub len: usize,
}

#[repr(C)]
pub struct LmFbxPart {
    pub index_start: u32,
    pub index_count: u32,
    pub material: i32,
}

#[repr(C)]
pub struct LmFbxMesh {
    pub name: LmStr,
    pub vertex_count: u32,
    pub index_count: u32,
    pub uv_set_count: u32,
    pub has_colors: u32,
    pub part_count: u32,
    pub positions: *const f32,
    pub normals: *const f32,
    pub uvs: *const f32,
    pub colors: *const f32,
    pub indices: *const u32,
    pub parts: *const LmFbxPart,
}

#[repr(C)]
pub struct LmFbxMaterial {
    pub name: LmStr,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub base_color_texture: i32,
    pub normal_texture: i32,
    pub occlusion_texture: i32,
    pub emissive_texture: i32,
    pub double_sided: u32,
}

#[repr(C)]
pub struct LmFbxTexture {
    pub filename: LmStr,
    pub relative_filename: LmStr,
    pub absolute_filename: LmStr,
    pub content: *const u8,
    pub content_size: usize,
}

#[repr(C)]
pub struct LmFbxScene {
    pub scene: *mut c_void,
    pub meshes: *const LmFbxMesh,
    pub mesh_count: usize,
    pub materials: *const LmFbxMaterial,
    pub material_count: usize,
    pub textures: *const LmFbxTexture,
    pub texture_count: usize,
    pub skipped_faces: usize,
}

unsafe extern "C" {
    pub fn lm_fbx_load(
        path: *const c_char,
        len: usize,
        error: *mut c_char,
        error_size: usize,
    ) -> *mut LmFbxScene;
    pub fn lm_fbx_free(scene: *mut LmFbxScene);
}
