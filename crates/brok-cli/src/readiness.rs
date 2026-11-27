//! What works right now and the one thing to do next, for editors
//! (`brok status --json`): they show it without reading error text.
//!
//! The report holds states and fixed commands only, never keys or file
//! content.

use brok_core::Workspace;
use brok_index::{ModelFiles, PartitionStatus};
use brok_shard::SYSTEM_AGENT;
use serde::Serialize;

use crate::daemon::{self, AgentState, DaemonStatusReply};
use crate::local_llm::{self, ModelState};

/// Whether keyword search has something to answer with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TextState {
    Ready,
    NotIndexed,
    /// Indexed, with nothing in it.
    Empty,
    Unreadable,
}

/// Whether searches find by meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MeaningState {
    Ready,
    /// Chunks wait for their vectors.
    Embedding,
    /// Nothing is indexed with vectors.
    NotBuilt,
    NoModel,
    /// This build cannot run the model.
    Unsupported,
    Disabled,
}

/// What the embedding settings allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Embeddings {
    Disabled,
    Unsupported,
    NoModel,
    Usable,
}

/// Whether something answers `ask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Agent {
    Ready,
    /// Installed; `brok start`, or the first `ask`, brings it up.
    Stopped,
    Starting,
    NotConnected,
    Disabled,
    NotInstalled,
    MissingProgram,
    Incompatible,
}

#[derive(Debug, Serialize)]
struct Text {
    state: TextState,
}

#[derive(Debug, Serialize)]
struct Meaning {
    state: MeaningState,
    embedded: usize,
    chunks: usize,
}

#[derive(Debug, Serialize)]
struct AgentReport {
    state: Agent,
}

#[derive(Debug, Serialize)]
struct Model {
    state: ModelState,
}

/// The single suggested step. `code` is stable; `command` is absent when no
/// command does it.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Action {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

/// The `readiness` object of `brok status --json`.
#[derive(Debug, Serialize)]
pub struct Readiness {
    text_search: Text,
    meaning_search: Meaning,
    agent: AgentReport,
    model: Model,
    next_action: Option<Action>,
}

/// What was observed.
struct Observed<'a> {
    /// `None` when the index cannot be read.
    index: Option<&'a [PartitionStatus]>,
    embeddings: Embeddings,
    /// Why `brok-agent` may be absent; [`AgentState::NotConnected`] means it
    /// is installed and able to run.
    installed: AgentState,
    daemon_running: bool,
    /// `None` when the daemon is stopped or does not answer yet.
    daemon: Option<DaemonStatusReply>,
    model: ModelState,
    /// Provider name and model, to suggest fetching it.
    provider: (&'a str, &'a str),
}

/// Looks at the workspace. Only a model server on this machine is contacted.
pub async fn assess(workspace: &Workspace, index: Option<&[PartitionStatus]>) -> Readiness {
    let config = workspace.config();
    let embeddings = if !config.embeddings.enabled {
        Embeddings::Disabled
    } else if !brok_index::compiled_features().contains(&"onnx") {
        Embeddings::Unsupported
    } else if ModelFiles::locate(&workspace.paths().models, &config.embeddings).is_ok_and(|f| f.is_present()) {
        Embeddings::Usable
    } else {
        Embeddings::NoModel
    };
    let name = config.llm.default.as_str();
    let provider = config.llm.providers.get(name);
    let model = match provider {
        None => ModelState::NotConfigured,
        Some(p) if local_llm::is_this_machine(&p.base_url) || local_llm::missing_key(p) => local_llm::check(p).await,
        Some(_) => ModelState::Unchecked,
    };
    let daemon_running = daemon::is_running(workspace).await;
    let daemon = if daemon_running { daemon::status(workspace).await } else { None };
    report(Observed {
        index,
        embeddings,
        installed: AgentState::of(workspace),
        daemon_running,
        daemon,
        model,
        provider: (name, provider.map_or("", |p| p.model.as_str())),
    })
}

