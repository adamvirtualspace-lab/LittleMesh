# LittleMesh — Algorithm Notes

How each v0.1 stage works, which library does the heavy lifting, and the math LittleMesh owns itself. The order things get built is in [milestones.md](milestones.md), and notes for stages that moved out of v0.1 (impostors, retopo, baking) are in [later.md](later.md).

Conventions used throughout: **Y-up, right-handed, meters** (the glTF convention).

---

## 1. Stage order (v0.1)

```
Import → Cleanup → [Auto UVs] → LOD chain → Render optimize (per LOD) → Export
```

- Auto UVs runs **before** LODs, so every LOD inherits the same UV layout. The simplifier preserves UVs and seams (§5).
- Every LOD level is built from LOD0, never from the previous level. Chaining compounds error.
- Render optimization runs last on each level, because simplification destroys any earlier vertex/index order.

## 2. Mesh representation

- Indexed triangle list with `u32` indices. Vertices are **split at attribute seams**: a corner with two UVs or two normals becomes two vertices.
- Attributes: position, normal, optional tangent, `uv[0..n]`, optional color.
- Submeshes are index ranges, each with a material slot.
- A **position-only remap** lives next to the mesh (meshoptimizer's `generateShadowIndexBuffer` style). Topology work uses it, because split vertices hide topology.
- A **half-edge view** is built on demand from the position-welded topology. Auto UVs needs it for adjacency, boundary loops, cutting along edges, and walking quad loops. Non-manifold edges are split before it's built, so every half-edge has at most one twin.

## 3. Cleanup (kept simple for v0.1)

| Step | Approach |
| --- | --- |
| Weld | Exact weld through meshoptimizer's vertex remap: merge vertices whose full attribute set is identical. |
| Degenerates | Drop triangles with repeated indices, and triangles with zero area. |
| Unused vertices | Compact the vertex buffer after index changes. |
| Unused materials | Drop slots with no triangles left and remap the submeshes. |

Warnings: non-manifold edges, missing UVs or normals (normals get generated), and extreme scale (bbox under 1 cm or over 1 km).

Moved to later: epsilon welding with a spatial hash, loose-part removal, and winding checks.

## 4. Render optimization (meshoptimizer)

Order per LOD, per submesh:

1. `optimizeVertexCache`
2. `optimizeOverdraw` with threshold `1.05`
3. `optimizeVertexFetch`

Report ACMR (average cache miss ratio) before and after, using `analyzeVertexCache`.

## 5. LOD simplification

- meshoptimizer's simplifier is a **quadric error metric (QEM)** edge collapser (Garland & Heckbert). Use the **attribute-aware** variant (`meshopt_simplifyWithAttributes`) with starting weights normals `0.5` and UVs `1.0`.
- Split vertices (§2) let the simplifier see UV and normal seams, so collapses keep seam edges together instead of tearing them.
- `LockBorder` is an option for open meshes whose edges must stay put. It's off by default.
- **Fallback ladder**, used only when a step stalls more than 10% above its target:
  1. Standard attribute-aware simplification.
  2. **Permissive:** collapses may cross UV and normal seams. Topology stays intact, but UVs can stretch near seams. Scans need this, since nearly every edge on them is a seam.
  3. **Sloppy:** `simplifySloppy` ignores topology but always reaches its target.

  Each LOD reports which method it used. On the M1 corpus run, 262 of 338 levels needed only the standard step, 76 needed permissive, and none needed sloppy.
- Submeshes are simplified separately with `ErrorAbsolute`. Vertices whose position is shared by more than one submesh are locked, so material borders can't crack.
- **Targets:** triangle % (`3 × round(tris × pct)` indices) or a triangle count. `result_error` × `meshopt_simplifyScale` gives the world-space error `e`.
- **Default chain:** 50%, 25%, 12.5% of LOD0. Stop early when a level saves under 15% of the previous level's triangles.

> **Checked in M1:** the `meshopt` 0.7 crate bundles meshoptimizer 1.3 and exposes `simplifyWithAttributes` with vertex locks, so we don't need our own bindings. Avoid its `optimize_vertex_fetch_remap`: it truncates the remap table to the used-vertex count, which breaks lookups. LittleMesh compacts vertices in first-use order itself.

## 6. Screen-size suggestions

Symbols: `r` is the bounding-sphere radius, `e` the LOD's world-space error, `H` the screen height in px, and `p_max` the allowed error in px (default 1 at 1080p).

```
switch screen size  s = 2 · r · p_max / (e · H)
```

`s` is the sphere's diameter as a fraction of screen height. It doesn't depend on the camera's FOV, so LittleMesh can suggest switch points without knowing the game camera. Example: `r = 1 m` and `e = 1 cm` give `s ≈ 0.185` at 1080p.

Derivation: pixel error is `p = e·H / (2·d·tan(θ/2))`. Setting `p = p_max` gives the switch distance `d_switch`, and substituting that into `s = r / (d·tan(θ/2))` makes the FOV cancel.

---

## 7. Auto UVs

The headline feature of v0.1: **fully automatic unwrapping that detects what kind of shape each part is and uses a specialized method for it**, the way an artist would. A pipe gets unrolled into a straight strip, a box gets folded out, a quad ribbon becomes a clean rectangle, and organic shapes get seams tucked where nobody looks.

> **Clean-room rule.** This feature set is inspired by what automatic unwrappers such as Ministry of Flat publicly describe: detecting many topology types, hiding seams, straightening islands, and hard-edge separation. Every algorithm here comes from published research or first principles. We never decompile, reverse-engineer, or bundle another tool, and we don't use its name in LittleMesh's marketing.

### 7.1 What "good" means (measurable)

| Quality | Measure | Rule |
| --- | --- | --- |
| No flipped triangles | `det J < 0` count | **Must be 0** |
| No overlaps (unless stacking is requested) | Raster overlap test | **Must be 0** |
| Low angle distortion | σ₁/σ₂ (§7.8) | Area-weighted mean < 1.1, p95 < 1.5 |
| Low area distortion | \|log₂(σ₁σ₂ / scale)\| | Mean < 0.25 |
| Few, hidden seams | Seam length weighted by visibility | Lower is better; compared against the baseline |
| Straight islands | Fraction of island boundary that's axis-aligned | Higher is better |
| Even texel density | Per-island density vs global | Within ±10%, except deliberate stretch |
| Good packing | UV area used / atlas area | Not worse than the baseline |

### 7.2 Pipeline

```
Analyze → Segment into parts → Classify each part → Unwrap with that part's strategy
       → Quality gate (split and retry, or fall back to Organic) → Straighten + scale → Pack → Validate
```

### 7.3 Analysis

- **Quad recovery.** Many inputs are triangulated quads, and quad structure is the strongest hint for strips and grids. Pair triangles greedily by best score. A pair qualifies when:
  - the shared edge is the longest edge of both triangles,
  - the dihedral angle is under 10°,
  - the resulting quad is convex with corner angles within 90° ± 25°.

  The original triangles are kept for output; quads are only an analysis view.
- **Hard edges:**
  - dihedral angle above a threshold (default 60°), or
  - an existing normal split in the input. That's an artist-authored hard edge, and the strongest signal we get.
- **Curvature:**
  - Gaussian curvature per vertex is the angle defect (`2π − Σ angles`). Near zero means the surface unfolds without stretch (developable).
  - Mean curvature comes from the cotangent Laplacian.
- **Visibility** per vertex: the fraction of 32 cosine-weighted hemisphere rays that escape the mesh, using a BVH over the part's mesh. 0 is fully hidden (inside a crease), 1 is fully exposed.
  - An optional **downward bias** treats faces pointing down (the underside of a prop) as less visible.
- **Seam cost** per edge (lower means a better place for a seam):

  ```
  cost(e) = length(e) · (w_vis · visibility(e) + w_smooth · smoothness(e)) · material_factor(e)
  ```

  - `smoothness` is low on hard and concave edges (seams there read as natural breaks) and high on smooth convex areas.
  - `material_factor` is near 0 on material borders, which are free seams.
  - Weights start at `w_vis = 1`, `w_smooth = 0.5` and get tuned against the corpus.

### 7.4 Segmentation into parts

A **part** is a region that gets one strategy. The strategy may still cut it into several charts.

1. Connected components.
2. Material borders always split.
3. Hard-edge regions: split along hard edges **only where they form closed region boundaries**. A dangling hard edge doesn't split anything.
4. The box strategy (§7.6) merges over-split flat regions back together, so step 3 can be aggressive.

### 7.5 Classification

Features computed per part:

| Feature | How |
| --- | --- |
| Topology | Euler characteristic `χ = V − E + F`, boundary loop count `b`, genus `g = (2 − χ − b) / 2`. Disk = (g 0, b 1), annulus/tube = (g 0, b 2), closed = (g 0, b 0), handles = g > 0. |
| Flatness | Maximum angle between face normals and the area-weighted mean normal. |
| Cylinder fit | A cylinder's normals are all perpendicular to its axis, so the **axis is the eigenvector of the normal covariance matrix with the smallest eigenvalue**. Project positions onto the plane perpendicular to the axis, then fit a circle (algebraic Kåsa fit). Error = RMS residual / radius. |
| Cone fit | Same axis estimate as a first guess, then fit a radius that varies linearly along the axis. |
| Sphere fit | Algebraic least-squares sphere. Error = RMS residual / radius. |
| Quad regularity | Fraction of the area covered by recovered quads, and the fraction of interior vertices with valence 4. |
| Developability | Area-weighted mean \|angle defect\|. |
| Shape | PCA eigenvalues of positions: elongation and size. |

Classes, checked in priority order. Every class reports a confidence, and anything below confidence 0.6 falls through to the next class:

| Class | Test (starting thresholds) | Strategy |
| --- | --- | --- |
| **Repeated** | Shape signature matches an earlier part (§7.6) | Reuse that part's unwrap |
| **Planar** | Normal spread < 5° | Planar projection |
| **Box / prism** | Faces fall into a few normal clusters separated by hard edges | Fold-out (hinge unfolding) |
| **Quad strip / grid** | Quads ≥ 80%, interior valence-4 ≥ 90%, disk or annulus | Quad rectification |
| **Cylinder / tube** | Cylinder error < 2% of the radius, annulus topology | Seam + unroll |
| **Cone / frustum** | Cone error < 2%, annulus topology | Unroll to a sector, straightened when narrow |
| **Bent tube** | Annulus, elongated, low developability, not a straight cylinder | Harmonic band unroll |
| **Disk / cap** | Disk topology, normal spread < 45° | Planar projection or LSCM |
| **Developable** | Low developability, any topology | Cut to a disk, then LSCM + ARAP |
| **Organic** | Fallback | Hidden-seam cut graph, then ARAP |

That's 10 classes for v0.1. More can be added later (for example torus/ring, stair-step or extrusion profiles, and branching tubes for trees), each as a new class plus strategy.

### 7.6 Strategies

**Planar projection.** Build a frame from the fitted plane normal. Rotate so the longest straight boundary edge is horizontal. Distortion is zero when the part is exactly planar.

**Box fold-out (hard-surface).**
1. Start with one planar chart per normal cluster.
2. Greedily merge neighbouring charts across their shared edge by rotating the neighbour flat around that edge (a hinge). This is isometric, so there's no distortion.
3. Accept a merge only when the result doesn't overlap itself and stays compact: bounding-rectangle fill ratio ≥ 0.6.
4. Order candidate hinges by **highest** seam cost first, so the edges kept as cuts are the cheap, hidden ones (bottom faces, inside corners).

The result is the cross or strip layout an artist would make for a box.

**Quad rectification.**
1. Walk quad loops with a BFS over quads, giving each vertex integer grid coordinates `(i, j)`. Reject the part (fall through to the next class) if the walk finds an inconsistency, which means a singular vertex inside.
2. Set column and row positions from cumulative **average** edge lengths, so rectangles keep their real proportions.
3. Run a light ARAP relax with the boundary held rectilinear.

This is an automatic "follow quads". Trims, straps, rails, and ribbons come out as perfect straight rectangles.

**Cylinder / tube unroll.**
1. **Seam:** run Dijkstra from boundary loop A to boundary loop B over edges weighted by `cost(e)`, plus a penalty for edges that deviate from the axis direction. That gives the least-visible straight-ish line.
2. Cut along it, then set `u = θ · r` (angle around the axis from the seam, times the radius) and `v` = position along the axis.
3. The island is a straight rectangle. As an option, orient `v` along world up, which suits tiling textures such as bark or pipes.

**Cone / frustum.** Unroll exactly to an annular sector, using slant distance and angle × sin(half-angle). If the sector angle is under about 60°, straighten it to a rectangle and accept the small, controlled stretch. Straight islands pack and texture better, a trade-off unwrappers like Ministry of Flat also make on purpose.

**Bent tube (pipes, branches, limbs, cables).**
1. Cut a seam as for the cylinder, but with no axis penalty. Seam cost alone decides.
2. Solve a **harmonic field** `v`: the Laplace equation with `v = 0` on loop A and `v = L` on loop B, where `L` is the mean geodesic length between the loops. It's smooth, with no local extrema.
3. Solve `u` with LSCM while `v` is held fixed. The result is a straight band even when the tube bends.
4. Run a short ARAP pass to even out area.

**Disk / cap.** Use planar projection when the cap is nearly flat, otherwise free-boundary LSCM.

**Developable and organic (hidden-seam cut graph).**
1. Find **cone points**: vertices with large accumulated angle defect, which are the spots that can't flatten without a cut (the tips of ears and fingers, the corners of a blob).
2. Connect them with an approximate minimum Steiner tree: an MST over pairwise shortest paths, using `cost(e)` as the weight. Seams naturally route through creases and hidden areas, in the spirit of Seamster (Sheffer & Hart 2002).
3. For closed parts, the tree must reach at least two points. For genus g > 0, add `2g` handle loops (tree–cotree on the dual graph, picking the cheapest loops).
4. Cut, initialize with LSCM, then refine with ARAP (§7.7).
5. **Quality gate:** if distortion exceeds §7.1, find the highest-distortion triangle, cut the cheapest path from it to the existing seam or boundary, and solve again. Allow at most 8 iterations, then accept and warn.

**Repeated parts.**
- **Signature:** triangle count, vertex count, surface area, and the sorted PCA eigenvalues.
- Confirm candidate matches with a rigid alignment (PCA frame plus a few ICP iterations, residual < 0.1% of the size).
- Unwrap the first part, then copy its UVs through the vertex correspondence.
- By default the copies get **separate** islands, because lightmaps and baking need unique UVs.
- `stack_repeats` overlaps them to save texture space. Mirrored matches stack only with `allow_mirror`, and a warning, since mirrored islands flip tangent space.

### 7.7 Solvers

| Solver | Used for | Notes |
| --- | --- | --- |
| **LSCM** (Lévy et al. 2002) | Init for most charts, disk caps, tube `u` | A sparse linear least-squares problem with two pinned vertices (the farthest boundary pair). Fast and conformal, but area can drift on long charts. |
| **ARAP** (Liu et al. 2008) | Refinement | Local step: best 2×2 rotation per triangle via SVD. Global step: a cotangent-Laplacian solve. The matrix is constant, so **factorize once** and iterate 10–50 times. |
| **Harmonic** | Bent-tube `v` | Laplacian with Dirichlet boundary values. |

- Linear algebra comes from **faer** (pure Rust, MIT): sparse Cholesky and LU.
- **Flip handling:** after solving, flipped triangles get a local relax. Any that remain trigger a cut through them. Final validation rejects the chart if flips survive.
- **Later:** SLIM (Rabinovich et al. 2017) for guaranteed flip-free maps on hard organic cases.

### 7.8 Distortion metrics

For each triangle, take the Jacobian `J` of the map from the 3D triangle (in its own 2D frame) to UV, with singular values `σ₁ ≥ σ₂`:

- **Angle distortion:** `σ₁ / σ₂`, where 1 is perfectly conformal.
- **Area distortion:** `log₂(σ₁σ₂ / s)`, where `s` is the global UV-to-3D area ratio and 0 is perfect.
- **L2 stretch** (Sander et al. 2001): `sqrt((σ₁² + σ₂²) / 2)`, area-weighted RMS.
- **Flip:** `det J` with the opposite sign to the triangle's island, whose orientation is the area-weighted majority. A wholly mirrored island isn't a flip; it's counted as `mirrored_islands`. Winding is measured with V pointing down, as glTF stores it. Triangles with 3D area but no UV area count alongside flips.
- **Overlap:** texels at resolution R whose centre is strictly inside two or more triangles of the same sheet. A shared edge is evaluated with identical arithmetic from both sides, so neighbours never register.
- Implemented in `lm-core/src/uv/metrics.rs`, per sheet (one material's 0–1 space) and combined weighted by 3D area.

### 7.9 Island post-processing

- **Straighten:** rotate each island to its minimum-area bounding rectangle (rotating calipers on the convex hull), so straight edges become horizontal or vertical. Strip, tube, and grid islands are already rectangles.
- **Texel density:** scale each island so its UV/3D area ratio matches the global ratio. The exception is the deliberate stretch some strategies allow, which stays capped by §7.1.
- **`separate_hard_edges`** option: forces a cut on every hard edge. It suits lightmaps and baking, at the cost of more islands.

### 7.10 Packing

- Padding `p = R / 256` px by default (4 px at 1024, 8 at 2048, 16 at 4096), which keeps mips clean down to 256 px. Adjustable as `island_gap`.
- Pack **per material** into its own 0–1 space by default. `single_sheet` packs everything together.
- v0.1 uses **xatlas's packer** on our own charts, through its UV-mesh input path. Rotation is limited to 90° steps for straightened islands, and free for organic ones.
- **Verified in M2.1:** each island keeps one uniform scale shared by all islands, and is only moved and rotated, *with one exception*. Stock xatlas stretches every chart separately along x and y so its extents land on whole texels: under 0.5% on big charts, up to about 33% on charts a few texels wide. Our vendored copy adds a `keepChartAspect` pack option that skips that stretch (`crates/lm-xatlas/vendor/xatlas/PATCHES.md`), and our own charts are packed with it on. Its `rotateChartsToAxis` rotates freely to the convex-hull axis, so straightened islands turn it off and keep only the 90° `rotateCharts`.
- **Later:** our own packer (a rectangle packer for straight islands plus raster bottom-left for the rest) and UDIM tiles.

### 7.11 Lightmap UVs (UV1)

- When UV0 has no stacked islands, **reuse the UV0 charts** and repack them for the lightmap: low resolution (64–512, set as an option) and at least 2 texels of padding at that resolution. That's nearly free.
- Otherwise run the unwrap again with `separate_hard_edges` forced on and stacking off.

### 7.12 Baseline and tuning

- **Baseline:** xatlas's default unwrap. It ships first (M2.2), so Auto UVs works from early on, and every strategy has to beat it.
- **Baseline notes from M2.2:**
  - xatlas drops every triangle with an absolute area under `FLT_EPSILON` (about 1.2·10⁻⁷) as degenerate, which on a 20 cm prop is every fine triangle. Each sheet is scaled to a total area of 10⁴ before it goes in; a uniform scale changes no UV shapes.
  - Its chart growing (the boundary-intersection checks) runs single-threaded per connected part and scales worse than linearly: about 5 s at 22k triangles, 21 s at 66k, 43 s at 100k, and 16.5 minutes on the 680k-triangle scan. That's fine for a yardstick but far off the §9 targets. From M2.5 on our own segmentation makes the charts and xatlas only packs them, which takes under a second here.
  - It ignores winding, so about half its charts come out mirrored. The metrics report this as `mirrored_islands`, separately from flips.
  - It can leave small self-overlaps inside a chart around one vertex without flipping any triangle. The raster check catches them.
  - **Corpus result (113 meshes, 1024 px):** 0 flips; 6 meshes with overlaps of 1–43 texels (boulder_01, coast_rocks_01, dead_tree_trunk, both crate lids, potted_plant_04_dirt). Against the artists' own UVs, median angle distortion is 1.033 vs 1.019, seams are about 1.7× longer (17.1 vs 10.4), islands 64 vs 24, and packing 51% vs 6% (the input figure is low because the modular kits share one atlas across many meshes). So the stock baseline doesn't meet the zero-overlap rule; our strategies have to.
- **Harness:** `littlemesh bench` runs the corpus and writes `<name>_uv.csv` next to the LOD CSV, one row per mesh. With `--uv` it measures Auto UVs; without it, the input file's own UVs, which makes a useful second reference. Columns:
  - distortion (angle, area, L2)
  - flips and overlaps
  - visibility-weighted seam length
  - island count
  - straight-edge ratio
  - packing efficiency
  - time
- **v0.1 bar:** with zero flips and overlaps, beat the baseline on angle distortion **and** visibility-weighted seam length on at least 80% of the corpus, without worse packing. On top of that comes your visual review.
- **Optional reference:** Blender's Smart UV Project, or any other unwrapper you own, compared by eye. Their outputs don't go into the repo or the tests.

## 8. Debug outputs and verification

| Output | What it shows |
| --- | --- |
| UV layout PNG (`--debug-uv`) | Islands filled by class colour, seams drawn, flips in red, overlaps in magenta |
| Class map glTF (`--debug-classes`) | The mesh with vertex colours by detected class |
| Checker glTF (`--debug-checker`) | The export with an embedded checker texture, for checking stretch and density |

Claude can view the PNGs directly and screenshot the checker glTF in Blender through the Blender MCP connection. Your eyes on the results stay the final check, especially for seam placement.

## 9. v0.1 performance targets

These are targets to measure, not known timings. Mid-range 8-core desktop.

| Input | Cleanup + LODs | Auto UVs |
| --- | --- | --- |
| 100k-triangle prop | < 1 s | < 5 s |
| 1M-triangle scan | < 5 s | < 60 s |

If Auto UVs on huge meshes misses its target, the first lever is running classification and seam search on a simplified proxy and transferring the cuts back.
