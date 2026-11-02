//! UV quality metrics (docs/algorithms.md §7.1, §7.8): distortion, flips,
//! overlaps, seams, straightness, texel density, and packing.
//!
//! Metrics are taken per *sheet*, the triangles that share a material and so
//! one 0–1 texture space, and combined weighted by 3D area.

use std::collections::HashMap;

use glam::DVec2;
use serde::Serialize;

use crate::Mesh;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UvMetrics {
    pub channel: usize,
    /// Groups of triangles sharing one texture space (one per material).
    pub sheets: usize,
    /// Connected UV patches.
    pub islands: usize,
    /// Triangles wound against the rest of their island: folds. Must be 0.
    pub flipped_triangles: usize,
    /// Triangles with 3D area but no UV area. Must be 0.
    pub zero_area_triangles: usize,
    /// Islands mirrored as a whole (fine for most engines, but worth knowing).
    pub mirrored_islands: usize,
    /// Texels covered by more than one triangle at `raster_resolution`. Must be 0.
    pub overlap_texels: usize,
    pub raster_resolution: u32,
    /// Triangles with a UV outside the 0–1 square.
    pub out_of_bounds_triangles: usize,
    /// σ₁/σ₂, area-weighted mean; 1 is conformal.
    pub angle_distortion_mean: f64,
    /// σ₁/σ₂, area-weighted 95th percentile.
    pub angle_distortion_p95: f64,
    /// |log₂(σ₁σ₂ / s)|, area-weighted mean; 0 is perfect.
    pub area_distortion_mean: f64,
    /// Sander et al. L2 stretch, normalized so 1 is perfect.
    pub l2_stretch: f64,
    /// Total 3D length of seam edges, divided by sqrt(surface area).
    pub seam_length: f64,
    /// Fraction of island boundary length (in UV) that is horizontal or vertical.
    pub straight_edge_ratio: f64,
    /// Per-island texel density against the sheet's: area-weighted mean of |ratio − 1|.
    pub density_deviation_mean: f64,
    /// Largest |ratio − 1| of any island.
    pub density_deviation_max: f64,
    /// UV area used per sheet, averaged over sheets.
    pub packing_efficiency: f64,
}

/// One triangle's side of a 3D edge: the triangle and the UV bits at the
/// edge's two ends, ordered by remapped vertex index.
type EdgeSide = (usize, [[u32; 2]; 2]);

/// An edge counts as straight within this angle of an axis (about 1°).
const STRAIGHT_SIN: f64 = 0.0175;