fn report(seen: Observed<'_>) -> Readiness {
    let parts = seen.index.unwrap_or_default();
    let chunks: usize = parts.iter().map(|p| p.chunks).sum();
    let embedded: usize = parts.iter().map(|p| p.embedded).sum();
    let waiting: usize = parts.iter().map(|p| p.waiting).sum();

    let text = match seen.index {
        None => TextState::Unreadable,
        Some([]) => TextState::NotIndexed,
        Some(_) if chunks == 0 => TextState::Empty,
        Some(_) => TextState::Ready,
    };
    let mut filled = parts.iter().filter(|p| p.chunks > 0);
    let meaning = match seen.embeddings {
        Embeddings::Disabled => MeaningState::Disabled,
        Embeddings::Unsupported => MeaningState::Unsupported,
        Embeddings::NoModel => MeaningState::NoModel,
        Embeddings::Usable if chunks > 0 && filled.all(|p| p.semantic) => MeaningState::Ready,
        Embeddings::Usable if waiting > 0 => MeaningState::Embedding,
        Embeddings::Usable => MeaningState::NotBuilt,
    };
    let agent = agent_state(seen.installed, seen.daemon_running, seen.daemon.as_ref());

    let action = |code, command: &str| Some(Action { code, command: Some(command.to_owned()) });
    let (provider, model_name) = seen.provider;
    let next_action = if text == TextState::Unreadable {
        action("rebuild_index", "brok index --rebuild")
    } else if meaning == MeaningState::NoModel {
        action("download_embedding_model", "brok model download")
    } else if text == TextState::NotIndexed && !seen.daemon_running {
        action("index_workspace", "brok index")
    } else if meaning == MeaningState::NotBuilt && chunks > 0 {
        // A running daemon rebuilds when it starts with the model.
        if seen.daemon_running { action("restart", "brok restart") } else { action("finish_indexing", "brok index") }
    } else if waiting > 0 && !seen.daemon_running && seen.embeddings == Embeddings::Usable {
        action("finish_indexing", "brok index")
    } else if agent == Agent::NotInstalled {
        action("install_agent", "brok shard install https://github.com/brokspace/brok-agent")
    } else if agent == Agent::MissingProgram {
        action("repair_agent", "brok init")
    } else if agent == Agent::Incompatible {
        action("upgrade", "brok upgrade")
    } else if agent == Agent::NotConnected {
        action("check_logs", "brok logs")
    } else if seen.model == ModelState::MissingKey {
        Some(Action { code: "set_api_key", command: None })
    } else if seen.model == ModelState::NotInstalled && provider == "ollama" && is_plain_name(model_name) {
        action("pull_model", &format!("ollama pull {model_name}"))
    } else if matches!(seen.model, ModelState::NotConfigured | ModelState::Unreachable | ModelState::NotInstalled) {
        action("setup_model", "brok setup llm")
    } else if agent == Agent::Stopped {
        action("start", "brok start")
    } else {
        None
    };

    Readiness {
        text_search: Text { state: text },
        meaning_search: Meaning { state: meaning, embedded, chunks },
        agent: AgentReport { state: agent },
        model: Model { state: seen.model },
        next_action,
    }
}

fn agent_state(installed: AgentState, running: bool, daemon: Option<&DaemonStatusReply>) -> Agent {
    // Another shard may answer instead of `brok-agent`.
    if daemon.is_some_and(|d| d.answers_asks == Some(true)) {
        return Agent::Ready;
    }
    match installed {
        AgentState::Disabled => Agent::Disabled,
        AgentState::NotInstalled => Agent::NotInstalled,
        AgentState::NoBinary => Agent::MissingProgram,
        AgentState::Incompatible => Agent::Incompatible,
        AgentState::NotConnected => match daemon {
            _ if !running => Agent::Stopped,
            None => Agent::Starting,
            Some(d) if d.starting.iter().any(|s| s == SYSTEM_AGENT) => Agent::Starting,
            // A daemon from before `answers_asks` lists who is connected.
            Some(d) if d.answers_asks.is_none() && d.identities.iter().any(|i| i == SYSTEM_AGENT) => Agent::Ready,
            Some(_) => Agent::NotConnected,
        },
    }
}

