//! `brok init`: from zero to a working Brokspace in one command.
//!
//! 1. create `.brokspace/` (idempotent),
//! 2. install the `brok-agent` system shard,
//! 3. set up semantic search (download the embedding model),
//! 4. configure a local LLM (Ollama / LM Studio, sized to this machine),
//! 5. build the first index,
//! 6. connect coding agents (skills + MCP).
//!
//! Every step can be skipped; `--yes` (or a non-interactive terminal) takes
//! the recommended answer everywhere.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use brok_core::Workspace;
use brok_core::init::{InitOptions, init_workspace};
use brok_sdk::ShardManifest;
use brok_shard::{Registry, SYSTEM_AGENT, ShardSource, find_executable};
use dialoguer::Confirm;

use super::setup::{agents, llm};
use crate::ui;

/// Arguments of `brok init`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Accept every recommendation without asking.
    #[arg(short, long)]
    yes: bool,
    /// Reset `brok.yml` to the defaults.
    #[arg(long)]
    force: bool,
    /// Do not download the embedding model (text search only until `brok model download`).
    #[arg(long)]
    skip_model: bool,
    /// Do not install the default `brok-agent` shard.
    #[arg(long)]
    no_agent: bool,
    /// Do not configure a local LLM.
    #[arg(long)]
    no_llm: bool,
    /// Do not index now.
    #[arg(long)]
    no_index: bool,
    /// Do not connect coding agents.
    #[arg(long)]
    no_integrations: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let ask = ui::interactive() && !args.yes;
    let confirm = |prompt: &str| -> Result<bool> {
        if ask { Ok(Confirm::new().with_prompt(prompt).default(true).interact()?) } else { Ok(true) }
    };

    section(1, "Workspace");
    let report = init_workspace(cwd, InitOptions { force: args.force })?;
    let mut workspace = report.workspace;
    if report.kept_config {
        ui::step(&format!("Using existing {}", workspace.paths().config_file.display()));
    } else {
        ui::success(&format!("Created {}", workspace.paths().brokspace.display()));
    }

    section(2, "Agent");
    if args.no_agent {
        ui::step("Skipped");
    } else {
        match install_agent(&Registry::new(workspace.paths())).await {
            Ok(version) => ui::success(&format!("Installed {SYSTEM_AGENT} {version}")),
            Err(e) => {
                ui::warn(&format!("{SYSTEM_AGENT} not installed: {e:#}"));
                ui::hint("Put brok-agent next to brok (the installer does this) and run `brok init` again, or:");
                ui::hint("brok shard install https://github.com/brokspace/brok-agent");
            }
        }
    }

    section(3, "Semantic search");
    setup_embeddings(&mut workspace, &args, &confirm).await?;

    section(4, "Local LLM");
    let user_llm = brok_core::user::UserConfig::load().llm.filter(|_| !report.kept_config);
    if args.no_llm {
        ui::step("Skipped (run `brok setup llm` later)");
    } else if let Some(llm) = user_llm {
        use_user_llm(&mut workspace, llm).await?;
    } else {
        let llm_args = llm::Args { yes: !ask, ..llm::Args::default() };
        if let Err(e) = llm::configure(&mut workspace, &llm_args).await {
            ui::warn(&format!("LLM setup incomplete: {e:#}"));
        }
    }

    section(5, "Index");
    if args.no_index || !confirm("Index the workspace now?")? {
        ui::step("Skipped (run `brok index`)");
    } else if let Err(e) = super::index::run(workspace.root(), super::index::Args::default()).await {
        ui::warn(&format!("indexing failed: {e:#}"));
    }

    section(6, "Coding agents");
    if args.no_integrations || !confirm("Connect your coding agents (Brokspace skills + MCP search tool)?")? {
        ui::step("Skipped (run `brok setup agents`)");
    } else {
        let agents_args = agents::Args { yes: !ask, ..agents::Args::default() };
        if let Err(e) = agents::configure(&workspace, &agents_args) {
            ui::warn(&format!("agent setup incomplete: {e:#}"));
        }
    }

    println!();
    ui::header("You're set");
    ui::hint("brok search \"how does X work\"   # find code by meaning");
    ui::hint("brok ask \"question\"              # answer from your LLM (starts brok in the background)");
    ui::hint("brok chat                         # conversation with follow-ups");
    ui::hint("brok review   /   brok commit      # AI review of your diff, commit message from staged changes");
    ui::hint("brok skill list                  # workflows: /ask-brok, /grill-me, /to-spec, tdd...");
    ui::hint("brok doctor                      # if anything looks off");
    Ok(())
}

