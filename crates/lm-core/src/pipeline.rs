//! The processing pipeline: runs the enabled stages over a scene in order
//! (docs/algorithms.md §1) and reports what each one did.

use serde::Serialize;

use crate::lod::{self, LodSettings, SimplifyMethod};
use crate::optimize::{self, CleanupReport, Warning, WarningKind};
use crate::uv::debug::DebugMesh;
use crate::uv::{self, UvReport, UvSettings};
use crate::{Mesh, Scene};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessOptions {
    pub cleanup: bool,
    pub optimize: bool,
    /// Replace UV0 with Auto UVs; `None` keeps the input UVs.
    pub uvs: Option<UvSettings>,
    /// Generate a LOD chain; `None` exports LOD0 only.
    pub lods: Option<LodSettings>,
    /// Keep a copy of each mesh as it left the UV stage, for debug outputs.
    pub debug: bool,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            cleanup: true,
            optimize: true,
            uvs: None,
            lods: None,
            debug: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessReport {
    pub meshes: Vec<MeshReport>,
    pub unused_materials: usize,
    pub unused_images: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeshReport {
    pub name: String,
    pub cleanup: Option<CleanupReport>,
    pub warnings: Vec<Warning>,
    /// Set when Auto UVs ran on this mesh.
    pub uv: Option<UvReport>,
    /// LOD0 first; a single entry when LODs are off.
    pub lods: Vec<LodReport>,
    /// The mesh after cleanup and Auto UVs, when [`ProcessOptions::debug`] is set.
    #[serde(skip)]
    pub debug: Option<DebugMesh>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LodReport {
    pub level: usize,
    /// Name of the exported mesh.
    pub name: String,
    pub triangles: usize,
    pub vertices: usize,
    pub target_triangles: usize,
    pub error: f32,
    pub screen_size: f32,
    pub method: SimplifyMethod,
    pub acmr_before: f32,
    pub acmr_after: f32,
}

/// Runs cleanup → Auto UVs → LOD chain → render optimization on every mesh.
pub fn process(mut scene: Scene, options: &ProcessOptions) -> (Scene, ProcessReport) {
    let mut report = ProcessReport::default();
    if options.cleanup {
        let cleanup = optimize::cleanup_scene(&mut scene);
        report.unused_materials = cleanup.unused_materials;
        report.unused_images = cleanup.unused_images;
        report.meshes = scene
            .meshes
            .iter()
            .zip(cleanup.meshes)
            .map(|(mesh, c)| mesh_report(mesh, Some(c)))
            .collect();
    } else {
        report.meshes = scene.meshes.iter().map(|m| mesh_report(m, None)).collect();
    }

    let mut output = Vec::new();
    for (mut mesh, mesh_report) in std::mem::take(&mut scene.meshes)
        .into_iter()
        .zip(&mut report.meshes)
    {
        let mut uv_failure = None;
        if let Some(settings) = &options.uvs {
            match uv::auto_uv(&mut mesh, settings) {
                Ok(r) => mesh_report.uv = Some(r),
                Err(e) => uv_failure = Some(e.to_string()),
            }
        }
        mesh_report.warnings = optimize::mesh_warnings(&mesh);
        if options.debug {
            mesh_report.debug = Some(match &mut mesh_report.uv {
                Some(r) => DebugMesh {
                    mesh: mesh.clone(),
                    classes: std::mem::take(&mut r.classes),
                },
                None => DebugMesh::input(mesh.clone()),
            });
        }
        if let Some(reason) = uv_failure {
            mesh_report.warnings.push(Warning {
                mesh: mesh.name.clone(),
                kind: WarningKind::AutoUvFailed,
                message: format!("Auto UVs failed, input UVs kept: {reason}"),
            });
        }
        let chain = match &options.lods {
            Some(settings) => lod::generate_chain(&mesh, settings),
            None => vec![lod::Lod {
                level: 0,
                target_triangles: mesh.triangle_count(),
                mesh,
                error: 0.0,
                screen_size: 1.0,
                method: SimplifyMethod::Standard,
            }],
        };
        let suffix = options.lods.is_some();
        for mut lod in chain {
            if suffix {
                lod.mesh.name = format!("{}_LOD{}", lod.mesh.name, lod.level);
            }
            let acmr_before = optimize::acmr(&lod.mesh);
            let acmr_after = if options.optimize {
                optimize::optimize_render(&mut lod.mesh).acmr_after
            } else {
                acmr_before
            };
            mesh_report.lods.push(LodReport {
                level: lod.level,
                name: lod.mesh.name.clone(),
                triangles: lod.mesh.triangle_count(),
                vertices: lod.mesh.vertex_count(),
                target_triangles: lod.target_triangles,
                error: lod.error,
                screen_size: lod.screen_size,
                method: lod.method,
                acmr_before,
                acmr_after,
            });
            output.push(lod.mesh);
        }
    }
    scene.meshes = output;
    (scene, report)
}

fn mesh_report(mesh: &Mesh, cleanup: Option<CleanupReport>) -> MeshReport {
    MeshReport {
        name: mesh.name.clone(),
        cleanup,
        warnings: Vec::new(),
        uv: None,
        debug: None,
        lods: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use crate::uv::UvClass;

    #[test]
    fn lods_become_suffixed_sibling_meshes() {
        let scene = Scene {
            meshes: vec![
                primitives::uv_sphere(1.0, 48, 24),
                primitives::torus(1.0, 0.3, 48, 24),
            ],
            ..Default::default()
        };
        let options = ProcessOptions {
            lods: Some(LodSettings::default()),
            ..Default::default()
        };
        let (out, report) = process(scene, &options);
        let names: Vec<&str> = out.meshes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            &names[..4],
            ["sphere_LOD0", "sphere_LOD1", "sphere_LOD2", "sphere_LOD3"]
        );
        assert!(names.contains(&"torus_LOD3"));
        assert_eq!(report.meshes.len(), 2);
        assert_eq!(report.meshes[0].lods.len(), 4);
        out.validate().unwrap();
    }

    #[test]
    fn without_lods_names_are_unchanged() {
        let scene = Scene {
            meshes: vec![primitives::cube(1.0)],
            ..Default::default()
        };
        let (out, report) = process(scene, &ProcessOptions::default());
        assert_eq!(out.meshes[0].name, "cube");
        assert_eq!(report.meshes[0].lods.len(), 1);
        assert!(report.meshes[0].warnings.is_empty());
    }

    #[test]
    fn auto_uvs_run_before_lods() {
        let mut sphere = primitives::uv_sphere(1.0, 48, 24);
        sphere.uvs.clear();
        let scene = Scene {
            meshes: vec![sphere],
            ..Default::default()
        };
        let options = ProcessOptions {
            uvs: Some(UvSettings::default()),
            lods: Some(LodSettings::default()),
            ..Default::default()
        };
        let (out, report) = process(scene, &options);
        let uv = report.meshes[0].uv.as_ref().unwrap();
        assert_eq!(uv.metrics.flipped_triangles, 0);
        assert!(report.meshes[0].warnings.is_empty());
        // Every LOD inherits UV0.
        assert!(out.meshes.iter().all(|m| m.uvs.len() == 1));
        out.validate().unwrap();
        assert!(report.meshes[0].debug.is_none());
    }

    #[test]
    fn debug_keeps_the_mesh_after_the_uv_stage() {
        let scene = Scene {
            meshes: vec![primitives::torus(1.0, 0.3, 24, 12)],
            ..Default::default()
        };
        let options = ProcessOptions {
            debug: true,
            ..Default::default()
        };
        let (_, report) = process(scene.clone(), &options);
        let debug = report.meshes[0].debug.as_ref().unwrap();
        assert!(debug.classes.iter().all(|&c| c == UvClass::Input));

        let options = ProcessOptions {
            uvs: Some(UvSettings::default()),
            ..options
        };
        let (_, report) = process(scene, &options);
        let debug = report.meshes[0].debug.as_ref().unwrap();
        assert_eq!(debug.classes.len(), debug.mesh.triangle_count());
        assert!(debug.classes.iter().all(|&c| c == UvClass::Xatlas));
        assert_eq!(
            debug.mesh.uvs[0].len(),
            report.meshes[0].uv.as_ref().unwrap().vertices_after
        );
    }
}