/// Whether a model name from `brok.yml`, which a repository can share, is
/// safe inside a command someone may run.
fn is_plain_name(name: &str) -> bool {
    !name.starts_with('-') && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(embedded: usize, waiting: usize, semantic: bool) -> PartitionStatus {
        PartitionStatus {
            id: "root".into(),
            path: String::new(),
            tier: brok_core::Tier::Cold,
            backend: "arrow",
            files: 1,
            chunks: embedded + waiting,
            embedded,
            waiting,
            semantic,
        }
    }

    /// Everything works.
    fn good(index: &[PartitionStatus]) -> Observed<'_> {
        Observed {
            index: Some(index),
            embeddings: Embeddings::Usable,
            installed: AgentState::NotConnected,
            daemon_running: true,
            daemon: Some(DaemonStatusReply { answers_asks: Some(true), ..Default::default() }),
            model: ModelState::Ready,
            provider: ("ollama", "qwen2.5-coder:7b"),
        }
    }

    fn next(seen: Observed<'_>) -> Option<(&'static str, Option<String>)> {
        report(seen).next_action.map(|a| (a.code, a.command))
    }

    fn code(seen: Observed<'_>) -> Option<&'static str> {
        next(seen).map(|(code, _)| code)
    }

    fn stopped(index: &[PartitionStatus]) -> Observed<'_> {
        Observed { daemon_running: false, daemon: None, ..good(index) }
    }

    #[test]
    fn nothing_to_do_when_everything_works() {
        let index = [part(10, 0, true)];
        let ready = report(good(&index));
        assert_eq!(ready.text_search.state, TextState::Ready);
        assert_eq!(ready.meaning_search.state, MeaningState::Ready);
        assert_eq!(ready.agent.state, Agent::Ready);
        assert_eq!(ready.next_action, None);
        assert_eq!(code(stopped(&index)), Some("start"));
    }

    #[test]
    fn the_index_comes_first() {
        let unreadable = Observed { index: None, model: ModelState::Unreachable, ..good(&[]) };
        assert_eq!(report(Observed { index: None, ..good(&[]) }).text_search.state, TextState::Unreadable);
        assert_eq!(code(unreadable), Some("rebuild_index"));
        assert_eq!(report(stopped(&[])).text_search.state, TextState::NotIndexed);
        assert_eq!(code(stopped(&[])), Some("index_workspace"));
        assert_eq!(code(good(&[])), None, "a running daemon is indexing");
        let empty = [part(0, 0, false)];
        assert_eq!(report(good(&empty)).text_search.state, TextState::Empty);
        assert_eq!(code(good(&empty)), None);
        let no_model = Observed { embeddings: Embeddings::NoModel, ..stopped(&[]) };
        assert_eq!(code(no_model), Some("download_embedding_model"), "indexing would fail without it");
    }

    #[test]
    fn meaning_search_states() {
        let waiting = [part(10, 90, false)];
        let seen = report(stopped(&waiting));
        assert_eq!(seen.meaning_search.state, MeaningState::Embedding);
        assert_eq!((seen.meaning_search.embedded, seen.meaning_search.chunks), (10, 100));
        assert_eq!(code(stopped(&waiting)), Some("finish_indexing"));
        assert_eq!(code(good(&waiting)), None, "the daemon carries on");

        // Enough to search by meaning, with a few still waiting.
        let almost = [part(99, 1, true)];
        assert_eq!(report(good(&almost)).meaning_search.state, MeaningState::Ready);
        assert_eq!(code(stopped(&almost)), Some("finish_indexing"));

        // Indexed without vectors.
        let text_only = [PartitionStatus { chunks: 5, ..part(0, 0, false) }];
        assert_eq!(report(good(&text_only)).meaning_search.state, MeaningState::NotBuilt);
        assert_eq!(code(good(&text_only)), Some("restart"));
        assert_eq!(code(stopped(&text_only)), Some("finish_indexing"));

        for (embeddings, state) in
            [(Embeddings::Disabled, MeaningState::Disabled), (Embeddings::Unsupported, MeaningState::Unsupported)]
        {
            let seen = Observed { embeddings, ..good(&text_only) };
            assert_eq!(report(seen).meaning_search.state, state);
            assert_eq!(code(Observed { embeddings, ..good(&text_only) }), None, "a choice, not a problem");
        }
    }

    #[test]
    fn agent_states_and_their_next_step() {
        let reply = |starting: &[&str], identities: &[&str], answers_asks| DaemonStatusReply {
            starting: starting.iter().map(|s| (*s).to_owned()).collect(),
            identities: identities.iter().map(|s| (*s).to_owned()).collect(),
            answers_asks,
            ..Default::default()
        };
        let installed = AgentState::NotConnected;
        assert_eq!(agent_state(installed, false, None), Agent::Stopped);
        assert_eq!(agent_state(installed, true, None), Agent::Starting);
        assert_eq!(agent_state(installed, true, Some(&reply(&[SYSTEM_AGENT], &[], Some(false)))), Agent::Starting);
        assert_eq!(agent_state(installed, true, Some(&reply(&[], &[], Some(false)))), Agent::NotConnected);
        assert_eq!(agent_state(installed, true, Some(&reply(&[], &[SYSTEM_AGENT], None))), Agent::Ready);
        assert_eq!(agent_state(AgentState::Disabled, true, Some(&reply(&[], &[], Some(true)))), Agent::Ready);

        let index = [part(10, 0, true)];
        for (installed, state, expected) in [
            (AgentState::NotInstalled, Agent::NotInstalled, Some("install_agent")),
            (AgentState::NoBinary, Agent::MissingProgram, Some("repair_agent")),
            (AgentState::Incompatible, Agent::Incompatible, Some("upgrade")),
            (AgentState::Disabled, Agent::Disabled, None),
            (AgentState::NotConnected, Agent::Stopped, Some("start")),
        ] {
            assert_eq!(report(Observed { installed, ..stopped(&index) }).agent.state, state);
            assert_eq!(code(Observed { installed, ..stopped(&index) }), expected);
        }
        let silent = Observed { daemon: Some(reply(&[], &[], Some(false))), ..good(&index) };
        assert_eq!(code(silent), Some("check_logs"));
    }

    #[test]
    fn model_states_and_their_next_step() {
        let index = [part(10, 0, true)];
        let with = |model, provider| Observed { model, provider, ..good(&index) };
        let ollama = ("ollama", "qwen2.5-coder:7b");
        assert_eq!(
            next(with(ModelState::NotInstalled, ollama)),
            Some(("pull_model", Some("ollama pull qwen2.5-coder:7b".to_owned())))
        );
        for unsafe_name in ["x; rm -rf ~", "--help", "$(id)", ""] {
            assert_eq!(code(with(ModelState::NotInstalled, ("ollama", unsafe_name))), Some("setup_model"));
        }
        assert_eq!(code(with(ModelState::NotInstalled, ("lmstudio", "m"))), Some("setup_model"));
        assert_eq!(code(with(ModelState::Unreachable, ollama)), Some("setup_model"));
        assert_eq!(code(with(ModelState::NotConfigured, ollama)), Some("setup_model"));
        assert_eq!(next(with(ModelState::MissingKey, ollama)), Some(("set_api_key", None)));
        assert_eq!(code(with(ModelState::Unchecked, ollama)), None);
    }
}
