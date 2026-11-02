//! UV debug outputs (docs/algorithms.md §8): a layout PNG per sheet, a class
//! map scene with vertex colours, and a scene textured with a checker.

use std::collections::HashMap;

use glam::{DVec2, Vec4};

use super::UvClass;
use super::metrics::{Layout, TriangleState, coverage, layout};
use crate::mesh::{Image, Material, TextureRef};
use crate::{Mesh, Scene};

/// A mesh as it left the UV stage, with the class of each triangle.
#[derive(Debug, Clone)]
pub struct DebugMesh {
    pub mesh: Mesh,
    pub classes: Vec<UvClass>,
}

impl DebugMesh {
    /// A mesh whose UVs came with the file.
    pub fn input(mesh: Mesh) -> Self {
        let classes = vec![UvClass::Input; mesh.triangle_count()];
        Self { mesh, classes }
    }
}

/// One sheet's layout image.
#[derive(Debug, Clone)]
pub struct LayoutImage {
    /// The sheet's material, as in [`crate::Submesh::material`].
    pub material: Option<usize>,
    pub png: Vec<u8>,
}

/// Fill colour per class, in sRGB. Red and magenta stay free for flips and
/// overlaps.
pub fn class_color(class: UvClass) -> [u8; 3] {
    match class {
        UvClass::Input => [140, 140, 140],
        UvClass::Xatlas => [120, 150, 190],
        UvClass::Repeated => [200, 200, 200],
        UvClass::Planar => [70, 140, 240],
        UvClass::Box => [245, 160, 50],
        UvClass::QuadGrid => [110, 190, 80],
        UvClass::Cylinder => [50, 190, 190],
        UvClass::Cone => [150, 110, 230],
        UvClass::BentTube => [40, 120, 110],
        UvClass::Disk => [235, 215, 70],
        UvClass::Developable => [170, 120, 80],
        UvClass::Organic => [225, 180, 150],
    }
}

const BACKGROUND: [u8; 3] = [30, 30, 34];
const GRID: [u8; 3] = [48, 48, 56];
const SEAM: [u8; 3] = [225, 225, 225];
const FLIP: [u8; 3] = [235, 30, 30];
const OVERLAP: [u8; 3] = [255, 0, 255];
/// Overlaps up to this many texels get a ring each.
const MAX_RINGS: usize = 64;

/// The class colour shaded per island, so neighbouring islands of one class
/// stay distinguishable.
fn island_color(class: UvClass, island: usize) -> [u8; 3] {
    let hash = (island as u32).wrapping_mul(2_654_435_761) >> 24;
    let shade = 0.5 + 0.8 * hash as f32 / 255.0;
    class_color(class).map(|c| (c as f32 * shade).min(255.0) as u8)
}