/// Applies the LLM chosen earlier with `brok setup llm` (in any workspace).
async fn use_user_llm(workspace: &mut Workspace, llm: brok_core::config::LlmConfig) -> Result<()> {
    let mut config = workspace.config().clone();
    config.llm = llm;
    workspace.save_config(config)?;
    let llm = &workspace.config().llm;
    let Some(provider) = llm.providers.get(&llm.default) else { return Ok(()) };
    ui::success(&format!(
        "Using your default LLM: {} via {} (change it with `brok setup llm`)",
        provider.model, llm.default
    ));
    if crate::local_llm::list_models(&provider.base_url).await.is_none() && provider.api_key_env.is_none() {
        ui::warn(&format!(
            "{} is not reachable at {} right now; start it before `brok ask`",
            llm.default, provider.base_url
        ));
    }
    Ok(())
}

fn section(step: u8, title: &str) {
    println!();
    println!("{}", console::style(format!("[{step}/6] {title}")).bold());
}

/// Downloads the model, or turns embeddings off when that is not possible,
/// so indexing always works.
async fn setup_embeddings(
    workspace: &mut Workspace,
    args: &Args,
    confirm: &dyn Fn(&str) -> Result<bool>,
) -> Result<()> {
    if !workspace.config().embeddings.enabled {
        ui::step("Disabled in brok.yml: text search only");
        return Ok(());
    }
    if !brok_index::compiled_features().contains(&"onnx") {
        disable_embeddings(workspace, "this build has no ONNX support; using fast text search")?;
        return Ok(());
    }
    let files = brok_index::ModelFiles::locate(&workspace.paths().models, &workspace.config().embeddings)?;
    if files.is_present() {
        ui::success("Embedding model ready");
        return Ok(());
    }
    if args.skip_model || !confirm("Download the embedding model for semantic search (about 140 MB, one time)?")? {
        disable_embeddings(workspace, "text search only; enable later with `brok model download`")?;
        return Ok(());
    }
    if let Err(e) = super::model::download(workspace).await {
        ui::warn(&format!("download failed: {e:#}"));
        disable_embeddings(workspace, "text search only until `brok model download` succeeds")?;
    }
    Ok(())
}

fn disable_embeddings(workspace: &mut Workspace, why: &str) -> Result<()> {
    let mut config = workspace.config().clone();
    config.embeddings.enabled = false;
    workspace.save_config(config)?;
    ui::step(&format!("Semantic search off: {why} (set embeddings.enabled: true to turn it on)"));
    Ok(())
}

/// Installs the `brok-agent` binary found next to `brok` or on PATH. The
/// agent describes itself with `brok-agent --manifest`.
async fn install_agent(registry: &Registry) -> Result<semver::Version> {
    let binary = find_executable(SYSTEM_AGENT).context("`brok-agent` was not found next to brok or on PATH")?;
    let manifest = agent_manifest(&binary).await?;
    let version = manifest.version.clone();
    brok_shard::install::install_builtin(registry, manifest, &binary)?;
    Ok(version)
}

/// Each workspace runs its own copy of `brok-agent`. After `brok upgrade`
/// the one next to `brok` is newer: copy it in again. Only the builtin
/// agent is refreshed; one installed from git or a folder is left alone.
pub(crate) async fn refresh_agent(registry: &Registry) -> Result<Option<semver::Version>> {
    let Some(installed) = registry.get(SYSTEM_AGENT)? else { return Ok(None) };
    if installed.lock.as_ref().is_none_or(|lock| lock.source != ShardSource::Builtin) {
        return Ok(None);
    }
    let Some(binary) = find_executable(SYSTEM_AGENT) else { return Ok(None) };
    let manifest = agent_manifest(&binary).await?;
    if manifest.version <= installed.manifest.version {
        return Ok(None);
    }
    let version = manifest.version.clone();
    brok_shard::install::install_builtin(registry, manifest, &binary)?;
    Ok(Some(version))
}

/// The agent describes itself with `brok-agent --manifest`.
async fn agent_manifest(binary: &Path) -> Result<ShardManifest> {
    let output = tokio::process::Command::new(binary).arg("--manifest").output().await?;
    ensure!(output.status.success(), "`{} --manifest` failed", binary.display());
    Ok(ShardManifest::from_json(&String::from_utf8_lossy(&output.stdout))?)
}
