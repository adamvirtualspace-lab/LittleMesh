//! End-to-end checks of the `littlemesh` binary.

use std::process::Command;

const QUAD_OBJ: &str = "\
o quad
v 0 0 0
v 2 0 0
v 2 0 3
v 0 0 3
vt 0 0
vt 1 0
vt 1 1
vt 0 1
f 1/1 4/4 3/3 2/2
";

fn littlemesh(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_littlemesh"))
        .args(args)
        .output()
        .expect("binary runs")
}

#[test]
fn process_then_stats() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("quad.obj");
    let glb = dir.path().join("quad.glb");
    std::fs::write(&obj, QUAD_OBJ).unwrap();

    let out = littlemesh(&[
        "process",
        obj.to_str().unwrap(),
        "-o",
        glb.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = littlemesh(&["stats", glb.to_str().unwrap(), "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mesh = &stats["meshes"][0];
    assert_eq!(mesh["name"], "quad");
    assert_eq!(mesh["triangles"], 2);
    assert_eq!(mesh["boundary_loops"], 1);
    assert_eq!(mesh["bounds_max"], serde_json::json!([2.0, 0.0, 3.0]));
}

#[test]
fn unsupported_format_fails_cleanly() {
    let out = littlemesh(&["stats", "model.fbx"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot import '.fbx' files"), "{stderr}");
}

/// A wavy `n × n` quad grid as OBJ text, dense enough to simplify.
fn wavy_grid_obj(n: usize) -> String {
    let mut obj = String::from("o wavy\n");
    for j in 0..=n {
        for i in 0..=n {
            let (x, z) = (i as f32 / n as f32, j as f32 / n as f32);
            let y = 0.05 * (x * 12.0).sin() * (z * 9.0).cos();
            obj += &format!("v {x} {y} {z}\nvt {x} {z}\n");
        }
    }
    for j in 0..n {
        for i in 0..n {
            let v = |i: usize, j: usize| j * (n + 1) + i + 1;
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
            obj += &format!("f {a}/{a} {d}/{d} {c}/{c} {b}/{b}\n");
        }
    }
    obj
}

#[test]
fn process_with_lods_writes_suffixed_meshes_and_report() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("wavy.obj");
    let glb = dir.path().join("wavy.glb");
    let report = dir.path().join("report.json");
    std::fs::write(&obj, wavy_grid_obj(40)).unwrap();

    let out = littlemesh(&[
        "process",
        obj.to_str().unwrap(),
        "-o",
        glb.to_str().unwrap(),
        "--lods",
        "--report",
        report.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = littlemesh(&["stats", glb.to_str().unwrap(), "--json"]);
    let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = stats["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["wavy_LOD0", "wavy_LOD1", "wavy_LOD2", "wavy_LOD3"]);
    assert_eq!(stats["meshes"][0]["triangles"], 3200);

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let lods = report["meshes"][0]["lods"].as_array().unwrap();
    assert_eq!(lods[2]["target_triangles"], 800);
    assert!(lods[3]["screen_size"].as_f64().unwrap() <= lods[1]["screen_size"].as_f64().unwrap());
}

#[test]
fn bad_lod_spec_is_rejected() {
    let out = littlemesh(&["process", "in.obj", "-o", "out.glb", "--lods", "150%"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("between 0 and 100"));
}

#[test]
fn process_with_uv_reports_clean_uvs() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("wavy.obj");
    let glb = dir.path().join("wavy.glb");
    let report = dir.path().join("report.json");
    std::fs::write(&obj, wavy_grid_obj(20)).unwrap();

    let out = littlemesh(&[
        "process",
        obj.to_str().unwrap(),
        "-o",
        glb.to_str().unwrap(),
        "--uv",
        "--uv-resolution",
        "512",
        "--report",
        report.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("UVs:"));

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let uv = &report["meshes"][0]["uv"];
    assert_eq!(uv["method"], "xatlas");
    assert_eq!(uv["metrics"]["flipped_triangles"], 0);
    assert_eq!(uv["metrics"]["overlap_texels"], 0);
    assert_eq!(uv["metrics"]["raster_resolution"], 512);

    let out = littlemesh(&["stats", glb.to_str().unwrap(), "--json"]);
    let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(stats["meshes"][0]["uv_channels"], 1);
    assert_eq!(stats["meshes"][0]["triangles"], 800);
}

#[test]
fn debug_outputs_are_written_next_to_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("wavy.obj");
    let gltf = dir.path().join("wavy.gltf");
    std::fs::write(&obj, wavy_grid_obj(10)).unwrap();

    let out = littlemesh(&[
        "process",
        obj.to_str().unwrap(),
        "-o",
        gltf.to_str().unwrap(),
        "--uv",
        "--lods",
        "--debug-uv",
        "--debug-classes",
        "--debug-checker",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let png = std::fs::read(dir.path().join("wavy.uv.png")).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    // Debug scenes keep the output's format and hold LOD0 only.
    for name in ["wavy.classes.gltf", "wavy.checker.gltf"] {
        let path = dir.path().join(name);
        let out = littlemesh(&["stats", path.to_str().unwrap(), "--json"]);
        assert!(out.status.success(), "{name}");
        let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(stats["meshes"].as_array().unwrap().len(), 1, "{name}");
        assert_eq!(stats["meshes"][0]["triangles"], 200, "{name}");
    }
}

#[test]
fn bad_uv_resolution_is_rejected() {
    let out = littlemesh(&["process", "in.obj", "-o", "out.glb", "--uv-resolution", "8"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("between 64 and 16384"));
}
