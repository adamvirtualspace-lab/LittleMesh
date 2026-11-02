//! FBX import through ufbx (binary and ASCII, any version).
//!
//! ufbx converts the scene to glTF's axes (Y up) and units (metres) and the
//! shim bakes every node instance into world space, so this only maps types
//! and finds the texture files. glTF holds only PNG and JPEG images, so other
//! formats (TGA, PSD, TIFF, DDS) are skipped with a warning.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glam::{Vec2, Vec3, Vec4};

use super::Import;
use crate::Result;
use crate::mesh::{AlphaMode, Image, Material, Mesh, Submesh, TextureRef};

pub fn load(path: &Path) -> Result<Import> {
    let fbx = lm_ufbx::load(path)?;
    let mut import = Import::default();
    let base = path.parent().unwrap_or(Path::new("."));

    // Only textures a material uses become images, in first-use order.
    let used: HashSet<usize> = fbx
        .materials
        .iter()
        .flat_map(|m| {
            [
                m.base_color_texture,
                m.normal_texture,
                m.occlusion_texture,
                m.emissive_texture,
            ]
        })
        .flatten()
        .collect();
    let images: Vec<Option<usize>> = fbx
        .textures
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let image = used
                .contains(&i)
                .then(|| load_texture(t, base, &mut import.warnings));
            image.flatten().map(|image| {
                import.scene.images.push(image);
                import.scene.images.len() - 1
            })
        })
        .collect();

    let texture = |t: Option<usize>| {
        t.and_then(|t| images.get(t).copied().flatten())
            .map(|image| TextureRef {
                image,
                uv_channel: 0,
            })
    };
    import.scene.materials = fbx
        .materials
        .iter()
        .map(|m| {
            let [r, g, b, a] = m.base_color;
            Material {
                name: m.name.clone(),
                base_color: Vec4::new(r, g, b, a.clamp(0.0, 1.0)),
                base_color_texture: texture(m.base_color_texture),
                metallic: m.metallic.clamp(0.0, 1.0),
                roughness: m.roughness.clamp(0.0, 1.0),
                normal_texture: texture(m.normal_texture),
                occlusion_texture: texture(m.occlusion_texture),
                emissive: Vec3::from(m.emissive),
                emissive_texture: texture(m.emissive_texture),
                alpha_mode: if a < 1.0 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                },
                double_sided: m.double_sided,
                ..Default::default()
            }
        })
        .collect();

    for (i, m) in fbx.meshes.into_iter().enumerate() {
        let name = match m.name.is_empty() {
            true => format!("mesh_{i}"),
            false => m.name,
        };
        import.scene.meshes.push(Mesh {
            name,
            positions: m.positions.into_iter().map(Vec3::from).collect(),
            normals: Some(m.normals.into_iter().map(Vec3::from).collect()),
            tangents: None,
            // FBX's V axis points up from the bottom of the image; glTF's points down.
            uvs: m
                .uvs
                .into_iter()
                .map(|set| {
                    set.into_iter()
                        .map(|[u, v]| Vec2::new(u, 1.0 - v))
                        .collect()
                })
                .collect(),
            colors: m.colors.map(|c| c.into_iter().map(Vec4::from).collect()),
            indices: m.indices,
            submeshes: m
                .parts
                .iter()
                .map(|p| Submesh {
                    index_start: p.index_start,
                    index_count: p.index_count,
                    material: p.material,
                })
                .collect(),
        });
    }
    if fbx.skipped_faces > 0 {
        import.warnings.push(format!(
            "skipped {} point or line faces (only polygons are imported)",
            fbx.skipped_faces
        ));
    }
    Ok(import)
}

/// The texture's bytes, embedded or from disk. FBX files often keep the
/// artist's absolute path, so the file name next to the FBX is tried too.
fn load_texture(t: &lm_ufbx::Texture, base: &Path, warnings: &mut Vec<String>) -> Option<Image> {
    let written = [&t.relative_filename, &t.absolute_filename, &t.filename]
        .into_iter()
        .find(|f| !f.is_empty())
        .cloned()
        .unwrap_or_default();
    let name = file_name(&written).to_owned();
    let data = match &t.content {
        Some(data) => data.clone(),
        None => {
            let candidates: Vec<PathBuf> = [&t.filename, &t.absolute_filename]
                .into_iter()
                .filter(|f| !f.is_empty())
                .map(PathBuf::from)
                .chain([base.join(&t.relative_filename), base.join(&name)])
                .collect();
            match candidates.iter().find_map(|p| std::fs::read(p).ok()) {
                Some(data) => data,
                None => {
                    warnings.push(format!("texture '{written}' not found"));
                    return None;
                }
            }
        }
    };
    let Some(mime_type) = sniff_mime(&data) else {
        warnings.push(format!(
            "texture '{name}' skipped: glTF only holds PNG and JPEG images"
        ));
        return None;
    };
    Some(Image {
        name,
        mime_type: mime_type.to_owned(),
        data,
    })
}

