//! `littlemesh`: the LittleMesh command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lm_core::lod::{LodSettings, LodTarget, SimplifyMethod};
use lm_core::pipeline::{self, ProcessOptions, ProcessReport};
use lm_core::stats::{SceneStats, scene_stats};
use lm_core::uv::debug::{self, DebugMesh};
use lm_core::uv::{UvMetrics, UvSettings, uv_metrics};

#[derive(Parser)]
#[command(name = "littlemesh", version, about = "Big meshes, little worries.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print mesh statistics: counts, bounds, and topology.
    Stats {
        /// glTF/GLB or OBJ file.
        input: PathBuf,
        /// Print machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Clean up and optimize a mesh, optionally with LODs, and write glTF/GLB.
    Process {
        /// glTF/GLB or OBJ file.
        input: PathBuf,
        /// Output .glb or .gltf file.
        #[arg(short, long)]
        output: PathBuf,
        /// Generate LODs. Without a value: 50%,25%,12.5%. Targets are percentages
        /// of LOD0 ("30%") or triangle counts ("5000").
        #[arg(long, value_name = "TARGETS", num_args = 0..=1, default_missing_value = DEFAULT_LODS)]
        lods: Option<String>,
        /// Replace UV0 with automatic UVs (xatlas baseline for now).
        #[arg(long)]
        uv: bool,
        /// Texture resolution the UVs are packed for; sets island padding.
        #[arg(long, value_name = "PX", default_value_t = 1024)]
        uv_resolution: u32,
        /// Also write a UV layout PNG per mesh and material, `<output>.uv.*.png`.
        #[arg(long)]
        debug_uv: bool,
        /// Also write the meshes coloured by UV class, `<output>.classes.glb`.
        #[arg(long)]
        debug_classes: bool,
        /// Also write the meshes with a checker texture, `<output>.checker.glb`.
        #[arg(long)]
        debug_checker: bool,
        /// Skip welding, degenerate removal, and unused-data removal.
        #[arg(long)]
        no_cleanup: bool,
        /// Skip vertex cache, overdraw, and fetch optimization.
        #[arg(long)]
        no_optimize: bool,
        /// Also write the processing report as JSON.
        #[arg(long, value_name = "FILE")]
        report: Option<PathBuf>,
    },
    /// Process every model in a folder with default LODs and write a CSV of the results.
    Bench {
        /// Folder searched recursively for .gltf, .glb, and .obj files.
        dir: PathBuf,
        /// CSV output file. UV metrics go next to it as `<name>_uv.csv`.
        #[arg(short, long, default_value = "bench.csv")]
        output: PathBuf,
        /// Run Auto UVs and measure them. Without it, the input UVs are measured.
        #[arg(long)]
        uv: bool,
    },
}

const DEFAULT_LODS: &str = "50%,25%,12.5%";

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Stats { input, json } => {
            let import = load(&input)?;
            let stats = scene_stats(&import.scene);
            if json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                print_stats(&stats);
            }
        }
        Command::Process {
            input,
            output,
            lods,
            uv,
            uv_resolution,
            debug_uv,
            debug_classes,
            debug_checker,
            no_cleanup,
            no_optimize,
            report,
        } => {
            anyhow::ensure!(
                (64..=16384).contains(&uv_resolution),
                "--uv-resolution must be between 64 and 16384"
            );
            let options = ProcessOptions {
                cleanup: !no_cleanup,
                optimize: !no_optimize,
                uvs: uv.then(|| UvSettings {
                    resolution: uv_resolution,
                    ..Default::default()
                }),
                lods: lods.as_deref().map(parse_lods).transpose()?,
                debug: debug_uv || debug_classes || debug_checker,
            };
            let import = load(&input)?;
            let (scene, mut process_report) = pipeline::process(import.scene, &options);
            lm_core::io::save(&scene, &output)
                .with_context(|| format!("writing {}", output.display()))?;
            print_process_report(&process_report);
            if let Some(path) = report {
                std::fs::write(&path, serde_json::to_string_pretty(&process_report)?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            let stats = scene_stats(&scene);
            println!(
                "\nwrote {} ({} meshes, {} triangles)",
                output.display(),
                stats.meshes.len(),
                stats.total_triangles
            );
            let debug_meshes: Vec<DebugMesh> = process_report
                .meshes
                .iter_mut()
                .filter_map(|m| m.debug.take())
                .collect();
            let mut written = Vec::new();
            if debug_uv {
                written.extend(write_layouts(
                    &output,
                    &debug_meshes,
                    uv_resolution.min(4096),
                )?);
            }
            for (wanted, kind, scene) in [
                (
                    debug_classes,
                    "classes",
                    debug::class_scene as fn(&[DebugMesh]) -> _,
                ),
                (debug_checker, "checker", debug::checker_scene),
            ] {
                if wanted {
                    let path = sibling(&output, &[kind], &extension(&output));
                    lm_core::io::save(&scene(&debug_meshes), &path)
                        .with_context(|| format!("writing {}", path.display()))?;
                    written.push(path);
                }
            }
            for path in written {
                println!("wrote {}", path.display());
            }
        }
        Command::Bench { dir, output, uv } => bench(&dir, &output, uv)?,
    }
    Ok(())
}

