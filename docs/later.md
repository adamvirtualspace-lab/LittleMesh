# LittleMesh — Notes for Later Stages

Design notes for stages that moved out of v0.1: impostors, auto retopo, and baking. They're kept here so the research isn't lost. The v0.1 stages are in [algorithms.md](algorithms.md), and the order things get built is in [milestones.md](milestones.md).

Same conventions as the main notes: **Y-up, right-handed, meters**.

---

## 1. Impostors

### Capture setup
- Orthographic camera. The view extent is the bounding-sphere radius `r` (plus 2% margin), the same for every frame, so frames are scale-consistent.
- The camera sits at `center + dir × (r + margin)` and looks at `center`. The up vector is world Y, falling back to world Z when `dir` is near ±Y.
- One frame per direction in an `N × N` grid. Typical values: props `N = 12`, trees `N = 16` (hemi), atlas 2048–4096 px. The frame size is `atlas / N`. For example, 16 × 16 frames at 4096 px gives 256 px per frame.

### Octahedral mapping (full sphere), Y-up

Encode a unit direction `d` to `uv ∈ [-1, 1]²`:

```
p = d / (|d.x| + |d.y| + |d.z|)
if p.y >= 0:  uv = (p.x, p.z)
else:         uv = ((1 - |p.z|) · sgn(p.x), (1 - |p.x|) · sgn(p.z))
```

Decode `uv` to a direction:

```
v = (uv.x, 1 - |uv.x| - |uv.y|, uv.y)
if v.y < 0:   (v.x, v.z) = ((1 - |uv.y|) · sgn(uv.x), (1 - |uv.x|) · sgn(uv.y))
d = normalize(v)
```

`sgn(0)` must return `+1`, or directions on the axes collapse.

### Hemi-octahedral mapping (upper hemisphere)

For ground-bound assets the lower hemisphere is wasted, so map only `d.y ≥ 0`. Rotate the diamond 45° so it fills the whole square:

```
encode:  p = d / (|d.x| + |d.y| + |d.z|);   uv = (p.x + p.z, p.x - p.z)
decode:  x = (uv.x + uv.y) / 2;  z = (uv.x - uv.y) / 2;  y = 1 - |x| - |z|;  d = normalize(x, y, z)
```

### Frame sampling convention
Sample directions at grid **points** `uv = 2 · (i / (N-1)) - 1`, not at cell centres. Border frames then sit exactly on the horizon (hemi) or the fold lines (full), which are the views a player most often sees. The runtime shader **must use the same convention**. Store it in the impostor metadata.

### Baked channels
| Channel | Contents | Format |
| --- | --- | --- |
| Base color + alpha | Unlit albedo, with alpha as coverage | RGBA8 sRGB |
| Normal | **Object-space** normal. The impostor quad rotates with the camera, so tangent space has no meaning here. | RGB8, or RGB10A2 when banding shows |
| Depth | `(distance_along_view - (r + margin)) / r`, remapped to [0, 1] | R8 or R16 |
| ORM | Occlusion, roughness, metallic from the source material | RGB8 linear |

### Edge dilation
Mipmapping blends transparent texels into the edges, which gives dark or halo fringes. After capture, **dilate** colour, normal, and ORM outward into empty texels using a jump-flood fill or a few iterative passes. Leave alpha untouched.

### Runtime (engine shader)
1. Compute the view direction in **object space** and encode it to grid coordinates.
2. Find the grid cell, split it along a diagonal, and pick the triangle containing the point. That gives 3 frames with barycentric weights.
3. For each frame, **reproject** the current pixel into that frame's image plane using its depth channel. This parallax correction is what keeps blends from ghosting.
4. Blend the 3 samples by weight and discard or dither on alpha.

### Billboard and cross-card fallbacks
- **Billboard:** one camera-facing quad. The texture is a single capture or a small ring of `K` views around Y, selected by view angle.
- **Cross-card:** 2–3 quads intersecting at the pivot, each with its own orthographic capture along its normal. No shader tricks needed, which makes it the cheapest option for grass and clutter.

