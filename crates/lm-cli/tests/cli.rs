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
