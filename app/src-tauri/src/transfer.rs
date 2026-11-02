//! Binary mesh transfer to the viewport (M3.2): raw little-endian buffers over
//! Tauri IPC, so a million triangles never go through JSON.
//!
//! Layout, all little-endian and 4-byte aligned:
//!
//! ```text
//! u32 mesh_count
//! per mesh:
//!   u32 vertex_count, u32 index_count, u32 flags (bit 0: has UV0), u32 name_len
//!   name bytes (UTF-8), zero-padded to a multiple of 4
//!   vertex_count × 8 f32: position xyz, normal xyz, uv0 (zero without UVs)
//!   index_count × u32
//! ```
//!
//! `app/src/transfer.ts` decodes it; keep the two in sync.

use lm_core::Mesh;

pub const FLAG_HAS_UV: u32 = 1;
pub const FLOATS_PER_VERTEX: usize = 8;

/// Encodes `meshes` for the viewport. Meshes without normals get generated ones.
pub fn encode(meshes: &[&Mesh]) -> Vec<u8> {
    let size: usize = 4 + meshes
        .iter()
        .map(|m| {
            16 + m.name.len().next_multiple_of(4)
                + m.vertex_count() * FLOATS_PER_VERTEX * 4
                + m.indices.len() * 4
        })
        .sum::<usize>();
    let mut out = Vec::with_capacity(size);
    fn u32s(out: &mut Vec<u8>, values: &[u32]) {
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    u32s(&mut out, &[meshes.len() as u32]);
    for &mesh in meshes {
        let generated;
        let normals = match &mesh.normals {
            Some(n) => n,
            None => {
                let mut copy = mesh.clone();
                copy.generate_normals();
                generated = copy.normals.unwrap_or_default();
                &generated
            }
        };
        let uvs = mesh.uvs.first();
        let flags = if uvs.is_some() { FLAG_HAS_UV } else { 0 };
        u32s(
            &mut out,
            &[
                mesh.vertex_count() as u32,
                mesh.indices.len() as u32,
                flags,
                mesh.name.len() as u32,
            ],
        );
        out.extend_from_slice(mesh.name.as_bytes());
        out.resize(out.len().next_multiple_of(4), 0);
        for (i, p) in mesh.positions.iter().enumerate() {
            let n = normals.get(i).copied().unwrap_or_default();
            let uv = uvs.map_or([0.0; 2], |u| u[i].to_array());
            for f in [p.x, p.y, p.z, n.x, n.y, n.z, uv[0], uv[1]] {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        u32s(&mut out, &mesh.indices);
    }
    debug_assert_eq!(out.len(), size);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_core::primitives;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn f32_at(b: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn encodes_header_vertices_and_indices() {
        let mut cube = primitives::cube(2.0);
        cube.name = "box".into(); // 3 bytes, padded to 4
        let b = encode(&[&cube]);
        assert_eq!(u32_at(&b, 0), 1);
        assert_eq!(u32_at(&b, 4) as usize, cube.vertex_count());
        assert_eq!(u32_at(&b, 8) as usize, cube.indices.len());
        assert_eq!(u32_at(&b, 12), FLAG_HAS_UV);
        assert_eq!(u32_at(&b, 16), 3);
        assert_eq!(&b[20..23], b"box");
        let vertices = 24;
        let p = cube.positions[1];
        let n = cube.normals.as_ref().unwrap()[1];
        let uv = cube.uvs[0][1];
        let v1 = vertices + FLOATS_PER_VERTEX * 4;
        let floats: Vec<f32> = (0..8).map(|k| f32_at(&b, v1 + k * 4)).collect();
        assert_eq!(floats, [p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y]);
        let indices = vertices + cube.vertex_count() * FLOATS_PER_VERTEX * 4;
        assert_eq!(u32_at(&b, indices + 4), cube.indices[1]);
        assert_eq!(b.len(), indices + cube.indices.len() * 4);
    }

    #[test]
    fn fills_in_missing_normals_and_uvs() {
        let mut sphere = primitives::uv_sphere(1.0, 8, 4);
        sphere.normals = None;
        sphere.uvs.clear();
        let b = encode(&[&sphere, &primitives::cube(1.0)]);
        assert_eq!(u32_at(&b, 0), 2);
        assert_eq!(u32_at(&b, 12), 0);
        let name = sphere.name.len().next_multiple_of(4);
        let first = 20 + name;
        let normal: Vec<f32> = (3..6).map(|k| f32_at(&b, first + k * 4)).collect();
        let length = normal.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-4, "{normal:?}");
        assert_eq!(f32_at(&b, first + 6 * 4), 0.0);
    }
}