## 2. Auto retopo

### Candidates (evaluate all three before integrating)
| Backend | How it works | Output | Notes |
| --- | --- | --- | --- |
| **QuadriFlow** | Instant-Meshes-style orientation and position fields, plus min-cost flow to reduce singularities | Pure quads | MIT. Depends on Eigen. Take the code from the upstream repository, **not** from Blender's source tree (Blender is GPL). |
| **Instant Meshes** | Field-aligned remeshing: 4-RoSy orientation field, then position field, then quad-dominant extraction | Quad-dominant | BSD. The core needs separating from its nanogui GUI. Fast. |
| **Isotropic remesh** (own Rust) | Botsch–Kobbelt incremental remeshing: split long edges, collapse short ones, flip toward valence 6, tangential smoothing, all toward a target edge length | Even triangles | Pure Rust, no FFI. Engines render triangles anyway, so for background assets an even triangle mesh may be all that's needed. |

### Budget slider
- Triangle budget `T` maps to a target edge length `ℓ ≈ sqrt(4 · A / (√3 · T))`, where `A` is the surface area (equilateral-triangle area).
- For quad backends, target face count `≈ T / 2`. Triangulate the quads afterwards along their shorter diagonals.

### Preprocessing (scans and sculpts)
1. Pre-decimate to about 250k triangles with meshoptimizer before running the field solve. Field quality barely changes, and speed improves a lot.
2. Close small holes (boundary loops under N edges) with a simple fan or ear fill. Large holes are left alone and raise a warning.
3. Remove non-manifold parts the backend rejects, and report them.

### Known weak spots
Thin parts (a single quad layer collapses), sharp hard-surface edges, and very noisy scans. For hard-surface assets the preset should prefer the **simplify** path ([algorithms §5](algorithms.md#5-lod-simplification), run aggressively) over retopo. The README already promises "good enough for background assets", so keep that promise and nothing more.

## 3. Baking (normal and AO)

### Texel setup
1. Rasterize each low-poly triangle in **UV space** at the bake resolution, with conservative coverage so seam texels get samples. For each covered texel, interpolate the low-poly position, normal, and tangent frame.
2. Compute tangents with **MikkTSpace** (the `mikktspace` crate) on the final low-poly mesh. Unreal, Unity, and Godot all expect MikkTSpace, and a mismatched tangent basis shows up as visible shading seams. Export the tangents as well.

### Ray casting
- Cast rays from the low-poly surface point along the interpolated normal in **both directions**, within a max distance (auto = 2% of the bbox diagonal). Take the nearest hit on the high-poly mesh.
- Use an **averaged-normal cage** (offset along normals averaged across hard edges) as the ray origin. This avoids gaps and duplicated details at hard edges.
- Acceleration: a BVH over the high-poly mesh. Start with a CPU BVH (the `bvh` crate or our own SAH builder) parallelized with rayon. A wgpu compute path is a later optimization.

### Normal map
`n_tangent = TBNᵀ · n_high`, encoded to `[0, 1]`. The green channel convention differs by engine: **Unreal uses DirectX (Y−)**, while **Unity and Godot use OpenGL (Y+)**. Bake as Y+ and flip at export from the engine preset.

### AO
- Cast cosine-weighted hemisphere rays around the high-poly normal at the hit point, against the high-poly mesh, with a small bias. Default 64 samples, up to 256 for hero assets.
- Ray budget example: a 2048² map with about 70% coverage is ≈ 2.9M texels. At 64 samples that's ≈ 190M rays. A good multi-threaded CPU BVH gets through that in roughly 10–20 s, which fits a "1M triangles in under a minute" goal. Measure it rather than trusting this estimate.

### Post
Dilate both maps outward by at least the UV padding, so mips and bilinear filtering never sample background texels.

