# LittleMesh — Milestones and Tasks

The lean v0.1 plan. Claude builds most of the code, and you steer, review, and test. Overview in [plan.md](../plan.md), how each stage works in [algorithms.md](algorithms.md), and research for stages that moved later in [later.md](later.md).

## v0.1 scope

**In:** glTF/OBJ import → cleanup and render optimization → **Auto UVs** (multi-strategy, the headline feature) → LOD chain → glTF export, in a Tauri app with a live preview and a CLI.

**Moved to Later:** impostors, presets and batch, auto retopo, baking, FBX import, USD, and engine import helpers. See [Later](#later-v02).

## How to read this

- **Size** describes work, not calendar time: **S** is a single sitting, **M** a few, **L** many, and **XL** means it needs several rounds of your feedback on results.
- **Done when** is the acceptance check. Wherever possible it's an automated test, so it keeps passing later.
- **Your part** is what each milestone needs from you. Your review time is the real pace-setter, because the code goes fast and judging results is what takes time.
- Task IDs (`M2.6`) are for commits, issues, and branch names.

## Overview

| Milestone | Focus | Size | You'll get |
| --- | --- | --- | --- |
| M0 | Core + CLI foundations | M | `littlemesh stats` and `process` on glTF/OBJ |
| M1 | Optimize + LODs | M | Cleaned meshes with LOD chains as glTF |
| M2 | Auto UVs: foundation | L | Working Auto UVs (baseline first, then our own organic and planar unwrap) |
| M3 | App + viewer | L | The desktop app: drag in, preview, checker, LOD scrubber, export |
| M4 | Auto UVs: specialized strategies | XL | Shape detection with a dedicated unwrap per shape |
| M5 | Release v0.1 | M | Windows installer, docs, gallery |

The CLI comes before the app on purpose. Claude can test every stage headlessly with the CLI, so the app is just a view on a core that already works.

---

## M0 — Core + CLI foundations

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M0.1 | Cargo workspace (`crates/lm-core`, `crates/lm-cli`), rustfmt, clippy, GitHub Actions CI on Windows and Linux (`test`, `clippy -D warnings`) | CI is green, and a clippy warning fails it | S |
| M0.2 | Core mesh type: split vertices, submeshes, materials, position remap, `validate()` ([algorithms §2](algorithms.md#2-mesh-representation)) | Unit tests cover valid meshes and each kind of invalid one | S |
| M0.3 | Half-edge view: adjacency, boundary loops, non-manifold splitting, Euler characteristic | Tests on a cube, a cylinder (two loops), a torus (genus 1), and a non-manifold case | M |
| M0.4 | Import glTF/GLB (`gltf` crate, flattened with world transforms) and OBJ (`tobj`) | Every corpus file imports, and a transformed node lands where it should | M |
| M0.5 | glTF/GLB export: meshes, materials, UV0/UV1, normals, embedded textures | Round-trips through our importer and opens correctly in Blender | M |
| M0.6 | CLI: `littlemesh stats <file>` and `littlemesh process <in> -o <out>` (`clap`) | Stats match Blender's numbers for 3 corpus meshes | S |
| M0.7 | Sample corpus: about 20 CC0 meshes covering hard-surface props, pipes and cables, quad-modelled trims, organic sculpts, scans, and foliage. Sources and licenses go in `samples/SOURCES.md`, and one command downloads them. | Download script works on a clean clone | S |

**Your part:** drop in a few meshes from your own work (ones you're allowed to share, or keep them local) so the corpus reflects real assets.

---

## M1 — Optimize + LODs

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M1.1 | Cleanup: exact weld, degenerate removal, unused vertices and materials ([algorithms §3](algorithms.md#3-cleanup-kept-simple-for-v01)) | Tests: duplicates merge, seam vertices survive, zero-area triangles are removed | S |
| M1.2 | Warnings: non-manifold edges, missing UVs or normals, extreme scale | Each warning has a test mesh that triggers it | S |
| M1.3 | Render optimization with meshoptimizer, plus ACMR before and after | ACMR improves on every corpus mesh | S |
| M1.4 | LOD chain with attribute-aware simplification, % or triangle-count targets, and the sloppy fallback. First check the `meshopt` crate's API and use our own bindings if needed. | A 3-level chain on every corpus mesh, each level within ±5% of its target or flagged | M |
| M1.5 | Screen-size suggestions ([algorithms §6](algorithms.md#6-screen-size-suggestions)) | Matches a hand calculation in tests | S |
| M1.6 | LOD export: sibling nodes named `<name>_LOD0..N` | Opens in Blender with all levels present | S |
| M1.7 | `littlemesh bench`: runs the corpus and writes a CSV of triangles, error, and timing (Auto UV columns get added in M2) | CI uploads the CSV as an artifact | S |

**Your part:** open a few exported LOD chains in Blender or your engine and say whether the reduction looks acceptable.

---

## M2 — Auto UVs: foundation

The goal is to get Auto UVs working end-to-end early, with the xatlas baseline first, and to build the measuring tools every later strategy is judged with.

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M2.1 | xatlas: compile the vendored upstream sources with `cc` behind a thin C shim, declared by hand in Rust (no `bindgen`, so no libclang). Verify its packer keeps our own charts intact. | Builds in CI, and a test packs hand-made charts without reshaping them | M |
| M2.2 | **Baseline Auto UVs** with xatlas's default unwrap and pack, and padding from the resolution | `process --uv` produces UVs with no flips or overlaps on the whole corpus | S |
| M2.3 | UV metrics and validation: angle, area, and L2 distortion, flips, overlaps, seam length, straight-edge ratio, packing efficiency ([algorithms §7.1, §7.8](algorithms.md#71-what-good-means-measurable)), added to `bench` | Metric unit tests on known maps (identity, 2× stretch, a flipped triangle) | M |
| M2.4 | Debug outputs: UV layout PNG, class-map glTF, checker glTF ([algorithms §8](algorithms.md#8-debug-outputs-and-verification)) | All three generate for a corpus mesh and look right | S |
| M2.5 | Analysis: quad recovery, hard edges, curvature, BVH visibility, seam cost ([§7.3](algorithms.md#73-analysis)) | Visibility and seam-cost heatmaps look right on 3 meshes (vertex-colour debug glTF) | M |
| M2.6 | Segmentation into parts ([§7.4](algorithms.md#74-segmentation-into-parts)) | Debug view shows sensible parts on hard-surface and organic corpus meshes | M |
| M2.7 | Solvers: LSCM, ARAP (factorize once), harmonic, with faer ([§7.7](algorithms.md#77-solvers)) | Tests: a flat patch maps without distortion, ARAP lowers area distortion against LSCM on a long strip | M |
| M2.8 | Planar, disk/cap, and organic strategies with the hidden-seam cut graph and quality gate ([§7.6](algorithms.md#76-strategies)) | Organic corpus meshes pass the §7.1 distortion rules with zero flips | L |
| M2.9 | Island post-processing (straighten, texel density, `separate_hard_edges`) and packing through xatlas ([§7.9, §7.10](algorithms.md#79-island-post-processing)) | Density within ±10% on every corpus mesh, no overlaps | M |
| M2.10 | Lightmap UVs on UV1 ([§7.11](algorithms.md#711-lightmap-uvs-uv1)) | Zero overlaps on UV1 across the corpus | S |

**Your part:** look at the UV layout PNGs and checker renders for a handful of meshes. Seam placement is a taste call, and your notes tune the seam-cost weights.

---

## M3 — App + viewer

### UI principles

The UI must be **very easy to use** and **responsive**. These rules are part of the acceptance check for every M3 task, and for any UI added later.

**Easy to use**
- **One obvious path:** drop a mesh, press **Process**, press **Export**. Defaults do the right thing, so nothing else is required.
- **Advanced options stay hidden** behind a collapsed "Options" section per stage. The main screen shows only what matters for the current step.
- **Plain words:** "Island gap", not "padding_px". Every option has a one-line tooltip and a sensible range.
- **Results speak:** the checker, UV layout, and LOD stats appear on their own after processing. Warnings say what's wrong and what to do about it.
- **Forgiving:** Undo/reset per option, and the last settings are remembered. Nothing destructive happens without an export.
- **Keyboard shortcuts** for the frequent actions (frame, wireframe, checker, next/previous LOD), shown in tooltips.

**Responsive**
- **The UI thread never waits on processing.** All core work runs on background threads, with progress events and a working **Cancel**.
- **Fast feedback:** clicks, toggles, and slider drags respond within **100 ms**, and the viewport keeps **60 fps** while orbiting a 1M-triangle mesh on a mid-range GPU.
- **Fast first look:** the mesh appears within about 1 s of dropping a typical prop, while stats and warnings stream in afterwards.
- **Live where it's cheap:** LOD and display changes preview instantly. Expensive steps (Auto UVs) show progress, and only results that are complete get swapped in.
- **Measured:** a perf check (scripted orbit plus option toggles on a 1M-triangle corpus mesh) runs before each release.

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M3.1 | Tauri 2 shell with an npm + Vite + TypeScript frontend; drag-and-drop and file dialogs | Dropping a file reaches the core | S |
| M3.2 | Binary mesh transfer from core to UI (interleaved vertex buffer and index buffer over Tauri IPC, no JSON) | A 1M-triangle mesh reaches the viewport in under 1 s | S |
| M3.3 | three.js viewport: orbit, grid, auto-frame, wireframe, stats overlay, warnings list | Corpus meshes display with correct stats | M |
| M3.4 | UV views: checker material on the model, a 2D UV layout panel, colour by detected class, seams highlighted | You can spot stretch and seams without leaving the app | M |
| M3.5 | Pipeline panel: toggle stages, edit options (LOD targets, UV resolution, island gap, `separate_hard_edges`), run with progress and cancel, export | The full process runs from the UI, and its output matches the CLI's | M |
| M3.6 | LOD preview: side-by-side mode and a distance scrubber driven by the suggested screen sizes | Scrubbing switches levels at the suggested sizes | M |
| M3.7 | Responsiveness pass: scripted perf check (orbit, toggles, sliders on a 1M-triangle mesh) and fixes | Meets every number in [UI principles](#ui-principles) | M |
| M3.8 | First-use test: you try a fresh build with no instructions, and anything confusing gets fixed | You get from drag-in to export without help, and every confusing spot is fixed | S |

**Status (first slice):**
- **Done:** M3.1–M3.3.
  - The app lives in `app/`, with the Rust side in `app/src-tauri` as workspace crate `lm-app`.
  - Meshes cross IPC as one raw buffer. A 680k-triangle scan is on screen 295 ms after opening.
  - The app's export is byte-identical to the CLI's for the same settings.
- **Started:**
  - M3.4: the checker material only.
  - M3.5: Process and Export with three options, but no progress events or Cancel yet.
  - M3.6: LOD buttons, but no side-by-side view or distance scrubber yet.

**Your part:** use the app on your own meshes and report what feels clunky or slow. UI polish is mostly taste, so your notes drive M3.8.

---

## M4 — Auto UVs: specialized strategies

Shapes get classified, each with a dedicated unwrap, then everything is tuned against the corpus. Each strategy lands on its own, behind the classifier, so the overall result improves step by step.

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M4.1 | Classifier: features and the 10 classes with confidence scores and fall-through ([§7.5](algorithms.md#75-classification)), plus a class colour in the debug views | Hand-labelled test parts (about 40 across the corpus) are classified correctly ≥ 85% of the time | L |
| M4.2 | Box fold-out (hinge unfolding, seams on the cheapest edges) | Box-like corpus parts come out as zero-distortion folded strips or crosses | M |
| M4.3 | Quad rectification (automatic follow-quads) | Quad trims and straps come out as straight rectangles with proportional rows | M |
| M4.4 | Cylinder/tube and cone unroll, with a hidden seam line | Pipes come out as straight rectangles with the seam on the least-visible side | M |
| M4.5 | Bent-tube harmonic band unroll | Bent pipes and branches come out as straight bands under the §7.1 distortion limits | M |
| M4.6 | Repeated-part detection; `stack_repeats` and `allow_mirror` options | Duplicated bolts in a test mesh are detected; stacking only on request | M |
| M4.7 | **Tuning pass:** adjust thresholds and seam-cost weights against `bench` and your reviews | Meets the v0.1 bar in [§7.12](algorithms.md#712-baseline-and-tuning): beats the baseline on ≥ 80% of the corpus | XL |

**Your part:** this milestone is where your eye matters most. Review the before/after (baseline vs ours) for each strategy as it lands.

---

## M5 — Release v0.1

| ID | Task | Done when | Size |
| --- | --- | --- | --- |
| M5.1 | Windows installer from the Tauri bundler; decide on code signing | A clean Windows VM installs and runs it | S |
| M5.2 | License audit: `THIRD_PARTY_LICENSES` and `cargo-deny` in CI | CI fails on a disallowed license | S |
| M5.3 | Docs: README getting started, CLI reference, an Auto UV options page | Someone new can go from install to export using the docs alone | M |
| M5.4 | Gallery: 6 before/after examples (baseline vs LittleMesh UVs, LOD chains) | Published in the README or docs | S |
| M5.5 | Changelog, v0.1 tag, GitHub release | v0.1 can be downloaded from GitHub Releases | S |

**Your part:** test the installer on your machine, choose the screenshots, and decide when it ships.

---

## Later (v0.2+)

In rough priority order. Design notes for the first three are in [later.md](later.md).

1. **Impostors:** octahedral and hemi-octahedral bakes with engine shaders.
2. **Auto retopo** plus normal/AO **baking**.
3. **Presets and batch:** Prop, Foliage, Hero, and Clutter presets; folder in, folder out.
4. More UV classes: torus/ring, extrusion profiles, and branching tubes (trees). Also UDIM tiles, our own packer, and SLIM for hard organic cases.
5. FBX import (ufbx), USD import and export.
6. Engine export presets (axes, units, normal-map green channel) and Unreal/Unity import helpers.