/// `out/rock.glb` + ["uv", "Body"] + "png" → `out/rock.uv.Body.png`.
fn sibling(output: &Path, parts: &[&str], extension: &str) -> PathBuf {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let parts: Vec<String> = parts
        .iter()
        .map(|p| {
            p.chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        })
        .collect();
    output.with_file_name(format!("{stem}.{}.{extension}", parts.join(".")))
}

fn extension(path: &Path) -> String {
    path.extension()
        .map_or("glb".into(), |e| e.to_string_lossy().to_ascii_lowercase())
}

/// Writes one layout PNG per mesh and sheet. The mesh name is left out for a
/// single mesh, and the material for a single sheet.
fn write_layouts(output: &Path, meshes: &[DebugMesh], size: u32) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for d in meshes {
        let images = debug::layout_pngs(d, size);
        for image in &images {
            let mut parts = vec!["uv".to_string()];
            if meshes.len() > 1 {
                parts.push(d.mesh.name.clone());
            }
            if images.len() > 1 {
                parts.push(image.material.map_or("default".into(), |m| format!("m{m}")));
            }
            let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
            let path = sibling(output, &parts, "png");
            std::fs::write(&path, &image.png)
                .with_context(|| format!("writing {}", path.display()))?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Parses "50%,25%,5000" into LOD targets.
fn parse_lods(spec: &str) -> Result<LodSettings> {
    let targets = spec
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_suffix('%') {
            Some(pct) => {
                let pct: f32 = pct
                    .parse()
                    .with_context(|| format!("bad LOD percentage '{s}'"))?;
                anyhow::ensure!(
                    pct > 0.0 && pct < 100.0,
                    "LOD percentage '{s}' must be between 0 and 100"
                );
                Ok(LodTarget::Ratio(pct / 100.0))
            }
            None => {
                Ok(LodTarget::Triangles(s.parse().with_context(|| {
                    format!("bad LOD triangle count '{s}'")
                })?))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(!targets.is_empty(), "no LOD targets given");
    Ok(LodSettings {
        targets,
        ..Default::default()
    })
}

fn print_process_report(report: &ProcessReport) {
    for mesh in &report.meshes {
        println!("{}", mesh.name);
        if let Some(c) = &mesh.cleanup
            && c.welded_vertices + c.degenerate_triangles + c.unused_vertices > 0
        {
            println!(
                "  cleanup: {} vertices welded, {} degenerate triangles, {} unused vertices removed",
                c.welded_vertices, c.degenerate_triangles, c.unused_vertices
            );
        }
        for w in &mesh.warnings {
            println!("  warning: {}", w.message);
        }
        if let Some(uv) = &mesh.uv {
            let m = &uv.metrics;
            println!(
                "  UVs: {} islands, {} flips, {} overlapping texels, angle distortion {:.3}, packing {:.0}% ({:.0} ms)",
                m.islands,
                m.flipped_triangles + m.zero_area_triangles,
                m.overlap_texels,
                m.angle_distortion_mean,
                m.packing_efficiency * 100.0,
                uv.elapsed_ms
            );
        }
        for lod in &mesh.lods {
            let mut line = format!(
                "  LOD{} {:>9} tris  ACMR {:.2} → {:.2}",
                lod.level, lod.triangles, lod.acmr_before, lod.acmr_after
            );
            if lod.level > 0 {
                line += &format!(
                    "  error {:.4} m  switch at screen size {:.3}",
                    lod.error, lod.screen_size
                );
                match lod.method {
                    SimplifyMethod::Standard => {}
                    SimplifyMethod::Permissive => line += "  (seams relaxed)",
                    SimplifyMethod::Sloppy => line += "  (sloppy fallback)",
                }
            }
            println!("{line}");
        }
    }
    if report.unused_materials + report.unused_images > 0 {
        println!(
            "removed {} unused materials and {} unused images",
            report.unused_materials, report.unused_images
        );
    }
}

/// Finds model files under `dir`, sorted for stable output.
fn find_models(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            find_models(&path, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && ["gltf", "glb", "obj"].contains(&ext.to_ascii_lowercase().as_str())
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

const UV_CSV_HEADER: &str = "file,mesh,method,triangles,sheets,islands,flips,zero_area,mirrored_islands,overlap_texels,out_of_bounds,angle_mean,angle_p95,area_mean,l2_stretch,seam_length,straight_ratio,density_mean,density_max,packing,uv_ms\n";

fn uv_csv_row(
    file: &str,
    mesh: &str,
    method: &str,
    triangles: usize,
    m: &UvMetrics,
    ms: f64,
) -> String {
    format!(
        "{file},{},{method},{triangles},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{ms:.1}\n",
        mesh.replace(',', " "),
        m.sheets,
        m.islands,
        m.flipped_triangles,
        m.zero_area_triangles,
        m.mirrored_islands,
        m.overlap_texels,
        m.out_of_bounds_triangles,
        m.angle_distortion_mean,
        m.angle_distortion_p95,
        m.area_distortion_mean,
        m.l2_stretch,
        m.seam_length,
        m.straight_edge_ratio,
        m.density_deviation_mean,
        m.density_deviation_max,
        m.packing_efficiency,
    )
}

fn bench(dir: &Path, output: &Path, uv: bool) -> Result<()> {
    let mut files = Vec::new();
    find_models(dir, &mut files)?;
    anyhow::ensure!(
        !files.is_empty(),
        "no .gltf, .glb, or .obj files in {}",
        dir.display()
    );

    let options = ProcessOptions {
        uvs: uv.then(UvSettings::default),
        lods: Some(LodSettings::default()),
        ..Default::default()
    };
    let resolution = UvSettings::default().resolution;
    let mut uv_csv = String::from(UV_CSV_HEADER);
    let (mut uv_meshes, mut uv_failed) = (0, 0);
    let mut csv = String::from(
        "file,mesh,level,triangles,target_triangles,vertices,error,screen_size,method,acmr_before,acmr_after,file_ms\n",
    );
    let mut failures = 0;
    let mut off_target = 0;
    for file in &files {
        let name = file
            .strip_prefix(dir)
            .unwrap_or(file)
            .display()
            .to_string()
            .replace('\\', "/");
        let start = Instant::now();
        let scene = match lm_core::io::load(file) {
            Ok(import) => import.scene,
            Err(e) => {
                eprintln!("failed  {name}: {e}");
                failures += 1;
                continue;
            }
        };
        if !uv {
            for mesh in &scene.meshes {
                if let Some(m) = uv_metrics(mesh, 0, resolution) {
                    uv_csv +=
                        &uv_csv_row(&name, &mesh.name, "input", mesh.triangle_count(), &m, 0.0);
                }
            }
        }
        let (_, report) = pipeline::process(scene, &options);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let (mut permissive, mut sloppy) = (0, 0);
        for mesh in &report.meshes {
            for lod in &mesh.lods {
                permissive += usize::from(lod.method == SimplifyMethod::Permissive);
                sloppy += usize::from(lod.method == SimplifyMethod::Sloppy);
                let miss = lod.triangles.abs_diff(lod.target_triangles);
                off_target += usize::from(lod.level > 0 && miss > lod.target_triangles / 20 + 2);
                csv += &format!(
                    "{name},{},{},{},{},{},{},{},{},{},{},{ms:.1}\n",
                    mesh.name.replace(',', " "),
                    lod.level,
                    lod.triangles,
                    lod.target_triangles,
                    lod.vertices,
                    lod.error,
                    lod.screen_size,
                    serde_json::to_value(lod.method)?
                        .as_str()
                        .unwrap_or_default(),
                    lod.acmr_before,
                    lod.acmr_after
                );
            }
        }
        for mesh in &report.meshes {
            if let Some(r) = &mesh.uv {
                let m = &r.metrics;
                uv_meshes += 1;
                uv_failed +=
                    usize::from(m.flipped_triangles + m.zero_area_triangles + m.overlap_texels > 0);
                let triangles = mesh.lods.first().map_or(0, |l| l.triangles);
                uv_csv += &uv_csv_row(&name, &mesh.name, "xatlas", triangles, m, r.elapsed_ms);
            }
        }
        let mut note = String::new();
        if permissive > 0 {
            note += &format!(", {permissive} with seams relaxed");
        }
        if sloppy > 0 {
            note += &format!(", {sloppy} sloppy");
        }
        println!("ok      {name} ({ms:.0} ms{note})");
    }
    std::fs::write(output, csv).with_context(|| format!("writing {}", output.display()))?;
    let uv_output = output.with_file_name(format!(
        "{}_uv.csv",
        output.file_stem().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&uv_output, uv_csv)
        .with_context(|| format!("writing {}", uv_output.display()))?;
    if uv {
        println!("Auto UVs: {uv_failed} of {uv_meshes} meshes have flips or overlaps");
    }
    if off_target > 0 {
        println!(
            "{off_target} LOD levels missed their triangle target by more than 5% (usually tiny parts)"
        );
    }
    println!(
        "\n{} of {} files processed, results in {} and {}",
        files.len() - failures,
        files.len(),
        output.display(),
        uv_output.display()
    );
    anyhow::ensure!(failures == 0, "{failures} files failed");
    Ok(())
}

fn load(path: &Path) -> Result<lm_core::io::Import> {
    let import = lm_core::io::load(path).with_context(|| format!("reading {}", path.display()))?;
    for warning in &import.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(import)
}

fn print_stats(stats: &SceneStats) {
    println!(
        "{:<24} {:>10} {:>10} {:>10} {:>4} {:>3} {:>6} {:>7} {:>6}",
        "mesh", "triangles", "vertices", "welded", "subs", "uvs", "parts", "borders", "nonman"
    );
    for m in &stats.meshes {
        println!(
            "{:<24} {:>10} {:>10} {:>10} {:>4} {:>3} {:>6} {:>7} {:>6}",
            truncate(&m.name, 24),
            m.triangles,
            m.vertices,
            m.unique_positions,
            m.submeshes,
            m.uv_channels,
            m.components,
            m.boundary_loops,
            m.non_manifold_edges,
        );
    }
    println!(
        "\n{} meshes, {} triangles, {} vertices, {} materials, {} images",
        stats.meshes.len(),
        stats.total_triangles,
        stats.total_vertices,
        stats.materials,
        stats.images
    );
    for m in &stats.meshes {
        let size: Vec<String> = (0..3)
            .map(|k| format!("{:.3}", m.bounds_max[k] - m.bounds_min[k]))
            .collect();
        let mut missing = Vec::new();
        if !m.has_normals {
            missing.push("normals");
        }
        if m.uv_channels == 0 {
            missing.push("UVs");
        }
        let note = if missing.is_empty() {
            String::new()
        } else {
            format!("  (no {})", missing.join(", "))
        };
        println!("  {}: size {} m{note}", m.name, size.join(" × "));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
