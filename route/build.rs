use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=BLOOM_ROUTE_RS");
    println!("cargo:rerun-if-changed=files");

    if env::var_os("BLOOM_ROUTE_RS").is_none() {
        let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let default_route = manifest_dir.join("files/$index.rs");
        println!("cargo:rustc-env=BLOOM_ROUTE_RS={}", default_route.display());
    }
}
