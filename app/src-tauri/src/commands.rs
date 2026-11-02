//! Commands the frontend invokes. Every one does its core work on a blocking
//! thread, so the UI never waits on processing (docs/milestones.md, UI
//! principles).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use lm_core::lod::LodSettings;
use lm_core::optimize::{self, Warning};
use lm_core::pipeline::{self, ProcessOptions, ProcessReport};
use lm_core::stats::{SceneStats, scene_stats};
use lm_core::uv::UvSettings;
use lm_core::{Mesh, Scene};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Response;

use crate::transfer;

#[derive(Default)]
pub struct AppState {
    session: Mutex<Session>,
}

#[derive(Default)]
struct Session {
    /// Bumped on every open, so work for an older file is dropped.
    generation: u64,
    input: Option<Arc<Scene>>,
    output: Option<Arc<Output>>,
}

struct Output {
    scene: Scene,
    report: ProcessReport,
}

impl AppState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn input(&self, generation: u64) -> Result<Arc<Scene>, String> {
        let session = self.lock();
        match &session.input {
            Some(scene) if session.generation == generation => Ok(scene.clone()),
            _ => Err(STALE.into()),
        }
    }
}

/// Returned when a newer file replaced the one a request was for.
const STALE: &str = "stale";

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

/// What the UI shows the moment a file has loaded; the slower topology
/// stats follow from [`input_stats`].
#[derive(Serialize)]
pub struct Opened {
    generation: u64,
    name: String,
    meshes: usize,
    triangles: usize,
    vertices: usize,
    /// Every mesh already has UV0, so Auto UVs is off by default.
    has_uvs: bool,
    import_warnings: Vec<String>,
}

#[tauri::command]
pub async fn open_file(path: String, state: State<'_, AppState>) -> Result<Opened, String> {
    let file = PathBuf::from(&path);
    let import = blocking(move || lm_core::io::load(&file))
        .await?
        .map_err(|e| e.to_string())?;
    let scene = import.scene;
    let opened = Opened {
        generation: 0,
        name: Path::new(&path)
            .file_name()
            .map_or(path.clone(), |n| n.to_string_lossy().into_owned()),
        meshes: scene.meshes.len(),
        triangles: scene.meshes.iter().map(Mesh::triangle_count).sum(),
        vertices: scene.meshes.iter().map(Mesh::vertex_count).sum(),
        has_uvs: scene.meshes.iter().all(|m| !m.uvs.is_empty()),
        import_warnings: import.warnings,
    };
    let mut session = state.lock();
    session.generation += 1;
    session.input = Some(Arc::new(scene));
    session.output = None;
    Ok(Opened {
        generation: session.generation,
        ..opened
    })
}

#[derive(Serialize)]
pub struct InputStats {
    stats: SceneStats,
    warnings: Vec<Warning>,
}

#[tauri::command]
pub async fn input_stats(
    generation: u64,
    state: State<'_, AppState>,
) -> Result<InputStats, String> {
    let scene = state.input(generation)?;
    blocking(move || InputStats {
        stats: scene_stats(&scene),
        warnings: scene
            .meshes
            .iter()
            .flat_map(optimize::mesh_warnings)
            .collect(),
    })
    .await
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Input,
    Output,
}

/// The meshes of one view as a binary buffer (see [`transfer`]). For the
/// output, `lod` picks the level per mesh, clamped to the levels it has.
#[tauri::command]
pub async fn scene_buffer(
    generation: u64,
    view: View,
    lod: usize,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let input = state.input(generation)?;
    let output = state.lock().output.clone();
    let bytes = blocking(move || match (view, output) {
        (View::Input, _) => Ok(transfer::encode(&input.meshes.iter().collect::<Vec<_>>())),
        (View::Output, Some(out)) => Ok(transfer::encode(&lod_meshes(&out, lod))),
        (View::Output, None) => Err("nothing processed yet".to_string()),
    })
    .await??;
    Ok(Response::new(bytes))
}

fn lod_meshes(out: &Output, lod: usize) -> Vec<&Mesh> {
    out.report
        .meshes
        .iter()
        .filter_map(|m| m.lods.get(lod).or(m.lods.last()))
        .filter_map(|l| out.scene.meshes.iter().find(|m| m.name == l.name))
        .collect()
}

#[derive(Deserialize)]
pub struct UiOptions {
    auto_uvs: bool,
    uv_resolution: u32,
    lods: bool,
}

#[derive(Serialize)]
pub struct Processed {
    report: ProcessReport,
    /// Most LOD levels any mesh has, LOD0 included.
    lod_levels: usize,
    triangles: usize,
    elapsed_ms: f64,
}

#[tauri::command]
pub async fn process(
    generation: u64,
    options: UiOptions,
    state: State<'_, AppState>,
) -> Result<Processed, String> {
    if !(64..=16384).contains(&options.uv_resolution) {
        return Err("Texture size must be between 64 and 16384 pixels.".into());
    }
    let input = state.input(generation)?;
    let start = Instant::now();
    let options = ProcessOptions {
        uvs: options.auto_uvs.then(|| UvSettings {
            resolution: options.uv_resolution,
            ..Default::default()
        }),
        lods: options.lods.then(LodSettings::default),
        ..Default::default()
    };
    let (scene, report) = blocking(move || pipeline::process((*input).clone(), &options)).await?;
    let processed = Processed {
        lod_levels: report
            .meshes
            .iter()
            .map(|m| m.lods.len())
            .max()
            .unwrap_or(0),
        triangles: report
            .meshes
            .iter()
            .filter_map(|m| m.lods.first())
            .map(|l| l.triangles)
            .sum(),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        report: report.clone(),
    };
    let mut session = state.lock();
    if session.generation != generation {
        return Err(STALE.into());
    }
    session.output = Some(Arc::new(Output { scene, report }));
    Ok(processed)
}

/// Writes the processed scene; the format follows the extension.
#[tauri::command]
pub async fn export(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let output = state
        .lock()
        .output
        .clone()
        .ok_or("Process the mesh before exporting.")?;
    blocking(move || lm_core::io::save(&output.scene, Path::new(&path)))
        .await?
        .map_err(|e| e.to_string())
}

/// A file passed on the command line, as "Open with" does.
#[tauri::command]
pub fn startup_file() -> Option<String> {
    std::env::args().nth(1)
}
