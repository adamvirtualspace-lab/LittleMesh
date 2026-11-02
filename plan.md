# LittleMesh — Development Plan

> Big meshes, little worries.

This plan covers what LittleMesh is, what it deliberately isn't, how it's built, and the order to build it in. Anything marked **Decision** is still open.

Detailed docs:
- [docs/milestones.md](docs/milestones.md): every milestone broken into tasks with acceptance checks.
- [docs/algorithms.md](docs/algorithms.md): how each v0.1 stage works, with Auto UVs in depth.
- [docs/later.md](docs/later.md): research notes for impostors, retopo, and baking (after v0.1).

---

## 1. Goals

1. **Fast, lean v0.1:** ship a small, solid tool first and grow it afterwards.
2. **Auto UVs as the headline:** fully automatic unwrapping that detects what kind of shape each part is and uses a specialized method for it, the way an artist would.
3. **Automatic first:** every stage gives a good result with zero tweaking. Options exist for adjustment, not as the main workflow.
4. **Very easy to use and responsive UI:** a first-time user goes from drag-in to export without reading docs, and the app never freezes, even on a 1M-triangle mesh. The concrete rules are in [UI principles](docs/milestones.md#ui-principles).
5. **Portfolio-grade:** a clean, public codebase and docs that show technical-art and tools-engineering skill.

**Way of working:** Claude builds most of the code. You steer, review results, and test in your own tools.

## 2. Scope

### v0.1
| Stage | What it does |
| --- | --- |
| Import / export | glTF/GLB and OBJ in, glTF/GLB out |
| Optimize | Cleanup (weld, degenerates, unused data), vertex cache and overdraw optimization, stats and warnings |
| Auto UVs | Multi-strategy automatic unwrap (planar, box, quad strip, cylinder, cone, bent tube, cap, organic, repeated parts), packing, lightmap UV channel |
| LODs | Automatic LOD chain with screen-size suggestions |
| App + CLI | Tauri app with live preview (checker, UV layout, LOD scrubber) and a CLI on the same core |

### Later (v0.2+)
Impostors, auto retopo and baking, presets and batch, FBX/USD, engine presets and import helpers. The order is in [milestones.md](docs/milestones.md#later-v02).

### Out of scope (on purpose)
- Manual UV editing, seam painting, or a UV editor.
- Manual retopo tools such as drawing quads or edge loops.
- Sculpting, modeling, or texturing.
- Rigged or skinned mesh support.

## 3. Architecture

```
┌──────────────────────────────────────────────┐
│ App (Tauri 2)                                │
│  ├─ Panels and pipeline options (TS)         │
│  └─ Viewport: three.js                       │
├──────────────────────────────────────────────┤
│ lm-core (Rust)                               │
│  ├─ mesh/      mesh type, half-edge view     │
│  ├─ io/        glTF + OBJ import, glTF export│
│  ├─ optimize/  cleanup, render optimization  │
│  ├─ uv/        analysis, classify, strategies│
│  │             solvers, pack, metrics        │
│  ├─ lod/       simplification chain          │
│  └─ pipeline/  stages, progress, cancel      │
├──────────────────────────────────────────────┤
│ lm-cli (same core): stats, process, bench    │
└──────────────────────────────────────────────┘
```

**Decided:** Tauri 2 + Rust. C++ + Qt was the alternative, with easier access to some geometry libraries, but Rust is safer and closer to existing skills. C++ libraries come in through thin FFI bindings. The CLI is built before the app, so every stage can be tested headlessly.

### Libraries
| Area | Library | License | Notes |
| --- | --- | --- | --- |
| Simplification, cache optimization | meshoptimizer | MIT | Via the `meshopt` crate, or our own bindings if the crate lags behind |
| UV baseline and packing | xatlas | MIT | Default unwrap as the baseline; packer for our own charts |
| Sparse linear algebra | faer | MIT | LSCM, ARAP, and harmonic solves |
| glTF | `gltf` crate | MIT / Apache | Import and export |
| OBJ | `tobj` | MIT | Import |
| FBX | ufbx | MIT / public domain | Import, vendored like xatlas |
| CLI | `clap` | MIT / Apache | |

Verify every license before shipping, and keep a `THIRD_PARTY_LICENSES` file.

## 4. Milestones

| Milestone | Focus | Size |
| --- | --- | --- |
| M0 | Core + CLI foundations | M |
| M1 | Optimize + LODs | M |
| M2 | Auto UVs: foundation (xatlas baseline, metrics, solvers, organic unwrap) | L |
| M3 | App + viewer | L |
| M4 | Auto UVs: specialized strategies + tuning | XL |
| M5 | Release v0.1 (Windows) | M |

The tasks, acceptance checks, and what each milestone needs from you are in [docs/milestones.md](docs/milestones.md).

## 5. Risks and open questions

- **Auto UV quality is a taste problem.** Metrics catch flips, stretch, and seam length, but not "this seam looks wrong". Tuning (M4.7) needs your review, and it's the part most likely to run long.
- **Clean-room code:** write everything fresh. That means no code or assets from workplace tools, and no decompiling, reverse-engineering, or bundling other unwrappers (for example Ministry of Flat). Features can be inspired by public descriptions; implementations come from published research.
- **Crate gaps:** the `meshopt` crate may not expose every upstream feature, and xatlas's packer has to keep our charts intact. Both get checked first in their milestones, with our own bindings as the fallback.
- **Scope creep:** the out-of-scope list and the Later list are a contract. Revisit them before adding anything to v0.1.
- **Distribution:** **Decision.** Fully free, free core with paid extras, or paid. LittleMesh is MIT today, so re-check the license once this is settled.

## 6. Success criteria for v0.1

- Drop in a typical prop or a 1M-triangle scan, and get a cleaned mesh with Auto UVs, UV1 lightmap UVs, and 3 LODs as glTF, with no settings touched.
- Auto UVs: zero flips and overlaps on the whole corpus, and better than the xatlas baseline on angle distortion and visible seam length on at least 80% of it.
- Output opens cleanly in Blender, Unreal, and Unity with UVs and LODs intact.
