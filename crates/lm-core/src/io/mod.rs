//! File import and export.

mod fbx_import;
mod gltf_export;
mod gltf_import;
mod obj_import;

use std::path::Path;

use crate::{Error, Result, Scene};

/// A loaded scene plus anything the importer had to skip or guess.
#[derive(Debug, Clone, Default)]
pub struct Import {
    pub scene: Scene,
    pub warnings: Vec<String>,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Loads a glTF/GLB, OBJ or FBX file, picked by extension.
pub fn load(path: &Path) -> Result<Import> {
    let import = match extension(path).as_str() {
        "gltf" | "glb" => gltf_import::load(path)?,
        "obj" => obj_import::load(path)?,
        "fbx" => fbx_import::load(path)?,
        other => {
            return Err(Error::UnsupportedFormat(format!(
                "cannot import '.{other}' files"
            )));
        }
    };
    import.scene.validate()?;
    Ok(import)
}

/// Saves a scene as glTF: binary for `.glb`, JSON with an embedded buffer for `.gltf`.
pub fn save(scene: &Scene, path: &Path) -> Result<()> {
    match extension(path).as_str() {
        "glb" => gltf_export::save(scene, path, true),
        "gltf" => gltf_export::save(scene, path, false),
        other => Err(Error::UnsupportedFormat(format!(
            "cannot export '.{other}' files"
        ))),
    }
}

/// Guesses an image MIME type from a file name.
fn mime_from_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/png"
    }
}
