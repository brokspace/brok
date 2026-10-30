//! `brok setup llm`: zero-config local LLM.
//!
//! Detects the machine's memory, recommends a model size (see
//! `docs/local-llm.md`), finds a running Ollama or LM Studio server, offers
//! to download the recommended model, and records the choice in `brok.yml`.

use std::path::Path;

use anyhow::{Context, Result};
use brok_core::Workspace;
use brok_core::config::LlmProvider;
use dialoguer::{Confirm, Select};

use crate::local_llm::{self, Server, Tier};
use crate::ui;

/// Arguments of `brok setup llm`.
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Accept the recommendations without asking.
    #[arg(short, long)]
    pub yes: bool,
    /// With `--yes`: also download the recommended model (`ollama pull`).
    #[arg(long)]
    pub pull: bool,
    /// Use this provider from `llm.providers` (e.g. `ollama`, `lmstudio`, `anthropic`).
    #[arg(long)]
    pub provider: Option<String>,
    /// Use this model name.
    #[arg(long)]
    pub model: Option<String>,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let mut workspace = crate::cmd::workspace(cwd)?;
    configure(&mut workspace, &args).await
}

/// The reusable flow, also driven by `brok init`.
pub async fn configure(workspace: &mut Workspace, args: &Args) -> Result<()> {
    let interactive = ui::interactive() && !args.yes;

    if let Some(provider) = &args.provider {
        let model = args.model.clone();
        save(workspace, provider, None, model.as_deref())?;
        ui::success(&format!("Default LLM: {provider}{}", model.map(|m| format!(" ({m})")).unwrap_or_default()));
        return Ok(());
    }

    let hardware = local_llm::detect_hardware();
    let tier = local_llm::recommend(hardware);
    ui::step(&format!(
        "This machine ({}) runs {} models comfortably; recommended: {}",
        tier.reason, tier.size, tier.ollama_model
    ));

    let servers = local_llm::probe().await;
    if servers.is_empty() {
        return no_server(workspace, &tier);
    }
    let server = if servers.len() > 1 && interactive {
        let labels: Vec<String> =
            servers.iter().map(|s| format!("{} ({} models at {})", s.label, s.models.len(), s.base_url)).collect();
        &servers[Select::new().with_prompt("Which local server?").items(&labels).default(0).interact()?]
    } else {
        &servers[0]
    };
    ui::success(&format!("Found {} at {} with {} model(s)", server.label, server.base_url, server.models.len()));

    let model = match server.provider {
        "ollama" => choose_ollama_model(server, &tier, interactive, args.pull).await?,
        _ => choose_listed_model(server, interactive)?,
    };
    match &model {
        Some(model) => {
            save(workspace, server.provider, Some(server.base_url), Some(model))?;
            ui::success(&format!("brok ask will use {} via {}", model, server.label));
            if server.provider == "ollama" {
                ui::hint(&format!("Aider can share it: aider --model ollama/{model}"));
            }
        }
        None => {
            save(workspace, server.provider, Some(server.base_url), Some(tier.ollama_model))?;
            ui::warn(&format!("no model available yet; run `ollama pull {}`", tier.ollama_model));
        }
    }
    Ok(())
}

async fn choose_ollama_model(server: &Server, tier: &Tier, interactive: bool, pull: bool) -> Result<Option<String>> {
    let recommended = tier.ollama_model;
    if local_llm::has_model(&server.models, recommended) {
        return Ok(Some(recommended.to_owned()));
    }
    let installed = local_llm::pick_installed(&server.models, recommended);
    let download = if interactive {
        let prompt = match &installed {
            Some(model) => format!("Download the recommended {recommended} ({})? (No keeps {model})", size_of(tier)),
            None => format!("Download the recommended {recommended} now ({})?", size_of(tier)),
        };
        Confirm::new().with_prompt(prompt).default(installed.is_none()).interact()?
    } else {
        pull
    };
    if download {
        ui::step(&format!("Running `ollama pull {recommended}`"));
        let status = tokio::process::Command::new("ollama")
            .args(["pull", recommended])
            .status()
            .await
            .context("cannot run `ollama` (is it on PATH?)")?;
        if status.success() {
            return Ok(Some(recommended.to_owned()));
        }
        ui::warn("ollama pull failed; keeping what is installed");
    }
    Ok(installed)
}

