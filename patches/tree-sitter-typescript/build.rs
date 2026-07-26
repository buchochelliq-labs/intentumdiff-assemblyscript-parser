fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = std::env::var("TARGET").unwrap_or_default();
    if target == "wasm32-wasip2" {
        // Two separate .a files: one for TypeScript, one for TSX.
        println!("cargo:rustc-link-lib=static=tree_sitter_typescript");
        println!("cargo:rustc-link-lib=static=tree_sitter_tsx");
        println!("cargo:rustc-link-search=native={}/lib", manifest);
        println!("cargo:rerun-if-changed=lib/libtree_sitter_typescript.a");
        println!("cargo:rerun-if-changed=lib/libtree_sitter_tsx.a");
        return;
    }

    let typescript_dir = std::path::Path::new("typescript").join("src");
    let tsx_dir = std::path::Path::new("tsx").join("src");
    let common_dir = std::path::Path::new("common");
    let mut cfg = cc::Build::new();
    cfg.include(&typescript_dir)
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Wno-unused-parameter");
    for path in [
        typescript_dir.join("parser.c"),
        typescript_dir.join("scanner.c"),
        tsx_dir.join("parser.c"),
        tsx_dir.join("scanner.c"),
    ] {
        cfg.file(&path);
        println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
    }
    println!("cargo:rerun-if-changed={}", common_dir.join("scanner.h").to_str().unwrap());
    cfg.compile("tree_sitter_typescript");
}
