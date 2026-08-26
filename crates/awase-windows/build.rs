fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        awase_build_support::embed_awase_manifest("Awase.Awase");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
