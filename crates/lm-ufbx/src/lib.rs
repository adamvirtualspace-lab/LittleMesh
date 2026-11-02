//! Safe wrapper over the vendored [ufbx](https://github.com/ufbx/ufbx)
//! (MIT or public domain, see vendor/ufbx/LICENSE): loads binary and ASCII
//! FBX files of any version. shim/shim.c flattens the scene, so this crate
//! only copies plain arrays out of C.
//!
//! Like lm-xatlas, this crate allows `unsafe` only for the FFI calls below.

#![allow(unsafe_code)]

mod ffi;

use std::ffi::{CStr, c_char};
use std::fmt;
use std::path::Path;
use std::slice;

/// A flattened FBX scene: Y up, metres, right-handed, as glTF expects.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    /// Point and line faces that were dropped.
    pub skipped_faces: usize,
}

/// One node instance of a mesh, triangulated and in world space.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// FBX convention: V points up from the bottom of the image.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
    pub parts: Vec<Part>,
}

/// A contiguous range of [`Mesh::indices`] with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub index_start: u32,
    pub index_count: u32,
    pub material: Option<usize>,
}

/// Material values as ufbx maps them to PBR; textures index [`Scene::textures`].
#[derive(Debug, Clone, Default)]
pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub base_color_texture: Option<usize>,
    pub normal_texture: Option<usize>,
    pub occlusion_texture: Option<usize>,
    pub emissive_texture: Option<usize>,
    pub double_sided: bool,
}

/// A unique texture file. `content` holds embedded bytes; otherwise the
/// caller reads the file from one of the paths.
#[derive(Debug, Clone, Default)]
pub struct Texture {
    /// Resolved against the FBX file's folder.
    pub filename: String,
    pub relative_filename: String,
    pub absolute_filename: String,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Owns the C scene until it is dropped.
struct Loaded(*mut ffi::LmFbxScene);

impl Drop for Loaded {
    fn drop(&mut self) {
        unsafe { ffi::lm_fbx_free(self.0) }
    }
}

pub fn load(path: &Path) -> Result<Scene, Error> {
    let path = path
        .to_str()
        .ok_or_else(|| Error("the file path is not valid Unicode".into()))?;
    let mut message = [0 as c_char; 1024];
    let raw = unsafe {
        ffi::lm_fbx_load(
            path.as_ptr().cast(),
            path.len(),
            message.as_mut_ptr(),
            message.len(),
        )
    };
    if raw.is_null() {
        let message = unsafe { CStr::from_ptr(message.as_ptr()) };
        return Err(Error(message.to_string_lossy().trim().to_owned()));
    }
    let loaded = Loaded(raw);
    // SAFETY: the shim fills every array with the counts it reports, and
    // everything stays alive until `loaded` drops at the end of this function.
    let s = unsafe { &*loaded.0 };
    Ok(Scene {
        meshes: unsafe { view(s.meshes, s.mesh_count) }
            .iter()
            .map(|m| unsafe { convert_mesh(m) })
            .collect(),
        materials: unsafe { view(s.materials, s.material_count) }
            .iter()
            .map(|m| unsafe { convert_material(m) })
            .collect(),
        textures: unsafe { view(s.textures, s.texture_count) }
            .iter()
            .map(|t| unsafe { convert_texture(t) })
            .collect(),
        skipped_faces: s.skipped_faces,
    })
}

/// A C array as a slice; empty for null or zero-length arrays.
unsafe fn view<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if data.is_null() || len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(data, len) }
    }
}

unsafe fn string(s: &ffi::LmStr) -> String {
    let bytes = unsafe { view(s.data.cast::<u8>(), s.len) };
    String::from_utf8_lossy(bytes).into_owned()
}

unsafe fn chunks<const N: usize>(data: *const f32, count: usize) -> Vec<[f32; N]> {
    unsafe { view(data, count * N) }.as_chunks::<N>().0.to_vec()
}

fn index(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

unsafe fn convert_mesh(m: &ffi::LmFbxMesh) -> Mesh {
    let vertices = m.vertex_count as usize;
    let uvs = unsafe { view(m.uvs, vertices * 2 * m.uv_set_count as usize) };
    Mesh {
        name: unsafe { string(&m.name) },
        positions: unsafe { chunks(m.positions, vertices) },
        normals: unsafe { chunks(m.normals, vertices) },
        uvs: uvs
            .chunks_exact(vertices * 2)
            .map(|set| set.as_chunks::<2>().0.to_vec())
            .collect(),
        colors: (m.has_colors != 0).then(|| unsafe { chunks(m.colors, vertices) }),
        indices: unsafe { view(m.indices, m.index_count as usize) }.to_vec(),
        parts: unsafe { view(m.parts, m.part_count as usize) }
            .iter()
            .map(|p| Part {
                index_start: p.index_start,
                index_count: p.index_count,
                material: index(p.material),
            })
            .collect(),
    }
}

unsafe fn convert_material(m: &ffi::LmFbxMaterial) -> Material {
    Material {
        name: unsafe { string(&m.name) },
        base_color: m.base_color,
        metallic: m.metallic,
        roughness: m.roughness,
        emissive: m.emissive,
        base_color_texture: index(m.base_color_texture),
        normal_texture: index(m.normal_texture),
        occlusion_texture: index(m.occlusion_texture),
        emissive_texture: index(m.emissive_texture),
        double_sided: m.double_sided != 0,
    }
}

unsafe fn convert_texture(t: &ffi::LmFbxTexture) -> Texture {
    Texture {
        filename: unsafe { string(&t.filename) },
        relative_filename: unsafe { string(&t.relative_filename) },
        absolute_filename: unsafe { string(&t.absolute_filename) },
        content: (!t.content.is_null())
            .then(|| unsafe { view(t.content, t.content_size) }.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_files() {
        let err = load(Path::new("does/not/exist.fbx")).unwrap_err();
        assert!(!err.0.is_empty());
    }

    #[test]
    fn reports_files_that_are_not_fbx() {
        let dir = tempfile_path("lm-ufbx-not-fbx.fbx");
        std::fs::write(&dir, b"definitely not an fbx file").unwrap();
        let err = load(&dir).unwrap_err();
        std::fs::remove_file(&dir).ok();
        assert!(!err.0.is_empty());
    }

    fn tempfile_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{}-{name}", std::process::id()))
    }
}
