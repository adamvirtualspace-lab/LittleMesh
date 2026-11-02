//! glTF 2.0 export, as GLB or as JSON with an embedded base64 buffer.
//!
//! Each mesh becomes one node; each submesh one primitive. All attribute and
//! index data goes into a single buffer, and images are embedded in it too.

use std::path::Path;

use base64::Engine;
use serde_json::{Value, json};

use crate::mesh::{AlphaMode, Material, Mesh, TextureRef};
use crate::{Result, Scene};

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

pub fn save(scene: &Scene, path: &Path, binary: bool) -> Result<()> {
    let (mut doc, bin) = build(scene);
    if binary {
        if !bin.is_empty() {
            doc["buffers"] = json!([{ "byteLength": bin.len() }]);
        }
        std::fs::write(path, glb(&doc, &bin))?;
    } else {
        if !bin.is_empty() {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bin);
            doc["buffers"] = json!([{
                "byteLength": bin.len(),
                "uri": format!("data:application/octet-stream;base64,{encoded}"),
            }]);
        }
        let text =
            serde_json::to_string_pretty(&doc).map_err(|e| crate::Error::Other(e.to_string()))?;
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Accumulates the binary buffer plus the buffer views and accessors into it.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        // Accessors need 4-byte alignment.
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        let mut view = json!({
            "buffer": 0,
            "byteOffset": self.bin.len(),
            "byteLength": bytes.len(),
        });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    fn floats<const N: usize>(&mut self, data: &[[f32; N]], with_bounds: bool) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let view = self.view(&bytes, Some(ARRAY_BUFFER));
        let mut accessor = json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": data.len(),
            "type": match N { 2 => "VEC2", 3 => "VEC3", 4 => "VEC4", _ => unreachable!() },
        });
        if with_bounds {
            let mut lo = [f32::INFINITY; N];
            let mut hi = [f32::NEG_INFINITY; N];
            for v in data {
                for k in 0..N {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            accessor["min"] = json!(lo.as_slice());
            accessor["max"] = json!(hi.as_slice());
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": data.len(),
            "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }
}

fn build(scene: &Scene) -> (Value, Vec<u8>) {
    let mut b = Builder::default();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();

    for mesh in scene.meshes.iter().filter(|m| m.triangle_count() > 0) {
        let attributes = mesh_attributes(&mut b, mesh);
        let primitives: Vec<Value> = mesh
            .submeshes
            .iter()
            .filter(|s| s.index_count > 0)
            .map(|s| {
                let range = s.index_start as usize..(s.index_start + s.index_count) as usize;
                let mut primitive = json!({
                    "attributes": attributes,
                    "indices": b.indices(&mesh.indices[range]),
                    "mode": 4,
                });
                if let Some(m) = s.material {
                    primitive["material"] = json!(m);
                }
                primitive
            })
            .collect();
        nodes.push(json!({ "name": mesh.name, "mesh": meshes.len() }));
        meshes.push(json!({ "name": mesh.name, "primitives": primitives }));
    }

    let images: Vec<Value> = scene
        .images
        .iter()
        .map(|image| {
            let view = b.view(&image.data, None);
            let mut value = json!({ "bufferView": view, "mimeType": image.mime_type });
            if !image.name.is_empty() {
                value["name"] = json!(image.name);
            }
            value
        })
        .collect();

    let mut doc = json!({
        "asset": { "version": "2.0", "generator": concat!("LittleMesh ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": meshes,
    });
    if !scene.materials.is_empty() {
        doc["materials"] = scene.materials.iter().map(material_json).collect();
    }
    if !images.is_empty() {
        // One texture per image, all sharing a default (repeat, mipmapped) sampler.
        doc["textures"] = (0..images.len())
            .map(|i| json!({ "source": i, "sampler": 0 }))
            .collect();
        doc["samplers"] = json!([{}]);
        doc["images"] = json!(images);
    }
    if !b.views.is_empty() {
        doc["bufferViews"] = json!(b.views);
        doc["accessors"] = json!(b.accessors);
    }
    (doc, b.bin)
}

fn mesh_attributes(b: &mut Builder, mesh: &Mesh) -> Value {
    let positions: Vec<[f32; 3]> = mesh.positions.iter().map(|p| p.to_array()).collect();
    let mut attributes = json!({ "POSITION": b.floats(&positions, true) });
    if let Some(normals) = &mesh.normals {
        let data: Vec<[f32; 3]> = normals.iter().map(|n| n.to_array()).collect();
        attributes["NORMAL"] = json!(b.floats(&data, false));
    }
    if let Some(tangents) = &mesh.tangents {
        let data: Vec<[f32; 4]> = tangents.iter().map(|t| t.to_array()).collect();
        attributes["TANGENT"] = json!(b.floats(&data, false));
    }
    for (channel, uvs) in mesh.uvs.iter().enumerate() {
        let data: Vec<[f32; 2]> = uvs.iter().map(|uv| uv.to_array()).collect();
        attributes[format!("TEXCOORD_{channel}")] = json!(b.floats(&data, false));
    }
    if let Some(colors) = &mesh.colors {
        let data: Vec<[f32; 4]> = colors.iter().map(|c| c.to_array()).collect();
        attributes["COLOR_0"] = json!(b.floats(&data, false));
    }
    attributes
}

fn material_json(m: &Material) -> Value {
    let texture = |t: &TextureRef| json!({ "index": t.image, "texCoord": t.uv_channel });
    let mut pbr = json!({
        "baseColorFactor": m.base_color.to_array(),
        "metallicFactor": m.metallic,
        "roughnessFactor": m.roughness,
    });
    if let Some(t) = &m.base_color_texture {
        pbr["baseColorTexture"] = texture(t);
    }
    if let Some(t) = &m.metallic_roughness_texture {
        pbr["metallicRoughnessTexture"] = texture(t);
    }
    let mut value = json!({
        "name": m.name,
        "pbrMetallicRoughness": pbr,
        "emissiveFactor": m.emissive.to_array(),
        "doubleSided": m.double_sided,
    });
    for (key, t) in [
        ("normalTexture", &m.normal_texture),
        ("occlusionTexture", &m.occlusion_texture),
        ("emissiveTexture", &m.emissive_texture),
    ] {
        if let Some(t) = t {
            value[key] = texture(t);
        }
    }
    match m.alpha_mode {
        AlphaMode::Opaque => {}
        AlphaMode::Mask => {
            value["alphaMode"] = json!("MASK");
            value["alphaCutoff"] = json!(m.alpha_cutoff);
        }
        AlphaMode::Blend => value["alphaMode"] = json!("BLEND"),
    }
    value
}

/// Packs a JSON document and binary buffer into the GLB container format.
fn glb(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(doc).expect("JSON values always serialize");
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin = bin.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);

    let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = 12 + 8 + json.len() + bin_chunk;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
    }
    out
}

#[cfg(test)]
mod tests {
    use glam::{Vec3, Vec4};

    use crate::mesh::{Image, Material, Submesh, TextureRef};
    use crate::{Scene, io, primitives};

    fn sample_scene() -> Scene {
        let mut cube = primitives::cube(2.0);
        // Two submeshes with different materials, plus a second UV channel and colors.
        cube.submeshes = vec![
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
        cube.uvs
            .push(cube.uvs[0].iter().map(|uv| uv * 0.5).collect());
        cube.colors = Some(vec![Vec4::new(1.0, 0.5, 0.25, 1.0); cube.vertex_count()]);
        Scene {
            meshes: vec![cube, primitives::torus(1.0, 0.25, 12, 6)],
            materials: vec![
                Material {
                    name: "painted".into(),
                    base_color_texture: Some(TextureRef {
                        image: 0,
                        uv_channel: 0,
                    }),
                    ..Default::default()
                },
                Material {
                    name: "metal".into(),
                    base_color: Vec4::new(0.8, 0.8, 0.9, 1.0),
                    metallic: 1.0,
                    roughness: 0.3,
                    ..Default::default()
                },
            ],
            images: vec![Image {
                name: "paint".into(),
                mime_type: "image/png".into(),
                data: vec![0x89, b'P', b'N', b'G', 1, 2, 3],
            }],
        }
    }

    fn round_trip(extension: &str) {
        let scene = sample_scene();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("out.{extension}"));
        io::save(&scene, &path).unwrap();
        let import = io::load(&path).unwrap();
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);
        assert_eq!(import.scene.meshes, scene.meshes);
        assert_eq!(import.scene.materials, scene.materials);
        assert_eq!(import.scene.images, scene.images);
    }

    #[test]
    fn glb_round_trips() {
        round_trip("glb");
    }

    #[test]
    fn gltf_round_trips() {
        round_trip("gltf");
    }

    #[test]
    fn importer_applies_node_transforms() {
        let scene = Scene {
            meshes: vec![primitives::cube(2.0)],
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moved.gltf");
        io::save(&scene, &path).unwrap();

        // Move and mirror the node by editing the JSON directly.
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        doc["nodes"][0]["translation"] = serde_json::json!([10.0, 0.0, 0.0]);
        doc["nodes"][0]["scale"] = serde_json::json!([-1.0, 1.0, 1.0]);
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();

        let mesh = &io::load(&path).unwrap().scene.meshes[0];
        let (lo, hi) = mesh.bounds().unwrap();
        assert!(lo.abs_diff_eq(Vec3::new(9.0, -1.0, -1.0), 1e-5));
        assert!(hi.abs_diff_eq(Vec3::new(11.0, 1.0, 1.0), 1e-5));

        // The mirror flips winding back, so triangles still face their normals.
        let normals = mesh.normals.as_ref().unwrap();
        for tri in mesh.indices.as_chunks::<3>().0 {
            let p = |k: usize| mesh.positions[tri[k] as usize];
            let face = (p(1) - p(0)).cross(p(2) - p(0));
            assert!(face.dot(normals[tri[0] as usize]) > 0.0);
        }
    }
}