/// Per-triangle UV structure shared by the metrics and the debug outputs.
pub(crate) struct Layout {
    /// Sheet (material group) of each triangle.
    pub sheet_of: Vec<usize>,
    pub sheet_count: usize,
    /// Island of each triangle, numbered from 0.
    pub island_of: Vec<usize>,
    pub island_count: usize,
    pub area3: Vec<f64>,
    /// Signed UV area, measured with V flipped.
    pub area_uv: Vec<f64>,
    /// Singular values of each triangle's 3D → UV Jacobian.
    pub sigma: Vec<(f64, f64)>,
    /// Area-weighted orientation per island; negative means mirrored.
    pub orientation: Vec<f64>,
    /// UV-to-3D area ratio per sheet.
    pub scale: Vec<f64>,
    /// Island boundary edges as (triangle, vertex a, vertex b).
    pub boundary: Vec<(usize, u32, u32)>,
    /// Total 3D length of seam edges.
    pub seam_length: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TriangleState {
    Good,
    /// Wound against the rest of its island.
    Flipped,
    /// 3D area but no UV area.
    ZeroArea,
    /// No 3D area, so nothing to measure.
    Degenerate,
}

impl Layout {
    pub fn state(&self, t: usize) -> TriangleState {
        if self.area3[t] == 0.0 {
            return TriangleState::Degenerate;
        }
        let (s1, s2) = self.sigma[t];
        if s1 * s2 <= self.scale[self.sheet_of[t]] * 1e-9 || self.area_uv[t] == 0.0 {
            TriangleState::ZeroArea
        } else if self.area_uv[t].signum() * self.orientation[self.island_of[t]] < 0.0 {
            TriangleState::Flipped
        } else {
            TriangleState::Good
        }
    }
}

/// Builds the [`Layout`] of UV channel `channel`; `None` if the mesh doesn't have it.
pub(crate) fn layout(mesh: &Mesh, channel: usize) -> Option<Layout> {
    let uvs = mesh.uvs.get(channel)?;
    let tri_count = mesh.triangle_count();
    // glTF's V points down, so flip it to measure winding in a right-handed frame.
    let uv = |v: u32| {
        let t = uvs[v as usize];
        DVec2::new(f64::from(t.x), -f64::from(t.y))
    };
    let pos = |v: u32| mesh.positions[v as usize].as_dvec3();
    let tri = |t: usize| -> [u32; 3] { [0, 1, 2].map(|k| mesh.indices[t * 3 + k]) };

    // Sheets: triangles grouped by material.
    let mut sheet_of = vec![0usize; tri_count];
    let mut sheet_ids: HashMap<Option<usize>, usize> = HashMap::new();
    for sub in &mesh.submeshes {
        let next = sheet_ids.len();
        let sheet = *sheet_ids.entry(sub.material).or_insert(next);
        let first = sub.index_start as usize / 3;
        for s in &mut sheet_of[first..first + sub.index_count as usize / 3] {
            *s = sheet;
        }
    }
    let sheet_count = sheet_ids.len().max(1);

    // Per-triangle Jacobian of the 3D → UV map.
    let mut area3 = vec![0.0f64; tri_count];
    let mut area_uv = vec![0.0f64; tri_count]; // signed
    let mut sigma = vec![(0.0f64, 0.0f64); tri_count];
    for t in 0..tri_count {
        let [a, b, c] = tri(t);
        let (e1, e2) = (pos(b) - pos(a), pos(c) - pos(a));
        let n = e1.cross(e2);
        let double_area = n.length();
        if double_area <= f64::EPSILON * e1.length_squared().max(e2.length_squared()) {
            continue;
        }
        let x = e1.normalize();
        let y = n.cross(e1).normalize();
        let (q1x, q2x, q2y) = (e1.length(), e2.dot(x), e2.dot(y));
        let (d1, d2) = (uv(b) - uv(a), uv(c) - uv(a));
        // J = D · Q⁻¹ with Q = [[q1x, q2x], [0, q2y]].
        let j00 = d1.x / q1x;
        let j10 = d1.y / q1x;
        let j01 = (d2.x - j00 * q2x) / q2y;
        let j11 = (d2.y - j10 * q2x) / q2y;
        let (e, f) = ((j00 + j11) / 2.0, (j00 - j11) / 2.0);
        let (g, h) = ((j10 + j01) / 2.0, (j10 - j01) / 2.0);
        let (q, r) = (e.hypot(h), f.hypot(g));
        area3[t] = double_area / 2.0;
        area_uv[t] = d1.perp_dot(d2) / 2.0;
        sigma[t] = (q + r, (q - r).abs());
    }

    // UV-space edges: triangles sharing a 3D edge with identical UVs at both
    // ends belong to the same island; differing UVs make the edge a seam.
    let remap = mesh.position_remap();
    let uv_key = |v: u32| {
        let t = uvs[v as usize];
        [t.x.to_bits(), t.y.to_bits()]
    };
    let mut edges: HashMap<(u32, u32), Vec<EdgeSide>> = HashMap::new();
    for t in 0..tri_count {
        let v = tri(t);
        for k in 0..3 {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            let (ra, rb) = (remap[a as usize], remap[b as usize]);
            if ra == rb {
                continue;
            }
            // Orient by remapped index so both sides agree on endpoint order.
            let (key, ends) = if ra < rb {
                ((ra, rb), [uv_key(a), uv_key(b)])
            } else {
                ((rb, ra), [uv_key(b), uv_key(a)])
            };
            edges.entry(key).or_default().push((t, ends));
        }
    }

    let mut islands = UnionFind::new(tri_count);
    let mut seam_length = 0.0;
    // Boundary edges per triangle side, as (triangle, uv a, uv b) to measure later.
    let mut boundary: Vec<(usize, u32, u32)> = Vec::new();
    for (&(ra, rb), sides) in &edges {
        let mut seam = false;
        for (i, &(t, ends)) in sides.iter().enumerate() {
            for &(u, other) in &sides[i + 1..] {
                if ends == other && sheet_of[t] == sheet_of[u] {
                    islands.union(t, u);
                } else {
                    seam = true;
                }
            }
        }
        if seam {
            seam_length += (pos(ra) - pos(rb)).length();
        }
        // A side is on an island boundary unless another side matches it.
        for (i, &(t, ends)) in sides.iter().enumerate() {
            let matched = sides
                .iter()
                .enumerate()
                .any(|(j, &(u, other))| j != i && other == ends && sheet_of[t] == sheet_of[u]);
            if !matched {
                let v = tri(t);
                let k = (0..3)
                    .find(|&k| {
                        let (a, b) = (remap[v[k] as usize], remap[v[(k + 1) % 3] as usize]);
                        (a, b) == (ra, rb) || (a, b) == (rb, ra)
                    })
                    .unwrap_or(0);
                boundary.push((t, v[k], v[(k + 1) % 3]));
            }
        }
    }

    // Island orientation by area-weighted majority; minority triangles are flips.
    let island_of: Vec<usize> = (0..tri_count).map(|t| islands.find(t)).collect();
    let mut island_ids: HashMap<usize, usize> = HashMap::new();
    for &root in &island_of {
        let next = island_ids.len();
        island_ids.entry(root).or_insert(next);
    }
    let island_of: Vec<usize> = island_of.iter().map(|r| island_ids[r]).collect();
    let island_count = island_ids.len();
    let mut orientation = vec![0.0f64; island_count];
    for t in 0..tri_count {
        orientation[island_of[t]] += area3[t] * area_uv[t].signum();
    }

    let mut sheet_area3 = vec![0.0f64; sheet_count];
    let mut sheet_area_uv = vec![0.0f64; sheet_count];
    for t in 0..tri_count {
        sheet_area3[sheet_of[t]] += area3[t];
        sheet_area_uv[sheet_of[t]] += area_uv[t].abs();
    }
    // Global UV-to-3D area ratio per sheet.
    let scale: Vec<f64> = (0..sheet_count)
        .map(|s| {
            if sheet_area3[s] > 0.0 {
                sheet_area_uv[s] / sheet_area3[s]
            } else {
                0.0
            }
        })
        .collect();

    Some(Layout {
        sheet_of,
        sheet_count,
        island_of,
        island_count,
        area3,
        area_uv,
        sigma,
        orientation,
        scale,
        boundary,
        seam_length,
    })
}

/// Measures UV channel `channel`; `None` if the mesh doesn't have it.
pub fn uv_metrics(mesh: &Mesh, channel: usize, raster_resolution: u32) -> Option<UvMetrics> {
    let uvs = mesh.uvs.get(channel)?;
    let l = layout(mesh, channel)?;
    let tri_count = mesh.triangle_count();
    let uv = |v: u32| {
        let t = uvs[v as usize];
        DVec2::new(f64::from(t.x), -f64::from(t.y))
    };
    let tri = |t: usize| -> [u32; 3] { [0, 1, 2].map(|k| mesh.indices[t * 3 + k]) };
    let Layout {
        sheet_of,
        sheet_count,
        island_of,
        island_count,
        area3,
        area_uv,
        sigma,
        orientation,
        scale,
        boundary,
        seam_length,
    } = &l;
    let (sheet_count, island_count) = (*sheet_count, *island_count);
    let mirrored_islands = orientation.iter().filter(|&&o| o < 0.0).count();
    let total_area3: f64 = area3.iter().sum();
    let mut sheet_area_uv = vec![0.0f64; sheet_count];
    for t in 0..tri_count {
        sheet_area_uv[sheet_of[t]] += area_uv[t].abs();
    }

    let mut flipped = 0;
    let mut zero_area = 0;
    let mut weighted_angles: Vec<(f64, f64)> = Vec::with_capacity(tri_count);
    let (mut angle_sum, mut area_dist_sum, mut l2_sum, mut measured_area) = (0.0, 0.0, 0.0, 0.0);
    for t in 0..tri_count {
        match l.state(t) {
            TriangleState::Degenerate => continue,
            TriangleState::ZeroArea => {
                zero_area += 1;
                continue;
            }
            TriangleState::Flipped => flipped += 1,
            TriangleState::Good => {}
        }
        let s = scale[sheet_of[t]];
        let (s1, s2) = sigma[t];
        let w = area3[t];
        let angle = s1 / s2;
        angle_sum += w * angle;
        weighted_angles.push((angle, w));
        area_dist_sum += w * (s1 * s2 / s).log2().abs();
        l2_sum += w * (s1 * s1 + s2 * s2) / 2.0 / s;
        measured_area += w;
    }
    let mean = |sum: f64| {
        if measured_area > 0.0 {
            sum / measured_area
        } else {
            0.0
        }
    };

    // Island texel density against the sheet's.
    let mut island_area3 = vec![0.0f64; island_count];
    let mut island_area_uv = vec![0.0f64; island_count];
    let mut island_sheet = vec![0usize; island_count];
    for t in 0..tri_count {
        island_area3[island_of[t]] += area3[t];
        island_area_uv[island_of[t]] += area_uv[t].abs();
        island_sheet[island_of[t]] = sheet_of[t];
    }
    let (mut density_sum, mut density_max) = (0.0f64, 0.0f64);
    for i in 0..island_count {
        let s = scale[island_sheet[i]];
        if island_area3[i] <= 0.0 || s <= 0.0 {
            continue;
        }
        let deviation = ((island_area_uv[i] / island_area3[i] / s).sqrt() - 1.0).abs();
        density_sum += island_area3[i] * deviation;
        density_max = density_max.max(deviation);
    }

    let (mut straight_len, mut boundary_len) = (0.0, 0.0);
    for &(_, a, b) in boundary {
        let d = uv(b) - uv(a);
        let len = d.length();
        boundary_len += len;
        if len > 0.0 && d.x.abs().min(d.y.abs()) / len < STRAIGHT_SIN {
            straight_len += len;
        }
    }

    let out_of_bounds = (0..tri_count)
        .filter(|&t| {
            tri(t).iter().any(|&v| {
                let p = uvs[v as usize];
                !(-1e-4..=1.0 + 1e-4).contains(&p.x) || !(-1e-4..=1.0 + 1e-4).contains(&p.y)
            })
        })
        .count();

    let overlap_texels = overlaps(mesh, uvs, sheet_of, sheet_count, raster_resolution);

    Some(UvMetrics {
        channel,
        sheets: sheet_count,
        islands: island_count,
        flipped_triangles: flipped,
        zero_area_triangles: zero_area,
        mirrored_islands,
        overlap_texels,
        raster_resolution,
        out_of_bounds_triangles: out_of_bounds,
        angle_distortion_mean: mean(angle_sum),
        angle_distortion_p95: weighted_percentile(&mut weighted_angles, 0.95),
        area_distortion_mean: mean(area_dist_sum),
        l2_stretch: mean(l2_sum).sqrt(),
        seam_length: if total_area3 > 0.0 {
            seam_length / total_area3.sqrt()
        } else {
            0.0
        },
        straight_edge_ratio: if boundary_len > 0.0 {
            straight_len / boundary_len
        } else {
            0.0
        },
        density_deviation_mean: if total_area3 > 0.0 {
            density_sum / total_area3
        } else {
            0.0
        },
        density_deviation_max: density_max,
        packing_efficiency: sheet_area_uv.iter().sum::<f64>() / sheet_count as f64,
    })
}

/// Counts texels whose centre lies strictly inside more than one triangle of
/// the same sheet.
fn overlaps(
    mesh: &Mesh,
    uvs: &[glam::Vec2],
    sheet_of: &[usize],
    sheet_count: usize,
    resolution: u32,
) -> usize {
    let res = resolution as usize;
    let mut hits = vec![0u8; res * res];
    (0..sheet_count)
        .map(|sheet| {
            coverage(mesh, uvs, sheet_of, sheet, resolution, &mut hits);
            hits.iter().filter(|&&h| h >= 2).count()
        })
        .sum()
}

/// Fills `hits` (row-major, `resolution`², row 0 at V = 0) with how many
/// triangles of `sheet` cover each texel centre, saturating at 255. Centres
/// exactly on a shared edge count for neither side, so neighbouring triangles
/// never register as overlapping.
pub(crate) fn coverage(
    mesh: &Mesh,
    uvs: &[glam::Vec2],
    sheet_of: &[usize],
    sheet: usize,
    resolution: u32,
    hits: &mut [u8],
) {
    let res = resolution as usize;
    hits.fill(0);
    for (t, corners) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        if sheet_of[t] != sheet {
            continue;
        }
        let p = corners.map(|v| {
            let uv = uvs[v as usize];
            DVec2::new(f64::from(uv.x), f64::from(uv.y)) * resolution as f64
        });
        let doubled = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if doubled == 0.0 {
            continue;
        }
        let lo = p[0].min(p[1]).min(p[2]);
        let hi = p[0].max(p[1]).max(p[2]);
        let x0 = (lo.x - 0.5).ceil().max(0.0) as usize;
        let y0 = (lo.y - 0.5).ceil().max(0.0) as usize;
        let x1 = ((hi.x - 0.5).floor() as i64).min(res as i64 - 1);
        let y1 = ((hi.y - 0.5).floor() as i64).min(res as i64 - 1);
        if x1 < 0 || y1 < 0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let c = DVec2::new(x as f64 + 0.5, y as f64 + 0.5);
                let w = [0, 1, 2].map(|k| edge_side(p[k], p[(k + 1) % 3], c) * doubled.signum());
                if w.iter().all(|&w| w > 0.0) {
                    let h = &mut hits[y * res + x];
                    *h = h.saturating_add(1);
                }
            }
        }
    }
}

