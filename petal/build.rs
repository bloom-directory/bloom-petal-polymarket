use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=BLOOM_ROUTE_PATH");
    println!("cargo:rerun-if-env-changed=BLOOM_ROUTE_CANONICAL_PATH");
    println!("cargo:rerun-if-env-changed=BLOOM_ROUTE_PARAMS");

    if env::var_os("BLOOM_ROUTE_PATH").is_none() {
        println!("cargo:rustc-env=BLOOM_ROUTE_PATH=$index");
    }
    if env::var_os("BLOOM_ROUTE_CANONICAL_PATH").is_none() {
        println!("cargo:rustc-env=BLOOM_ROUTE_CANONICAL_PATH=");
    }
    if env::var_os("BLOOM_ROUTE_PARAMS").is_none() {
        println!("cargo:rustc-env=BLOOM_ROUTE_PARAMS=");
    }
}