fn choose_listed_model(server: &Server, interactive: bool) -> Result<Option<String>> {
    if server.models.is_empty() {
        ui::warn(&format!("{} has no model loaded: download one in the app and load it", server.label));
        return Ok(None);
    }
    if interactive && server.models.len() > 1 {
        let default = server.models.iter().position(|m| m.contains("coder")).unwrap_or(0);
        let index = Select::new().with_prompt("Which model?").items(&server.models).default(default).interact()?;
        return Ok(Some(server.models[index].clone()));
    }
    Ok(local_llm::pick_installed(&server.models, ""))
}

/// No server answered: configure the recommendation and explain the
/// one-time install.
fn no_server(workspace: &mut Workspace, tier: &Tier) -> Result<()> {
    save(workspace, "ollama", Some("http://localhost:11434/v1"), Some(tier.ollama_model))?;
    if crate::integrations::on_path("ollama") {
        ui::warn("Ollama is installed but not running. Start the Ollama app (or `ollama serve`), then:");
        ui::hint(&format!("ollama pull {}", tier.ollama_model));
    } else {
        ui::warn("No local LLM server found. Install one (one time):");
        for line in local_llm::install_instructions() {
            ui::hint(&line);
        }
        ui::hint(&format!("then: ollama pull {}   (LM Studio: search \"{}\")", tier.ollama_model, tier.lmstudio_hint));
    }
    for (provider, var) in [("anthropic", "ANTHROPIC_API_KEY"), ("deepseek", "DEEPSEEK_API_KEY")] {
        if std::env::var_os(var).is_some() {
            ui::hint(&format!("{var} is set: `brok ask -p {provider} \"...\"` works right away"));
        }
    }
    ui::hint("Run `brok setup llm` again once it is running; search and indexing work without an LLM.");
    Ok(())
}

fn size_of(tier: &Tier) -> &'static str {
    match tier.ollama_model {
        "qwen2.5-coder:14b" => "about 9 GB",
        "qwen2.5-coder:7b" => "about 4.7 GB",
        "qwen2.5-coder:3b" => "about 1.9 GB",
        _ => "about 1 GB",
    }
}

/// Makes `provider` the default, creating or updating its entry.
fn save(workspace: &mut Workspace, provider: &str, base_url: Option<&str>, model: Option<&str>) -> Result<()> {
    let mut config = workspace.config().clone();
    let known = local_llm::KNOWN.iter().find(|(name, _, _)| *name == provider).map(|(_, _, url)| *url);
    let entry = config.llm.providers.entry(provider.to_owned()).or_insert_with(|| LlmProvider {
        base_url: base_url.or(known).unwrap_or("http://localhost:11434/v1").to_owned(),
        model: model.unwrap_or("qwen2.5-coder:7b").to_owned(),
        api_key_env: None,
        temperature: None,
        max_tokens: None,
    });
    if let Some(url) = base_url {
        entry.base_url = url.to_owned();
    }
    if let Some(model) = model {
        entry.model = model.to_owned();
    }
    config.llm.default = provider.to_owned();
    let llm = config.llm.clone();
    workspace.save_config(config)?;

    // Remember the choice for every future `brok init`.
    let mut user = brok_core::user::UserConfig::load();
    user.llm = Some(llm);
    match user.save() {
        Ok(path) => tracing::debug!(path = %path.display(), "saved user LLM defaults"),
        Err(e) => tracing::warn!(error = %e, "cannot save user LLM defaults"),
    }
    Ok(())
}