/// The last component of a path written on any OS.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn sniff_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad lying flat and a triangle standing up, Z up in centimetres (as
    /// Blender and 3ds Max write), with one Lambert material each.
    const Z_UP_CM: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",2
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",1
		P: "FrontAxisSign", "int", "Integer", "",-1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}
}
Objects:  {
	Geometry: 1000, "Geometry::Shape", "Mesh" {
		Vertices: *15 {
			a: 0,0,0,100,0,0,100,100,0,0,100,0,0,0,100
		}
		PolygonVertexIndex: *7 {
			a: 0,1,2,-4,0,1,-5
		}
		GeometryVersion: 124
		LayerElementUV: 0 {
			Version: 101
			Name: "UVMap"
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "IndexToDirect"
			UV: *8 {
				a: 0,0,1,0,1,1,0,1
			}
			UVIndex: *7 {
				a: 0,1,2,3,0,1,3
			}
		}
		LayerElementMaterial: 0 {
			Version: 101
			Name: ""
			MappingInformationType: "ByPolygon"
			ReferenceInformationType: "IndexToDirect"
			Materials: *2 {
				a: 0,1
			}
		}
		Layer: 0 {
			Version: 100
			LayerElement:  {
				Type: "LayerElementUV"
				TypedIndex: 0
			}
			LayerElement:  {
				Type: "LayerElementMaterial"
				TypedIndex: 0
			}
		}
	}
	Model: 2000, "Model::Shape", "Mesh" {
		Version: 232
	}
	Material: 3000, "Material::Red", "" {
		Version: 102
		ShadingModel: "lambert"
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",1,0,0
		}
	}
	Material: 3001, "Material::Blue", "" {
		Version: 102
		ShadingModel: "lambert"
		Properties70:  {
			P: "DiffuseColor", "Color", "", "A",0,0,1
		}
	}
}
Connections:  {
	C: "OO",2000,0
	C: "OO",1000,2000
	C: "OO",3000,2000
	C: "OO",3001,2000
}
"#;

    #[test]
    fn imports_z_up_centimetres_as_y_up_metres() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shape.fbx");
        std::fs::write(&path, Z_UP_CM).unwrap();
        let import = crate::io::load(&path).unwrap();
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);

        let scene = import.scene;
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.name, "Shape");
        assert_eq!(mesh.triangle_count(), 3, "the quad is triangulated");
        assert_eq!(mesh.submeshes.len(), 2);
        assert!(mesh.normals.is_some(), "missing normals are generated");

        // Z up becomes Y up and 100 cm becomes 1 m.
        let (min, max) = mesh.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-5), "{min}");
        assert!(max.abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1e-5), "{max}");

        let colors: Vec<Vec4> = scene.materials.iter().map(|m| m.base_color).collect();
        assert_eq!(
            colors,
            [Vec4::new(1.0, 0.0, 0.0, 1.0), Vec4::new(0.0, 0.0, 1.0, 1.0)]
        );
        let materials: Vec<_> = mesh.submeshes.iter().map(|s| s.material).collect();
        assert_eq!(materials, [Some(0), Some(1)]);
        // UV 0 0 is the bottom-left of the image, which is V = 1 in glTF.
        assert!(mesh.uvs[0].contains(&Vec2::new(0.0, 1.0)));
    }

    #[test]
    fn file_names_from_any_os() {
        assert_eq!(file_name(r"C:\art\rock\diffuse.png"), "diffuse.png");
        assert_eq!(file_name("textures/diffuse.png"), "diffuse.png");
        assert_eq!(file_name("diffuse.png"), "diffuse.png");
    }

    #[test]
    fn only_png_and_jpeg_are_kept() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_mime(b"\0\0\x02\0"), None, "TGA");
    }
}
