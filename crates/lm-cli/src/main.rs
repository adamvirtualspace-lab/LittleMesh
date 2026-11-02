//! `littlemesh`: the LittleMesh command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lm_core::stats::{SceneStats, scene_stats};

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
    /// Process a mesh and write the result as glTF/GLB.
    Process {
        /// glTF/GLB or OBJ file.
        input: PathBuf,
        /// Output .glb or .gltf file.
        #[arg(short, long)]
        output: PathBuf,
    },
}

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
        Command::Process { input, output } => {
            let import = load(&input)?;
            lm_core::io::save(&import.scene, &output)
                .with_context(|| format!("writing {}", output.display()))?;
            let stats = scene_stats(&import.scene);
            println!(
                "wrote {} ({} meshes, {} triangles)",
                output.display(),
                stats.meshes.len(),
                stats.total_triangles
            );
        }
    }
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