/// Draws UV0 of each sheet at `size`² pixels, V = 0 at the top as glTF
/// textures are: islands filled by class colour, island borders light, flipped
/// and zero-area triangles red, overlapping texels magenta. Empty if the mesh
/// has no UV0.
pub fn layout_pngs(debug: &DebugMesh, size: u32) -> Vec<LayoutImage> {
    let mesh = &debug.mesh;
    let (Some(uvs), Some(l)) = (mesh.uvs.first(), layout(mesh, 0)) else {
        return Vec::new();
    };
    let mut materials = vec![None; l.sheet_count];
    for sub in mesh.submeshes.iter().filter(|s| s.index_count > 0) {
        materials[l.sheet_of[sub.index_start as usize / 3]] = sub.material;
    }
    let px = |v: u32| {
        let uv = uvs[v as usize];
        DVec2::new(f64::from(uv.x), f64::from(uv.y)) * f64::from(size)
    };
    let tri = |t: usize| -> [u32; 3] { [0, 1, 2].map(|k| mesh.indices[t * 3 + k]) };
    let mut hits = vec![0u8; (size * size) as usize];

    (0..l.sheet_count)
        .map(|sheet| {
            let mut img = Canvas::new(size);
            let in_sheet = |t: &usize| l.sheet_of[*t] == sheet;
            for t in (0..mesh.triangle_count()).filter(in_sheet) {
                let color = match l.state(t) {
                    TriangleState::Degenerate => continue,
                    TriangleState::Good => island_color(debug.classes[t], l.island_of[t]),
                    TriangleState::Flipped | TriangleState::ZeroArea => FLIP,
                };
                img.fill_triangle(tri(t).map(px), color);
            }
            for &(t, a, b) in l.boundary.iter().filter(|(t, ..)| in_sheet(t)) {
                if l.state(t) != TriangleState::Degenerate {
                    img.line(px(a), px(b), SEAM);
                }
            }
            // Outline bad triangles too, since they may cover no pixel centre.
            for t in (0..mesh.triangle_count()).filter(in_sheet) {
                if matches!(l.state(t), TriangleState::Flipped | TriangleState::ZeroArea) {
                    let p = tri(t).map(px);
                    for k in 0..3 {
                        img.line(p[k], p[(k + 1) % 3], FLIP);
                    }
                }
            }
            coverage(mesh, uvs, &l.sheet_of, sheet, size, &mut hits);
            let overlapping: Vec<usize> = (0..hits.len()).filter(|&i| hits[i] >= 2).collect();
            for &i in &overlapping {
                img.set(i, OVERLAP);
            }
            // A few stray texels vanish in a full-size image, so ring them.
            if overlapping.len() <= MAX_RINGS {
                for &i in &overlapping {
                    let (x, y) = (i as u32 % size, i as u32 / size);
                    img.ring(DVec2::new(f64::from(x), f64::from(y)) + 0.5, OVERLAP);
                }
            }
            LayoutImage {
                material: materials[sheet],
                png: img.encode(),
            }
        })
        .collect()
}

/// The meshes with vertex colours by class, shaded per island, under one
/// plain material. Vertices shared by two islands are split.
pub fn class_scene(meshes: &[DebugMesh]) -> Scene {
    let srgb_to_linear = |c: u8| (f32::from(c) / 255.0).powf(2.2);
    let meshes = meshes
        .iter()
        .map(|debug| {
            let mut mesh = debug.mesh.clone();
            let islands = layout(&mesh, 0).map(|l: Layout| l.island_of);
            let island = |t: usize| islands.as_ref().map_or(0, |i| i[t]);
            let mut sources = Vec::new();
            let mut colors = Vec::new();
            let mut slot: HashMap<(u32, UvClass, usize), u32> = HashMap::new();
            for t in 0..mesh.triangle_count() {
                let key = (debug.classes[t], island(t));
                for v in &mut mesh.indices[t * 3..t * 3 + 3] {
                    *v = *slot.entry((*v, key.0, key.1)).or_insert_with(|| {
                        let [r, g, b] = island_color(key.0, key.1).map(srgb_to_linear);
                        sources.push(*v);
                        colors.push(Vec4::new(r, g, b, 1.0));
                        sources.len() as u32 - 1
                    });
                }
            }
            mesh.gather_vertices(&sources);
            mesh.colors = Some(colors);
            mesh.tangents = None;
            single_material(&mut mesh);
            mesh
        })
        .collect();
    Scene {
        meshes,
        materials: vec![Material {
            name: "uv_classes".into(),
            metallic: 0.0,
            ..Default::default()
        }],
        images: Vec::new(),
    }
}

/// The meshes that have UV0, under one material with an embedded checker
/// texture for judging stretch, density, and orientation.
pub fn checker_scene(meshes: &[DebugMesh]) -> Scene {
    let meshes = meshes
        .iter()
        .filter(|d| !d.mesh.uvs.is_empty())
        .map(|debug| {
            let mut mesh = debug.mesh.clone();
            mesh.colors = None;
            mesh.tangents = None;
            single_material(&mut mesh);
            mesh
        })
        .collect();
    Scene {
        meshes,
        materials: vec![Material {
            name: "uv_checker".into(),
            base_color_texture: Some(TextureRef {
                image: 0,
                uv_channel: 0,
            }),
            metallic: 0.0,
            ..Default::default()
        }],
        images: vec![Image {
            name: "uv_checker".into(),
            mime_type: "image/png".into(),
            data: checker_png(1024),
        }],
    }
}

