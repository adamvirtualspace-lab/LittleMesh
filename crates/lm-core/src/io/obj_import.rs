//! Wavefront OBJ import (with MTL materials).
//!
//! tobj splits an object into one model per material; consecutive models with
//! the same name are merged back into one mesh with a submesh per material.

use std::collections::HashMap;
use std::path::Path;

use glam::{Vec2, Vec3, Vec4};

use super::{Import, mime_from_path};
use crate::Result;
use crate::mesh::{Image, Material, Mesh, Submesh, TextureRef};

pub fn load(path: &Path) -> Result<Import> {
    let options = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let (models, materials) = tobj::load_obj(path, &options)?;
    let mut import = Import::default();
    let base = path.parent().unwrap_or(Path::new("."));

    let materials = materials.unwrap_or_else(|e| {
        import.warnings.push(format!("materials not loaded: {e}"));
        Vec::new()
    });
    let mut image_by_path: HashMap<String, usize> = HashMap::new();
    for m in &materials {
        let mut texture = |file: &Option<String>, import: &mut Import| {
            let file = file.as_ref()?;
            if let Some(&image) = image_by_path.get(file) {
                return Some(TextureRef {
                    image,
                    uv_channel: 0,
                });
            }
            match std::fs::read(base.join(file)) {
                Ok(data) => {
                    import.scene.images.push(Image {
                        name: file.clone(),
                        mime_type: mime_from_path(file).to_owned(),
                        data,
                    });
                    let image = import.scene.images.len() - 1;
                    image_by_path.insert(file.clone(), image);
                    Some(TextureRef {
                        image,
                        uv_channel: 0,
                    })
                }
                Err(e) => {
                    import
                        .warnings
                        .push(format!("texture '{file}' not loaded: {e}"));
                    None
                }
            }
        };
        let base_color_texture = texture(&m.diffuse_texture, &mut import);
        let normal_texture = texture(&m.normal_texture, &mut import);
        let diffuse = m.diffuse.unwrap_or([1.0; 3]);
        // Blinn-Phong exponent to an approximate GGX roughness.
        let roughness = m
            .shininess
            .map_or(1.0, |ns| (2.0 / (ns.max(0.0) + 2.0)).sqrt());
        import.scene.materials.push(Material {
            name: m.name.clone(),
            base_color: Vec4::new(
                diffuse[0],
                diffuse[1],
                diffuse[2],
                m.dissolve.unwrap_or(1.0),
            ),
            base_color_texture,
            normal_texture,
            metallic: 0.0,
            roughness,
            ..Default::default()
        });
    }

    for model in models {
        let tobj::Model { mesh: src, name } = model;
        if src.indices.is_empty() {
            continue;
        }
        let merge_into_last = import.scene.meshes.last().is_some_and(|m| m.name == name);
        if !merge_into_last {
            import.scene.meshes.push(Mesh {
                name: name.clone(),
                ..Default::default()
            });
        }
        let mesh = import.scene.meshes.last_mut().unwrap();
        append_model(mesh, &src, materials.len(), &mut import.warnings);
    }

    // Fill normals the file didn't provide.
    for mesh in &mut import.scene.meshes {
        if mesh.normals.is_none() {
            mesh.generate_normals();
        }
    }
    Ok(import)
}

fn append_model(
    mesh: &mut Mesh,
    src: &tobj::Mesh,
    material_count: usize,
    warnings: &mut Vec<String>,
) {
    let count = src.positions.len() / 3;
    let offset = mesh.positions.len() as u32;
    let had_vertices = offset > 0;

    let positions = src
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| Vec3::new(p[0], p[1], p[2]));
    mesh.positions.extend(positions);

    // An attribute present in only some models gets filled for the others,
    // except normals: a partial set is dropped and regenerated later.
    let normals = (!src.normals.is_empty()).then(|| {
        src.normals
            .as_chunks::<3>()
            .0
            .iter()
            .map(|n| Vec3::new(n[0], n[1], n[2]))
    });
    match (normals, had_vertices) {
        (Some(n), false) => mesh.normals = Some(n.collect()),
        (Some(n), true) => {
            if let Some(existing) = &mut mesh.normals {
                existing.extend(n);
            }
        }
        (None, _) => mesh.normals = None,
    }

    if !src.texcoords.is_empty() {
        if mesh.uvs.is_empty() {
            mesh.uvs.push(vec![Vec2::ZERO; offset as usize]);
        }
        // OBJ's V axis points up from the bottom of the image; glTF's points down.
        mesh.uvs[0].extend(
            src.texcoords
                .as_chunks::<2>()
                .0
                .iter()
                .map(|t| Vec2::new(t[0], 1.0 - t[1])),
        );
    } else if let Some(uv) = mesh.uvs.first_mut() {
        uv.extend(std::iter::repeat_n(Vec2::ZERO, count));
    }

    if !src.vertex_color.is_empty() {
        let colors = mesh
            .colors
            .get_or_insert_with(|| vec![Vec4::ONE; offset as usize]);
        colors.extend(
            src.vertex_color
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| Vec4::new(c[0], c[1], c[2], 1.0)),
        );
    } else if let Some(colors) = &mut mesh.colors {
        colors.extend(std::iter::repeat_n(Vec4::ONE, count));
    }

    let material = src.material_id.filter(|&m| {
        let exists = m < material_count;
        if !exists {
            warnings.push(format!("{}: material {m} is missing", mesh.name));
        }
        exists
    });
    let start = mesh.indices.len() as u32;
    mesh.indices.extend(src.indices.iter().map(|i| i + offset));
    mesh.submeshes.push(Submesh {
        index_start: start,
        index_count: src.indices.len() as u32,
        material,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_MATERIALS: &str = "\
mtllib box.mtl
o box
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
v 0 0 1
v 1 0 1
vt 0 0
vt 1 0
vt 1 1
vt 0 1
usemtl red
f 1/1 2/2 3/3 4/4
usemtl blue
f 1/1 2/2 6/3 5/4
";
    const MTL: &str = "\
newmtl red
Kd 1 0 0
newmtl blue
Kd 0 0 1
Ns 98
";

    #[test]
    fn imports_quads_with_two_materials() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("box.obj"), TWO_MATERIALS).unwrap();
        std::fs::write(dir.path().join("box.mtl"), MTL).unwrap();
        let import = crate::io::load(&dir.path().join("box.obj")).unwrap();
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);

        let scene = import.scene;
        assert_eq!(scene.meshes.len(), 1, "both materials merge into one mesh");
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.triangle_count(), 4, "quads are triangulated");
        assert_eq!(mesh.submeshes.len(), 2);
        assert!(mesh.normals.is_some(), "missing normals are generated");
        assert_eq!(scene.materials[0].base_color, Vec4::new(1.0, 0.0, 0.0, 1.0));
        assert!(scene.materials[1].roughness < 0.2);
        // vt 0 0 is the bottom-left of the image, which is V = 1 in glTF.
        assert!(mesh.uvs[0].contains(&Vec2::new(0.0, 1.0)));
    }
}
