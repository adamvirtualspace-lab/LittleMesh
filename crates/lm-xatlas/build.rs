fn main() {
    println!("cargo:rerun-if-changed=shim/shim.cpp");
    println!("cargo:rerun-if-changed=vendor/xatlas/xatlas.cpp");
    println!("cargo:rerun-if-changed=vendor/xatlas/xatlas.h");

    cc::Build::new()
        .cpp(true)
        .std("c++14")
        .include("vendor/xatlas")
        .file("vendor/xatlas/xatlas.cpp")
        .file("shim/shim.cpp")
        // xatlas is far too slow unoptimized, so debug builds optimize it too.
        .opt_level(2)
        .define("NDEBUG", None)
        .define("XA_DEBUG", "0")
        .warnings(false)
        .compile("lm_xatlas");
}
