//! Installer and registry round trip with local sources.

use brok_core::WorkspacePaths;
use brok_sdk::ShardManifest;
use brok_shard::{AutoApprove, Consent, InstallMethod, InstallOptions, InstallSource, Registry, ShardSource, install};

fn make_shard(dir: &std::path::Path, version: &str, requires: Option<&str>) {
    let mut manifest = serde_json::json!({
        "name": "demo-shard",
        "version": version,
        "entrypoint": "bin/demo",
        "permissions": { "filesystem": { "read": ["src/"] } },
        "subscribes_to": ["file:changed"],
        "publishes_to": ["demo:found"]
    });
    if let Some(requires) = requires {
        manifest["brok"] = requires.into();
    }
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("shard.json"), manifest.to_string()).unwrap();
    let binary = if cfg!(windows) { "bin/demo.exe" } else { "bin/demo" };
    std::fs::write(dir.join(binary), b"#!/bin/sh\necho demo\n").unwrap();
}

struct Deny;
impl Consent for Deny {
    fn approve(&self, _: &ShardManifest, report: &[String]) -> bool {
        assert!(report.iter().any(|l| l.contains("src/")));
        false
    }
}

#[tokio::test]
async fn install_upgrade_and_remove() {
    let ws = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let paths = WorkspacePaths::new(ws.path());
    let registry = Registry::new(&paths);
    let log = |_: &str| {};

    make_shard(src.path(), "0.1.0", Some(">=0.1.0"));
    let source = InstallSource::parse(src.path().to_str().unwrap(), None);
    assert!(matches!(source, InstallSource::Local(_)));

    let err = install(&registry, source.clone(), InstallOptions::default(), &Deny, &log).await.unwrap_err();
    assert!(err.to_string().contains("cancelled"));
    assert!(registry.list().unwrap().is_empty());

    let outcome = install(&registry, source.clone(), InstallOptions::default(), &AutoApprove, &log).await.unwrap();
    assert_eq!(outcome.method, InstallMethod::Prebuilt);
    assert!(outcome.shard.is_runnable());
    assert!(matches!(outcome.shard.lock.as_ref().unwrap().source, ShardSource::Local { .. }));

    // Same version again needs --force; a newer version upgrades.
    assert!(install(&registry, source.clone(), InstallOptions::default(), &AutoApprove, &log).await.is_err());
    make_shard(src.path(), "0.2.0", None);
    let upgraded = install(&registry, source.clone(), InstallOptions::default(), &AutoApprove, &log).await.unwrap();
    assert_eq!(upgraded.replaced, Some(semver::Version::new(0, 1, 0)));

    let listed = registry.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].manifest.version, semver::Version::new(0, 2, 0));
    assert!(!paths.shards.join(".staging").read_dir().unwrap().any(|_| true), "staging is cleaned up");

    assert!(registry.remove("demo-shard").unwrap());
    assert!(registry.list().unwrap().is_empty());
    assert!(!registry.remove("demo-shard").unwrap());
}

#[tokio::test]
async fn incompatible_brok_versions_are_refused() {
    let ws = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    make_shard(src.path(), "1.0.0", Some(">=99.0.0"));
    let registry = Registry::new(&WorkspacePaths::new(ws.path()));
    let source = InstallSource::Local(src.path().to_path_buf());
    let err = install(&registry, source, InstallOptions::default(), &AutoApprove, &|_| {}).await.unwrap_err();
    assert!(err.to_string().contains("requires brok"), "{err}");
}

#[tokio::test]
async fn skill_only_shards_install_without_a_process() {
    let ws = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("skills/release-notes")).unwrap();
    std::fs::write(
        src.path().join("skills/release-notes/SKILL.md"),
        "---
name: release-notes
description: x
---
Body
",
    )
    .unwrap();
    std::fs::write(
        src.path().join("shard.json"),
        r#"{ "name": "acme-skills", "version": "1.0.0", "skills": ["skills"] }"#,
    )
    .unwrap();

    let registry = Registry::new(&WorkspacePaths::new(ws.path()));
    let source = InstallSource::Local(src.path().to_path_buf());
    let outcome = install(&registry, source, InstallOptions::default(), &AutoApprove, &|_| {}).await.unwrap();
    assert_eq!(outcome.method, InstallMethod::SkillsOnly);
    assert!(!outcome.shard.is_runnable());
    assert!(outcome.shard.executable().is_none());
    let skills = outcome.shard.skill_dirs();
    assert!(skills[0].join("release-notes/SKILL.md").is_file());
    assert_eq!(outcome.shard.lock.unwrap().binary_sha256.len(), 64);
}
