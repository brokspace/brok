//! Embeds a Windows application manifest declaring long path awareness.
//!
//! Index files live deep under `.brokspace/databases/`; without the manifest
//! Windows enforces the 260 character limit on brok even when long paths
//! are enabled system-wide (`LongPathsEnabled`), and storage writes fail in
//! deep workspaces.

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::Setting;
        embed_manifest::embed_manifest(
            embed_manifest::new_manifest("Brokspace.Brok").long_path_aware(Setting::Enabled),
        )
        .expect("unable to embed the Windows manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
