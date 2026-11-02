//! glTF 2.0 / GLB import.
//!
//! The node hierarchy is flattened: every node with a mesh becomes one
//! [`Mesh`] in world space, and its primitives become submeshes. Images keep
//! their encoded bytes so they export unchanged.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use base64::Engine;
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use super::{Import, mime_from_path};
use crate::mesh::{AlphaMode, Image, Material, Mesh, Submesh, TextureRef};
use crate::{Error, Result};

pub fn load(path: &Path) -> Result<Import> {
    let gltf::Gltf { document, blob } = gltf::Gltf::open(path)?;
    let base = path.parent();
    let buffers = gltf::import_buffers(&document, base, blob)?;
    let mut import = Import::default();

    for image in document.images() {
        import
            .scene
            .images
            .push(load_image(&image, &buffers, base)?);
    }
    let texture_image: Vec<usize> = document.textures().map(|t| t.source().index()).collect();
    import.scene.materials = document
        .materials()
        .map(|m| convert_material(&m, &texture_image))
        .collect();

    let roots: Vec<gltf::Node> = match document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        Some(scene) => scene.nodes().collect(),
        None => {
            let children: HashSet<usize> = document
                .nodes()
                .flat_map(|n| n.children().map(|c| c.index()).collect::<Vec<_>>())
                .collect();
            document
                .nodes()
                .filter(|n| !children.contains(&n.index()))
                .collect()
        }
    };
    for node in roots {
        visit(&node, &Mat4::IDENTITY, &buffers, &mut import);
    }
    Ok(import)
}

fn visit(node: &gltf::Node, parent: &Mat4, buffers: &[gltf::buffer::Data], import: &mut Import) {
    let world = *parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let name = node
            .name()
            .or(mesh.name())
            .map_or_else(|| format!("mesh_{}", mesh.index()), str::to_owned);
        if let Some(converted) = convert_mesh(&mesh, name, &world, buffers, &mut import.warnings) {
            import.scene.meshes.push(converted);
        }
    }
    for child in node.children() {
        visit(&child, &world, buffers, import);
    }
}

/// One primitive's attributes, already in world space.
struct Primitive {
    /// The accessors behind its vertex attributes; primitives with the same
    /// key share one vertex array (as our exporter writes them).
    vertex_key: Vec<(String, usize)>,
    positions: Vec<Vec3>,
    normals: Option<Vec<Vec3>>,
    tangents: Option<Vec<Vec4>>,
    uvs: Vec<Vec<Vec2>>,
    colors: Option<Vec<Vec4>>,
    indices: Vec<u32>,
    material: Option<usize>,
}

fn convert_mesh(
    mesh: &gltf::Mesh,
    name: String,
    world: &Mat4,
    buffers: &[gltf::buffer::Data],
    warnings: &mut Vec<String>,
) -> Option<Mesh> {
    let normal_matrix = Mat3::from_mat4(*world).inverse().transpose();
    let linear = Mat3::from_mat4(*world);
    let mirrored = linear.determinant() < 0.0;

    let mut primitives = Vec::new();
    for primitive in mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            warnings.push(format!(
                "{name}: skipped a {:?} primitive (only triangles are supported)",
                primitive.mode()
            ));
            continue;
        }
        let reader = primitive.reader(|b| Some(&buffers[b.index()]));
        let Some(positions) = reader.read_positions() else {
            warnings.push(format!("{name}: skipped a primitive without positions"));
            continue;
        };
        // Untransformed nodes keep their data bit-exact (no renormalizing).
        let identity = *world == Mat4::IDENTITY;
        let positions: Vec<Vec3> = positions
            .map(|p| world.transform_point3(p.into()))
            .collect();
        let normals = reader.read_normals().map(|n| {
            n.map(|n| match identity {
                true => Vec3::from(n),
                false => (normal_matrix * Vec3::from(n)).normalize_or_zero(),
            })
            .collect()
        });
        let tangents = reader.read_tangents().map(|t| {
            t.map(|[x, y, z, w]| match identity {
                true => Vec4::new(x, y, z, w),
                false => {
                    let dir = (linear * Vec3::new(x, y, z)).normalize_or_zero();
                    dir.extend(if mirrored { -w } else { w })
                }
            })
            .collect()
        });
        let uvs = (0..)
            .map_while(|set| reader.read_tex_coords(set))
            .map(|t| t.into_f32().map(Vec2::from).collect())
            .collect();
        let colors = reader
            .read_colors(0)
            .map(|c| c.into_rgba_f32().map(Vec4::from).collect());
        let mut indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        if mirrored {
            for tri in indices.as_chunks_mut::<3>().0 {
                tri.swap(1, 2);
            }
        }
        let mut vertex_key: Vec<(String, usize)> = primitive
            .attributes()
            .map(|(s, a)| (format!("{s:?}"), a.index()))
            .collect();
        vertex_key.sort();
        primitives.push(Primitive {
            vertex_key,
            positions,
            normals,
            tangents,
            uvs,
            colors,
            indices,
            material: primitive.material().index(),
        });
    }
    if primitives.is_empty() {
        return None;
    }
    Some(merge(name, primitives))
}

