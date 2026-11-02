# LittleMesh — Development Plan

> Big meshes, little worries.

This plan covers what LittleMesh is, what it deliberately isn't, how it's built, and the order to build it in. Anything marked **Decision** is still open.

---

## 1. Goals

1. **One-stop asset prep**: take a raw mesh in and get an engine-ready asset out (cleaned, retopologized, unwrapped, LODs, impostor).
2. **Automatic first**: every stage should produce a good result with zero tweaking. Sliders exist for adjustment, not as the main workflow.
3. **Batch-friendly**: process a whole folder with one preset as easily as a single mesh.
4. **Portfolio-grade**: clean, public codebase and docs that show technical-art and tools-engineering skill.

## 2. Scope

### In scope
| Stage | What it does |
| --- | --- |
| Optimize | Cleanup (weld, degenerate/loose removal), vertex cache and overdraw optimization, mesh stats and warnings |
| Auto Retopo | One-click remesh to a triangle/quad budget, plus normal/AO bake from the source |
| Auto UVs | Automatic seams, unwrap, packing, and lightmap UV channel |
| LODs | Automatic LOD chain with screen-size suggestions |
| Impostors | Octahedral, hemi-octahedral, and billboard bakes with texture atlases |

### Out of scope (on purpose)
- Manual UV editing, seam painting, or a UV editor (that's Cozy Blanket territory).
- Manual retopo tools such as drawing quads or edge loops.
- Sculpting, modeling, or texturing.
- Rigged or skinned mesh support in v1 (see Later).

## 3. Proposed architecture

```
┌──────────────────────────────────────────────┐
│ UI shell (Tauri)                             │
│  ├─ Panels, presets, batch queue (TS)        │
│  └─ Viewport: three.js / WebGPU              │
├──────────────────────────────────────────────┤
│ Core (Rust)                                  │
│  ├─ io/        import & export               │
│  ├─ optimize/  cleanup, cache opt            │
│  ├─ retopo/    remesh + bake                 │
│  ├─ uv/        unwrap, pack, lightmap UVs    │
│  ├─ lod/       simplification chain          │
│  ├─ impostor/  capture + atlas bake (wgpu)   │
│  └─ pipeline/  presets, job graph, batch     │
├──────────────────────────────────────────────┤
│ CLI (same core, no UI)                       │
└──────────────────────────────────────────────┘
```

**Why this stack:** it reuses the TS + three.js/WebGPU + Rust experience from Organic Machines, keeps heavy processing native and fast, and gets a CLI almost for free.

**Decision:** Tauri + Rust (proposed) or C++ + Qt. C++ has easier access to some geometry libraries; Rust is safer and closer to existing skills.

### Candidate libraries
| Area | Library | License | Notes |
| --- | --- | --- | --- |
| Simplification / cache opt | meshoptimizer | MIT | Industry standard, Rust bindings via `meshopt` |
| UV unwrap + pack | xatlas | MIT | Also handles lightmap UVs |
| Retopo | Instant Meshes / QuadriFlow | BSD / MIT | Evaluate both for quality and speed |
| glTF | `gltf` crate | MIT / Apache | Import and export |
| FBX import | ufbx | MIT / public domain | Read-only |
| USD | OpenUSD | Apache-style | Heavy dependency, could land in a later phase |
| GPU baking | wgpu | MIT / Apache | Impostor capture and normal/AO bakes |

Verify every license before shipping, and keep a `THIRD_PARTY_LICENSES` file.

## 4. Milestones

### M0 — Foundations
- [ ] Repo, CI, and app shell (Tauri window and empty viewport)
- [ ] glTF and OBJ import into the internal mesh format
- [ ] Viewport: orbit, wireframe, and stats overlay

### M1 — Optimize + LODs (first usable build)
- [ ] Cleanup pass (weld, degenerates, loose parts)
- [ ] Vertex cache and overdraw optimization
- [ ] LOD chain generation with %, triangle-count, or screen-size targets
- [ ] Side-by-side LOD preview with a distance scrubber
- [ ] glTF export with LODs

### M2 — Impostors
- [ ] Octahedral and hemi-octahedral capture with wgpu
- [ ] Atlases: base color, normal, depth, mask/ORM
- [ ] Billboard and cross-card fallback
- [ ] Impostor as the final LOD
- [ ] Reference impostor shaders/materials for Unreal and Unity

### M3 — Auto UVs + Auto Retopo
- [ ] xatlas integration: unwrap, pack, and lightmap channel
- [ ] Retopo backend evaluation, then integration
- [ ] Normal/AO bake from source to retopo mesh
- [ ] Before/after comparison view

### M4 — Pipeline
- [ ] Presets (Prop, Foliage, Hero, Clutter) that can be saved and shared
- [ ] Batch queue for folder in, folder out
- [ ] CLI mode with the same presets
- [ ] FBX import

### M5 — Release
- [ ] Engine export presets (Unreal, Unity, Godot)
- [ ] Docs site, sample assets, before/after gallery
- [ ] Installers (Windows first)
- [ ] Public v0.1 release

### Later
- USD import and export
- Unreal and Unity import helpers (auto-setup of LOD groups and impostor materials)
- Skinned mesh LODs
- Nanite-aware presets (fallback meshes)

## 5. Risks and open questions

- **FBX export**: ufbx can't write FBX, and the Autodesk FBX SDK has redistribution terms to check. Option: ship glTF/USD export first and add FBX later.
- **Retopo quality**: automatic retopo is hard on thin or hard-surface parts. Keep the promise to "good enough for background assets" and point to dedicated tools for hero work.
- **Impostor fidelity**: parallax and depth-based blending live in the engine shader, so reference materials matter as much as the bake.
- **Scope creep**: the out-of-scope list is a contract, so revisit it before adding any manual tool.
- **Clean-room code**: write everything fresh, with no code or assets taken from workplace tools.
- **Distribution**: **Decision.** Fully free, free core with paid extras, or paid on Gumroad/itch. Re-check the license choice once this is settled.

## 6. Success criteria for v0.1

- Drop in a 1M-triangle scan and get a cleaned, retopo'd, UV'd mesh with 3 LODs and an impostor in under a minute, with no settings touched.
- Batch-process 100 props with one preset without crashing.
- Output imports cleanly into Unreal and Unity with correct LODs and working impostor materials.