fn single_material(mesh: &mut Mesh) {
    for sub in &mut mesh.submeshes {
        sub.material = Some(0);
    }
}

/// An 8×8 checker whose cells get redder to the right (+U) and greener
/// downward (+V), so mirrored and rotated islands show, with a fine grid of
/// 64 lines across for judging stretch.
fn checker_png(size: u32) -> Vec<u8> {
    let mut img = Canvas::new(size);
    let fine = (size / 64).max(1);
    for y in 0..size {
        for x in 0..size {
            let (i, j) = (x * 8 / size, y * 8 / size);
            let mut c = [
                0.30 + 0.55 * i as f32 / 7.0,
                0.30 + 0.55 * j as f32 / 7.0,
                0.55,
            ];
            let mut k = if (i + j) % 2 == 0 { 1.0 } else { 0.5 };
            if x % fine == 0 || y % fine == 0 {
                k *= 0.8;
            }
            for v in &mut c {
                *v *= k;
            }
            img.set((y * size + x) as usize, c.map(|v| (v * 255.0) as u8));
        }
    }
    img.encode()
}

/// An RGB image with a few drawing primitives.
struct Canvas {
    size: u32,
    rgb: Vec<u8>,
}

impl Canvas {
    fn new(size: u32) -> Self {
        let mut canvas = Self {
            size,
            rgb: BACKGROUND.repeat((size * size) as usize),
        };
        let step = (size / 8).max(1);
        for i in 0..size * size {
            let (x, y) = (i % size, i / size);
            if x % step == 0 || y % step == 0 {
                canvas.set(i as usize, GRID);
            }
        }
        canvas
    }

    fn set(&mut self, i: usize, color: [u8; 3]) {
        self.rgb[i * 3..i * 3 + 3].copy_from_slice(&color);
    }

    /// Fills the pixels whose centres lie inside or on the triangle.
    fn fill_triangle(&mut self, p: [DVec2; 3], color: [u8; 3]) {
        let doubled = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if doubled == 0.0 {
            return;
        }
        let max = f64::from(self.size) - 1.0;
        let lo = p[0].min(p[1]).min(p[2]);
        let hi = p[0].max(p[1]).max(p[2]);
        let (x0, y0) = ((lo.x - 0.5).ceil().max(0.0), (lo.y - 0.5).ceil().max(0.0));
        let (x1, y1) = ((hi.x - 0.5).floor().min(max), (hi.y - 0.5).floor().min(max));
        if x1 < x0 || y1 < y0 {
            return;
        }
        for y in y0 as u32..=y1 as u32 {
            for x in x0 as u32..=x1 as u32 {
                let c = DVec2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let inside = (0..3)
                    .all(|k| (p[(k + 1) % 3] - p[k]).perp_dot(c - p[k]) * doubled.signum() >= 0.0);
                if inside {
                    self.set((y * self.size + x) as usize, color);
                }
            }
        }
    }

    fn line(&mut self, a: DVec2, b: DVec2, color: [u8; 3]) {
        let steps = (b - a).abs().max_element().ceil().max(1.0) as u32;
        for s in 0..=steps {
            let p = a.lerp(b, f64::from(s) / f64::from(steps));
            let (x, y) = (p.x.floor(), p.y.floor());
            if (0.0..f64::from(self.size)).contains(&x) && (0.0..f64::from(self.size)).contains(&y)
            {
                self.set((y as u32 * self.size + x as u32) as usize, color);
            }
        }
    }