/// Concatenates primitives into one mesh, filling attributes some lack.
fn merge(name: String, mut primitives: Vec<Primitive>) -> Mesh {
    let uv_channels = primitives.iter().map(|p| p.uvs.len()).max().unwrap_or(0);
    let all_tangents = primitives.iter().all(|p| p.tangents.is_some());
    let any_colors = primitives.iter().any(|p| p.colors.is_some());

    let mut mesh = Mesh {
        name,
        normals: Some(Vec::new()),
        tangents: all_tangents.then(Vec::new),
        uvs: vec![Vec::new(); uv_channels],
        colors: any_colors.then(Vec::new),
        ..Default::default()
    };
    let mut vertex_offsets: HashMap<Vec<(String, usize)>, u32> = HashMap::new();
    for p in &mut primitives {
        let start = mesh.indices.len() as u32;
        let offset = match vertex_offsets.get(&p.vertex_key) {
            Some(&offset) => offset,
            None => {
                let offset = mesh.positions.len() as u32;
                vertex_offsets.insert(std::mem::take(&mut p.vertex_key), offset);
                append_vertices(&mut mesh, p);
                offset
            }
        };
        mesh.indices.extend(p.indices.iter().map(|i| i + offset));
        mesh.submeshes.push(Submesh {
            index_start: start,
            index_count: p.indices.len() as u32,
            material: p.material,
        });
    }
    mesh
}

fn append_vertices(mesh: &mut Mesh, p: &mut Primitive) {
    let count = p.positions.len();
    if p.normals.is_none() {
        let mut temp = Mesh {
            positions: p.positions.clone(),
            indices: p.indices.clone(),
            ..Default::default()
        };
        temp.generate_normals();
        p.normals = temp.normals;
    }
    mesh.positions.append(&mut p.positions);
    mesh.normals
        .as_mut()
        .unwrap()
        .append(p.normals.as_mut().unwrap());
    if let Some(tangents) = &mut mesh.tangents {
        tangents.append(p.tangents.as_mut().unwrap());
    }
    for (channel, uv) in mesh.uvs.iter_mut().enumerate() {
        match p.uvs.get_mut(channel) {
            Some(src) => uv.append(src),
            None => uv.extend(std::iter::repeat_n(Vec2::ZERO, count)),
        }
    }
    if let Some(colors) = &mut mesh.colors {
        match &mut p.colors {
            Some(src) => colors.append(src),
            None => colors.extend(std::iter::repeat_n(Vec4::ONE, count)),
        }
    }
}

fn convert_material(material: &gltf::Material, texture_image: &[usize]) -> Material {
    let texture = |index: usize, uv_channel: u32| TextureRef {
        image: texture_image[index],
        uv_channel,
    };
    let pbr = material.pbr_metallic_roughness();
    Material {
        name: material.name().unwrap_or_default().to_owned(),
        base_color: Vec4::from(pbr.base_color_factor()),
        base_color_texture: pbr
            .base_color_texture()
            .map(|t| texture(t.texture().index(), t.tex_coord())),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr
            .metallic_roughness_texture()
            .map(|t| texture(t.texture().index(), t.tex_coord())),
        normal_texture: material
            .normal_texture()
            .map(|t| texture(t.texture().index(), t.tex_coord())),
        occlusion_texture: material
            .occlusion_texture()
            .map(|t| texture(t.texture().index(), t.tex_coord())),
        emissive: Vec3::from(material.emissive_factor()),
        emissive_texture: material
            .emissive_texture()
            .map(|t| texture(t.texture().index(), t.tex_coord())),
        alpha_mode: match material.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
        double_sided: material.double_sided(),
    }
}

fn load_image(
    image: &gltf::Image,
    buffers: &[gltf::buffer::Data],
    base: Option<&Path>,
) -> Result<Image> {
    let name = image.name().unwrap_or_default().to_owned();
    let (mime_type, data) = match image.source() {
        gltf::image::Source::View { view, mime_type } => {
            let buffer = &buffers[view.buffer().index()];
            let bytes = buffer[view.offset()..view.offset() + view.length()].to_vec();
            (mime_type.to_owned(), bytes)
        }
        gltf::image::Source::Uri { uri, mime_type } => {
            if let Some(rest) = uri.strip_prefix("data:") {
                let (header, payload) = rest
                    .split_once(',')
                    .ok_or_else(|| Error::Other("malformed data URI in image".into()))?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .map_err(|e| Error::Other(format!("bad base64 image data: {e}")))?;
                let mime = mime_type
                    .or_else(|| header.split(';').next())
                    .unwrap_or("image/png");
                (mime.to_owned(), bytes)
            } else {
                let decoded = percent_decode(uri);
                let file = base.map_or_else(|| decoded.clone().into(), |b| b.join(&decoded));
                let bytes = std::fs::read(&file)?;
                (
                    mime_type.unwrap_or(mime_from_path(&decoded)).to_owned(),
                    bytes,
                )
            }
        }
    };
    Ok(Image {
        name,
        mime_type,
        data,
    })
}

/// Decodes `%XX` escapes in a relative URI (spaces arrive as `%20`).
fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("rock%20diffuse.png"), "rock diffuse.png");
        assert_eq!(percent_decode("100%.png"), "100%.png");
    }
}
