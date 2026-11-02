<div align="center">

# LittleMesh

**Big meshes, little worries.**

A standalone app for getting 3D assets ready for real-time engines: optimize, retopo, UVs, LODs, and impostors in one place.

![status](https://img.shields.io/badge/status-early%20development-orange)
![license](https://img.shields.io/badge/license-MIT-blue)

</div>

---

## What is LittleMesh?

Every scene has heroes and background players. LittleMesh takes your heavy source meshes and turns them into engine-ready assets: cleaned up and optimized, retopologized into tidy geometry, unwrapped with clean UVs, and backed by the lighter stand-ins that take over when they're far from the camera. Those stand-ins are a chain of LODs that step down gracefully and impostors that fake the whole thing with a few triangles and a baked texture.

It's built for technical artists and environment artists who want production-ready assets without hand-tuning every one, and who'd rather not open a full DCC just to fix up a rock.

## Features

### Optimize
- **Mesh cleanup**: merge duplicate vertices and remove degenerate triangles, loose geometry, and unused material slots.
- **Render optimization**: reorder vertices and indices for better GPU cache use and less overdraw.
- **Mesh stats**: triangle, vertex, and material counts, with warnings for common problems before export.

### Auto Retopo
One-click retopology. No manual drawing or edge-loop tools, just a clean result you can ship.
- **Automatic rebuild**: turns scans, sculpts, and generated meshes into clean, evenly distributed geometry.
- **Budget slider**: set a triangle or polygon target and LittleMesh fits the surface to it.
- **Detail baking**: bakes normal and AO maps from the original high-poly onto the new mesh automatically.

### Auto UVs
Hands-off unwrapping that thinks like an artist. It isn't a UV editor, just great UVs without the busywork.
- **Shape-aware unwrapping**: detects what each part is (flat panels, boxes, quad strips, pipes, cones, bent tubes, caps, organic shapes) and unwraps each with a method built for it. Pipes and strips come out as straight rectangles.
- **Hidden seams**: seams go into creases, hard edges, and places nobody looks.
- **Repeated parts**: detects duplicates like bolts, with optional stacking to save texture space.
- **Auto packing**: packs islands efficiently, with padding matched to the target texture resolution.
- **Lightmap UVs**: generates non-overlapping second UV channels for baked lighting.

> Need fine control over seams or topology? LittleMesh plays nicely with dedicated tools: export, tweak there, and bring the mesh back for LODs and impostors.

### LOD generation
- **Automatic LOD chains**: generate LOD1–LODn from a single source mesh with per-level reduction targets (triangle %, triangle count, or screen size).
- **Silhouette-aware reduction**: preserves outlines, hard edges, and UV seams so LODs don't pop or smear textures.
- **Attribute preservation**: keeps normals, UVs, vertex colors, and material slots intact across levels.
- **Screen-size suggestions**: recommends switch distances based on how much each LOD actually changes on screen.

### Impostors
- **Octahedral and hemi-octahedral impostors**: full-sphere captures for props, upper-hemisphere captures for trees and ground-bound assets.
- **Billboard / cross-card impostors**: cheap fallbacks for foliage and distant clutter.
- **Baked texture sets**: base color, normal, depth, and mask/ORM atlases at configurable resolution and frame count.
- **Impostor as final LOD**: drop the impostor at the end of the LOD chain automatically.

### Workflow
- **Live preview**: orbit, zoom, and scrub through LODs side by side with wireframe, triangle count, and distance overlays.
- **Batch processing**: point it at a folder and process whole asset libraries with shared presets.
- **Engine presets**: export settings tuned for Unreal Engine, Unity, and Godot.
- **Standalone**: no DCC or engine install required.

## Supported formats

| Import | Export |
| --- | --- |
| FBX, OBJ, glTF / GLB, USD | FBX, glTF / GLB, USD, plus PNG / EXR texture atlases for impostors |

## Getting started

> LittleMesh is in early development. Installation instructions will be added with the first release.

### Building from source

Requires a recent stable [Rust](https://rustup.rs) toolchain.

```bash
cargo build --release
```

```bash
cargo run --release -p lm-cli -- stats path/to/model.glb
```

```bash
cargo run --release -p lm-cli -- process path/to/model.obj -o out/model.glb --lods
```

`--lods` on its own builds 50%, 25%, and 12.5% levels. You can also give your own targets, such as `--lods 30%,10%` or `--lods 5000,1000`.

To fetch the sample corpus (CC0 meshes from Poly Haven, about 80 MB) for testing, run this with [Node.js](https://nodejs.org):

```bash
node scripts/fetch-samples.mjs
```

### Planned workflow

```text
1. Open LittleMesh and drag in a mesh (or a folder of meshes).
2. Pick a preset, e.g. "Unreal – Prop" or "Unity – Foliage".
3. Tweak the LOD count, reduction targets, and impostor settings.
4. Preview, then hit Export.
```

## Presets

Presets bundle LOD and impostor settings so a whole library can be processed consistently.

| Preset | LODs | Impostor | Good for |
| --- | --- | --- | --- |
| Prop | 3 | Octahedral | Rocks, crates, set dressing |
| Foliage | 2 | Hemi-octahedral | Trees, bushes, large plants |
| Hero | 4 | None | Characters and close-up assets |
| Clutter | 1 | Billboard | Grass clumps, debris, small scatter |

Custom presets can be saved and shared as files.

## Roadmap

**v0.1**
- [ ] Mesh cleanup and render optimization
- [ ] Core LOD generation with screen-size suggestions
- [ ] Shape-aware automatic UV unwrapping, packing, and lightmap UVs
- [ ] Live preview: checker, UV layout, LOD scrubber
- [ ] Command-line mode for pipeline integration

**Later**
- [ ] Octahedral impostor baking
- [ ] One-click retopology with detail baking
- [ ] Batch processing and presets
- [ ] FBX and USD support
- [ ] Engine-specific export presets
- [ ] Unreal and Unity import helpers (auto-setup of LOD groups and impostor materials)

See [plan.md](plan.md) and [docs/milestones.md](docs/milestones.md) for details.

## Contributing

Feedback, bug reports, and feature ideas are welcome. Please open an issue describing your use case, and include a sample mesh if you can.

## License

LittleMesh is released under the [MIT License](LICENSE).
