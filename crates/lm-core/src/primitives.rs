//! Procedural test shapes with normals and UV0.
//!
//! Each shape is a parametric grid with vertices split along its UV seams, the
//! same way artist-made meshes arrive, so tests exercise the seam handling.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use crate::mesh::{Mesh, Submesh};

/// Builds a `cols × rows` quad grid from a surface function returning
/// `(position, outward normal)` for parameters `u, v ∈ [0, 1]`.
///
/// Triangles are wound counter-clockwise around the outward normal, and
/// triangles that collapse to zero area (at sphere poles) are skipped.
fn param_grid(
    name: &str,
    cols: u32,
    rows: u32,
    surface: impl Fn(f32, f32) -> (Vec3, Vec3),
) -> Mesh {
    let mut mesh = Mesh {
        name: name.to_owned(),
        normals: Some(Vec::new()),
        uvs: vec![Vec::new()],
        ..Default::default()
    };
    for j in 0..=rows {
        for i in 0..=cols {
            let uv = Vec2::new(i as f32 / cols as f32, j as f32 / rows as f32);
            let (p, n) = surface(uv.x, uv.y);
            mesh.positions.push(p);
            mesh.normals.as_mut().unwrap().push(n);
            mesh.uvs[0].push(uv);
        }
    }
    let at = |i: u32, j: u32| j * (cols + 1) + i;
    for j in 0..rows {
        for i in 0..cols {
            let quad = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                push_oriented(&mut mesh, tri);
            }
        }
    }
    finish(mesh)
}

/// Pushes a triangle, flipped if needed so it faces along its vertex normals.
fn push_oriented(mesh: &mut Mesh, [a, b, c]: [u32; 3]) {
    let p = |i: u32| mesh.positions[i as usize];
    let face = (p(b) - p(a)).cross(p(c) - p(a));
    if face.length_squared() < 1e-12 {
        return;
    }
    let normals = mesh.normals.as_ref().unwrap();
    let avg = normals[a as usize] + normals[b as usize] + normals[c as usize];
    if face.dot(avg) >= 0.0 {
        mesh.indices.extend([a, b, c]);
    } else {
        mesh.indices.extend([a, c, b]);
    }
}

fn finish(mut mesh: Mesh) -> Mesh {
    mesh.submeshes = vec![Submesh {
        index_start: 0,
        index_count: mesh.indices.len() as u32,
        material: None,
    }];
    mesh
}

/// Appends `other` to `mesh`, keeping a single submesh.
fn append(mesh: &mut Mesh, other: Mesh) {
    let offset = mesh.positions.len() as u32;
    mesh.positions.extend(other.positions);
    mesh.normals
        .get_or_insert_with(Vec::new)
        .extend(other.normals.unwrap_or_default());
    if mesh.uvs.is_empty() {
        mesh.uvs.push(Vec::new());
    }
    mesh.uvs[0].extend(other.uvs.into_iter().next().unwrap_or_default());
    mesh.indices
        .extend(other.indices.into_iter().map(|i| i + offset));
}

/// An axis-aligned cube centred on the origin: 6 faces, 24 split vertices.
pub fn cube(size: f32) -> Mesh {
    let h = size * 0.5;
    let mut mesh = Mesh {
        name: "cube".into(),
        ..Default::default()
    };
    for axis in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ] {
        // Two tangent axes spanning the face.
        let t = if axis.y.abs() > 0.5 { Vec3::X } else { Vec3::Y };
        let s = axis.cross(t);
        let face = param_grid("face", 1, 1, |u, v| {
            (
                axis * h + s * (u * 2.0 - 1.0) * h + t * (v * 2.0 - 1.0) * h,
                axis,
            )
        });
        append(&mut mesh, face);
    }
    finish(mesh)
}

/// A flat grid on the XZ plane facing +Y, centred on the origin.
pub fn grid(width: f32, depth: f32, cols: u32, rows: u32) -> Mesh {
    param_grid("grid", cols, rows, |u, v| {
        (
            Vec3::new((u - 0.5) * width, 0.0, (v - 0.5) * depth),
            Vec3::Y,
        )
    })
}

/// An open cylinder along Y (no caps): a tube with two boundary loops.
pub fn cylinder(radius: f32, height: f32, segments: u32, rings: u32) -> Mesh {
    param_grid("cylinder", segments, rings, |u, v| {
        let (s, c) = (u % 1.0 * TAU).sin_cos();
        let n = Vec3::new(c, 0.0, s);
        (n * radius + Vec3::Y * (v - 0.5) * height, n)
    })
}

/// A torus around Y: closed, genus 1.
pub fn torus(major: f32, minor: f32, major_segments: u32, minor_segments: u32) -> Mesh {
    param_grid("torus", major_segments, minor_segments, |u, v| {
        let (su, cu) = (u % 1.0 * TAU).sin_cos();
        let (sv, cv) = (v % 1.0 * TAU).sin_cos();
        let ring = Vec3::new(cu, 0.0, su);
        let n = ring * cv + Vec3::Y * sv;
        (ring * major + n * minor, n)
    })
}

/// A UV sphere: closed, genus 0.
pub fn uv_sphere(radius: f32, segments: u32, rings: u32) -> Mesh {
    param_grid("sphere", segments, rings, |u, v| {
        let (su, cu) = (u % 1.0 * TAU).sin_cos();
        // Snap the poles so every column welds to one pole vertex.
        let (sv, cv) = match v {
            0.0 => (0.0, 1.0),
            1.0 => (0.0, -1.0),
            _ => (v * PI).sin_cos(),
        };
        let n = Vec3::new(cu * sv, -cv, su * sv);
        (n * radius, n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangles_face_outward() {
        for mesh in [
            cube(1.0),
            cylinder(1.0, 1.0, 8, 2),
            torus(1.0, 0.25, 12, 8),
            uv_sphere(1.0, 12, 6),
        ] {
            let normals = mesh.normals.as_ref().unwrap();
            for tri in mesh.indices.as_chunks::<3>().0 {
                let p = |k: usize| mesh.positions[tri[k] as usize];
                let face = (p(1) - p(0)).cross(p(2) - p(0));
                assert!(
                    face.dot(normals[tri[0] as usize]) > 0.0,
                    "{} has an inward triangle",
                    mesh.name
                );
            }
        }
    }

    #[test]
    fn sphere_skips_pole_degenerates() {
        let sphere = uv_sphere(1.0, 8, 4);
        // 8 × 4 quads = 64 triangles, minus one degenerate per quad on each pole row.
        assert_eq!(sphere.triangle_count(), 64 - 16);
    }
}
