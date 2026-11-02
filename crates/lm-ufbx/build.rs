fn main() {
    println!("cargo:rerun-if-changed=shim/shim.c");
    println!("cargo:rerun-if-changed=vendor/ufbx/ufbx.c");
    println!("cargo:rerun-if-changed=vendor/ufbx/ufbx.h");

    cc::Build::new()
        .include("vendor/ufbx")
        .file("vendor/ufbx/ufbx.c")
        .file("shim/shim.c")
        // Large files load far too slowly unoptimized, so debug builds optimize too.
        .opt_level(2)
        .define("NDEBUG", None)
        .warnings(false)
        .compile("lm_ufbx");
}