    /// A two-pixel circle around `c`, big enough to spot at a glance.
    fn ring(&mut self, c: DVec2, color: [u8; 3]) {
        const SEGMENTS: u32 = 32;
        for r in [7.0, 8.0] {
            for s in 0..SEGMENTS {
                let at = |s: u32| {
                    let a = std::f64::consts::TAU * f64::from(s) / f64::from(SEGMENTS);
                    c + DVec2::new(a.cos(), a.sin()) * r
                };
                self.line(at(s), at(s + 1), color);
            }
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.size, self.size);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("writing PNG to memory cannot fail");
        writer
            .write_image_data(&self.rgb)
            .expect("writing PNG to memory cannot fail");
        writer.finish().expect("writing PNG to memory cannot fail");
        out
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::*;
    use crate::primitives;

    fn decode(png: &[u8]) -> (u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(info.width, info.height);
        buf.truncate(info.buffer_size());
        (info.width, buf)
    }

    fn count(rgb: &[u8], color: [u8; 3]) -> usize {
        rgb.as_chunks::<3>()
            .0
            .iter()
            .filter(|&&c| c == color)
            .count()
    }

    #[test]
    fn layout_fills_islands_with_v_down() {
        // A flat grid squeezed into the top half of the sheet.
        let mut mesh = primitives::grid(1.0, 1.0, 8, 8);
        for uv in &mut mesh.uvs[0] {
            uv.y *= 0.5;
        }
        let images = layout_pngs(&DebugMesh::input(mesh), 64);
        assert_eq!(images.len(), 1);
        let (size, rgb) = decode(&images[0].png);
        assert_eq!(size, 64);
        assert!(count(&rgb, island_color(UvClass::Input, 0)) > 64 * 20);
        assert!(count(&rgb, SEAM) > 0);
        assert_eq!(count(&rgb, FLIP), 0);
        assert_eq!(count(&rgb, OVERLAP), 0);
        let bottom = &rgb[64 * 34 * 3..];
        assert_eq!(count(bottom, island_color(UvClass::Input, 0)), 0);
    }

    #[test]
    fn folds_show_as_flips_and_overlaps() {
        let mut mesh = primitives::grid(1.0, 1.0, 8, 8);
        let centre = mesh.uvs[0]
            .iter()
            .position(|uv| (*uv - Vec2::splat(0.5)).length() < 1e-6)
            .unwrap();
        mesh.uvs[0][centre] = Vec2::splat(0.7);
        let images = layout_pngs(&DebugMesh::input(mesh), 256);
        let (_, rgb) = decode(&images[0].png);
        assert!(count(&rgb, FLIP) > 0);
        assert!(count(&rgb, OVERLAP) > 0);
    }

    #[test]
    fn each_sheet_gets_an_image() {
        let mut debug = DebugMesh::input(primitives::cube(1.0));
        debug.mesh.submeshes = vec![
            crate::Submesh {
                index_start: 0,
                index_count: 18,
                material: Some(3),
            },
            crate::Submesh {
                index_start: 18,
                index_count: 18,
                material: None,
            },
        ];
        let images = layout_pngs(&debug, 32);
        let materials: Vec<_> = images.iter().map(|i| i.material).collect();
        assert_eq!(materials, [Some(3), None]);
        let mut no_uvs = debug.clone();
        no_uvs.mesh.uvs.clear();
        assert!(layout_pngs(&no_uvs, 32).is_empty());
    }

    #[test]
    fn class_scene_colours_every_corner_of_a_triangle_alike() {
        let mut debug = DebugMesh::input(primitives::torus(1.0, 0.3, 12, 6));
        let n = debug.classes.len();
        debug.classes[n / 2..].fill(UvClass::Cylinder);
        let scene = class_scene(&[debug]);
        scene.validate().unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.triangle_count(), n);
        let colors = mesh.colors.as_ref().unwrap();
        for tri in mesh.indices.as_chunks::<3>().0 {
            assert_eq!(colors[tri[0] as usize], colors[tri[1] as usize]);
            assert_eq!(colors[tri[0] as usize], colors[tri[2] as usize]);
        }
        let first = colors[mesh.indices[0] as usize];
        let last = colors[mesh.indices[mesh.indices.len() - 1] as usize];
        assert_ne!(first, last);
        assert!(mesh.submeshes.iter().all(|s| s.material == Some(0)));
    }

    #[test]
    fn checker_scene_embeds_a_texture_and_skips_meshes_without_uvs() {
        let mut bare = primitives::cube(1.0);
        bare.uvs.clear();
        let scene = checker_scene(&[
            DebugMesh::input(primitives::uv_sphere(1.0, 16, 8)),
            DebugMesh::input(bare),
        ]);
        scene.validate().unwrap();
        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.images.len(), 1);
        let (size, _) = decode(&scene.images[0].data);
        assert_eq!(size, 1024);
        assert!(scene.materials[0].base_color_texture.is_some());
    }
}
