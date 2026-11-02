//! `brok config`: read and change `brok.yml` from scripts and editors.
//!
//! Keys are dotted paths (`search.limit`, `llm.providers.ollama.model`).
//! Values are YAML (`true`, `0.8`, `[a, b]`, text). Every change is
//! validated before it is written, and the template's comments are kept.

use std::path::Path;

use anyhow::{Context, Result, bail};
use brok_core::BrokConfig;
use clap::Subcommand;
use serde_json::Value;

use crate::{daemon, ui};

/// `brok config` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the configuration, or one key of it.
    Get(GetArgs),
    /// Change one key.
    Set(SetArgs),
    /// Remove one key, so its default applies.
    Unset(UnsetArgs),
    /// Print the path of brok.yml.
    Path,
}

/// Arguments of `brok config get`.
#[derive(Debug, clap::Args)]
pub struct GetArgs {
    /// Dotted key; the whole configuration when omitted.
    key: Option<String>,
    /// Print JSON instead of YAML.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok config set`.
#[derive(Debug, clap::Args)]
pub struct SetArgs {
    /// Dotted key, e.g. `llm.default` or `search.limit`.
    key: String,
    /// New value, as YAML.
    value: String,
}

/// Arguments of `brok config unset`.
#[derive(Debug, clap::Args)]
pub struct UnsetArgs {
    /// Dotted key.
    key: String,
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let mut workspace = super::workspace(cwd)?;
    match command {
        Command::Path => {
            println!("{}", workspace.paths().config_file.display());
            Ok(())
        }
        Command::Get(args) => {
            let value = tidy(serde_json::to_value(workspace.config())?);
            let value = match &args.key {
                Some(key) => lookup(&value, key)?.clone(),
                None => value,
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                match &value {
                    Value::String(text) => println!("{text}"),
                    other => print!("{}", serde_yaml_ng::to_string(other)?),
                }
            }
            Ok(())
        }
        Command::Set(args) => {
            let value: Value = serde_yaml_ng::from_str(&args.value)
                .with_context(|| format!("`{}` is not a valid YAML value", args.value))?;
            let config = edit(workspace.config(), &args.key, Some(value))?;
            workspace.save_config(config)?;
            ui::success(&format!("{} updated", args.key));
            hint_restart(&workspace).await;
            Ok(())
        }
        Command::Unset(args) => {
            let config = edit(workspace.config(), &args.key, None)?;
            workspace.save_config(config)?;
            ui::success(&format!("{} reset to its default", args.key));
            hint_restart(&workspace).await;
            Ok(())
        }
    }
}

/// `config` with `key` set to `value` (removed when `None`), validated.
pub fn edit(config: &BrokConfig, key: &str, value: Option<Value>) -> Result<BrokConfig> {
    let parts: Vec<&str> = key.split('.').filter(|p| !p.is_empty()).collect();
    let Some((last, parents)) = parts.split_last() else { bail!("the key is empty") };
    let mut root = serde_json::to_value(config)?;
    let mut node = &mut root;
    for part in parents {
        let map = node.as_object_mut().with_context(|| format!("`{key}`: `{part}` is not a section"))?;
        node = map.entry((*part).to_owned()).or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    let map = node.as_object_mut().with_context(|| format!("`{key}` is not inside a section"))?;
    match value {
        Some(value) => {
            map.insert((*last).to_owned(), value);
        }
        None => {
            map.remove(*last);
        }
    }
    let config: BrokConfig = serde_json::from_value(root).map_err(|e| anyhow::anyhow!("`{key}`: {e}"))?;
    config.validate()?;
    Ok(config)
}

/// Weights are `f32`: print `0.85`, not the `f64` widening `0.8500000238418579`.
fn tidy(value: Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => {
            let short = n.as_f64().map(|f| (f as f32).to_string().parse::<f64>().unwrap_or(f));
            short.and_then(serde_json::Number::from_f64).map_or(Value::Number(n), Value::Number)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(tidy).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, tidy(v))).collect()),
        other => other,
    }
}

fn lookup<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    key.split('.')
        .filter(|p| !p.is_empty())
        .try_fold(value, |node, part| node.get(part))
        .with_context(|| format!("no key `{key}` in brok.yml"))
}

async fn hint_restart(workspace: &brok_core::Workspace) {
    if daemon::is_running(workspace).await {
        ui::hint("the background brok uses the old settings until `brok restart`");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_unset_and_validate() {
        let config = BrokConfig::default();
        let changed = edit(&config, "search.limit", Some(Value::from(12))).unwrap();
        assert_eq!(changed.search.limit, 12);
        let changed = edit(&changed, "llm.providers.ollama.model", Some(Value::from("qwen3:8b"))).unwrap();
        assert_eq!(changed.llm.providers["ollama"].model, "qwen3:8b");
        let changed = edit(&changed, "search.limit", None).unwrap();
        assert_eq!(changed.search.limit, 8);

        assert!(edit(&config, "search.nope", Some(Value::from(1))).unwrap_err().to_string().contains("nope"));
        assert!(edit(&config, "search.cold_weight", Some(Value::from(3))).is_err());
        assert!(edit(&config, "llm.default", Some(Value::from("missing"))).is_err());
        let value = tidy(serde_json::to_value(&config).unwrap());
        assert_eq!(lookup(&value, "memory.enabled").unwrap(), &Value::Bool(true));
        assert_eq!(lookup(&value, "search.cold_weight").unwrap().to_string(), "0.85");
    }
}