/// Which side of edge `a → b` the point `c` is on. The arithmetic depends only
/// on the unordered edge, so the two triangles sharing it get exactly opposite
/// signs and a point on the edge is never strictly inside both.
fn edge_side(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    if (a.x, a.y) < (b.x, b.y) {
        (b - a).perp_dot(c - a)
    } else {
        -(a - b).perp_dot(c - b)
    }
}

/// Value below which `p` of the total weight lies.
fn weighted_percentile(values: &mut [(f64, f64)], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = values.iter().map(|v| v.1).sum();
    let mut acc = 0.0;
    for &(v, w) in values.iter() {
        acc += w;
        if acc >= p * total {
            return v;
        }
    }
    values[values.len() - 1].0
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use glam::Vec2;

    /// A 1×1 grid whose UVs are exactly its XZ coordinates (+0.5), so the map is the identity.
    fn identity_grid() -> Mesh {
        primitives::grid(1.0, 1.0, 8, 8)
    }

    #[test]
    fn identity_map_is_perfect() {
        let m = uv_metrics(&identity_grid(), 0, 256).unwrap();
        assert_eq!(m.islands, 1);
        assert_eq!(m.flipped_triangles, 0);
        assert_eq!(m.zero_area_triangles, 0);
        assert_eq!(m.mirrored_islands, 0);
        assert_eq!(m.overlap_texels, 0);
        assert_eq!(m.out_of_bounds_triangles, 0);
        assert!((m.angle_distortion_mean - 1.0).abs() < 1e-6);
        assert!((m.angle_distortion_p95 - 1.0).abs() < 1e-6);
        assert!(m.area_distortion_mean.abs() < 1e-6);
        assert!((m.l2_stretch - 1.0).abs() < 1e-6);
        assert!(m.density_deviation_max < 1e-6);
        assert!((m.straight_edge_ratio - 1.0).abs() < 1e-9);
        assert!((m.packing_efficiency - 1.0).abs() < 1e-6);
        assert_eq!(m.seam_length, 0.0);
    }

    #[test]
    fn uniform_scale_is_not_distortion() {
        let mut mesh = identity_grid();
        for uv in &mut mesh.uvs[0] {
            *uv *= 0.5;
        }
        let m = uv_metrics(&mesh, 0, 256).unwrap();
        assert!((m.angle_distortion_mean - 1.0).abs() < 1e-6);
        assert!(m.area_distortion_mean.abs() < 1e-6);
        assert!((m.l2_stretch - 1.0).abs() < 1e-6);
        assert!((m.packing_efficiency - 0.25).abs() < 1e-6);
    }

    #[test]
    fn stretch_shows_as_angle_and_l2_distortion() {
        let mut mesh = identity_grid();
        for uv in &mut mesh.uvs[0] {
            uv.x *= 0.5; // squash U by 2×
        }
        let m = uv_metrics(&mesh, 0, 256).unwrap();
        assert!((m.angle_distortion_mean - 2.0).abs() < 1e-6);
        assert!((m.angle_distortion_p95 - 2.0).abs() < 1e-6);
        // Uniform squash is one global scale, so no area distortion.
        assert!(m.area_distortion_mean.abs() < 1e-6);
        // σ = (2, 1)/√2 after normalization: sqrt((2 + 0.5) / 2).
        assert!((m.l2_stretch - 1.25f64.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn a_folded_triangle_is_a_flip_and_an_overlap() {
        let mut mesh = identity_grid();
        // Pull an interior vertex across its neighbours so one triangle folds over.
        let target = mesh.uvs[0]
            .iter()
            .position(|uv| (*uv - Vec2::new(0.5, 0.5)).length() < 1e-6)
            .unwrap();
        mesh.uvs[0][target] = Vec2::new(0.5 + 0.2, 0.5 + 0.2);
        let m = uv_metrics(&mesh, 0, 256).unwrap();
        assert!(m.flipped_triangles >= 1, "{m:?}");
        assert!(m.overlap_texels > 0);
    }

    #[test]
    fn mirrored_island_is_reported_but_not_a_flip() {
        let mut mesh = identity_grid();
        for uv in &mut mesh.uvs[0] {
            uv.x = 1.0 - uv.x;
        }
        let m = uv_metrics(&mesh, 0, 256).unwrap();
        assert_eq!(m.mirrored_islands, 1);
        assert_eq!(m.flipped_triangles, 0);
    }

    #[test]
    fn cube_faces_are_separate_straight_islands() {
        let mut mesh = primitives::cube(1.0);
        // Lay the six faces out side by side so they don't overlap.
        let uvs = &mut mesh.uvs[0];
        for (i, uv) in uvs.iter_mut().enumerate() {
            let face = (i / 4) as f32;
            *uv = Vec2::new((uv.x + face) / 6.0, uv.y / 6.0);
        }
        let m = uv_metrics(&mesh, 0, 512).unwrap();
        assert_eq!(m.islands, 6);
        assert_eq!(m.overlap_texels, 0);
        assert!((m.straight_edge_ratio - 1.0).abs() < 1e-9);
        // 12 cube edges of length 1 over sqrt(6).
        assert!((m.seam_length - 12.0 / 6f64.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn stacked_cube_faces_overlap() {
        // The cube's own UVs put every face on the full 0–1 square.
        let m = uv_metrics(&primitives::cube(1.0), 0, 64).unwrap();
        assert!(m.overlap_texels > 0);
    }

    #[test]
    fn missing_channel_is_none() {
        assert!(uv_metrics(&identity_grid(), 1, 64).is_none());
    }
}
