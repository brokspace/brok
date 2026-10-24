//! Local LLM discovery and model recommendation.
//!
//! Brokspace works with any OpenAI-compatible endpoint, and is tuned for the
//! two most common local servers: Ollama (`localhost:11434`) and LM Studio
//! (`localhost:1234`). The hardware tiers follow the local LLM guide in
//! `docs/local-llm.md`.

use std::time::Duration;

use serde::Deserialize;

/// A local server that answered `GET /v1/models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// Provider key in `brok.yml` (`ollama` or `lmstudio`).
    pub provider: &'static str,
    /// Human name.
    pub label: &'static str,
    /// OpenAI-compatible base URL.
    pub base_url: &'static str,
    /// Models the server can serve.
    pub models: Vec<String>,
}

/// Known local servers, in order of preference.
pub const KNOWN: [(&str, &str, &str); 2] =
    [("ollama", "Ollama", "http://localhost:11434/v1"), ("lmstudio", "LM Studio", "http://localhost:1234/v1")];

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// Lists the models of an OpenAI-compatible endpoint (`None` when it does
/// not answer within a second or two).
pub async fn list_models(base_url: &str) -> Option<Vec<String>> {
    let client = reqwest::Client::builder().timeout(Duration::from_millis(1500)).build().ok()?;
    let response = client.get(format!("{}/models", base_url.trim_end_matches('/'))).send().await.ok()?;
    let list: ModelList = response.error_for_status().ok()?.json().await.ok()?;
    let mut models: Vec<String> = list.data.into_iter().map(|m| m.id).collect();
    models.sort();
    Some(models)
}

/// Probes every known local server concurrently.
pub async fn probe() -> Vec<Server> {
    let probes = KNOWN.map(|(provider, label, base_url)| async move {
        list_models(base_url).await.map(|models| Server { provider, label, base_url, models })
    });
    futures_join(probes).await.into_iter().flatten().collect()
}

async fn futures_join<F: std::future::Future>(futures: [F; 2]) -> [F::Output; 2] {
    let [a, b] = futures;
    let (a, b) = tokio::join!(a, b);
    [a, b]
}

/// What the machine can run comfortably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    /// Total system memory in GiB.
    pub ram_gb: u64,
    /// Dedicated NVIDIA GPU memory in GiB, when detectable.
    pub vram_gb: Option<u64>,
    /// Apple Silicon: memory is unified, so RAM doubles as VRAM.
    pub apple_silicon: bool,
}

/// Reads total memory and, when `nvidia-smi` is available, GPU memory.
pub fn detect_hardware() -> Hardware {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let ram_gb = system.total_memory() / (1024 * 1024 * 1024);
    let vram_gb = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse::<u64>().ok()).max())
        .map(|mib| mib / 1024);
    let apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    Hardware { ram_gb, vram_gb, apple_silicon }
}

/// A model size class from the local LLM guide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier {
    /// e.g. "7B to 9B".
    pub size: &'static str,
    /// Recommended Ollama model.
    pub ollama_model: &'static str,
    /// What to search for in LM Studio.
    pub lmstudio_hint: &'static str,
    /// Good alternatives.
    pub alternatives: &'static [&'static str],
    /// Why this tier was picked.
    pub reason: String,
}

/// Picks the largest comfortable tier: 14B-class with 32 GB RAM or 12 GB
/// VRAM, 7B-class with 16 GB RAM or 6 GB VRAM, otherwise 1.5B-3B.
pub fn recommend(hardware: Hardware) -> Tier {
    let vram = hardware.vram_gb.unwrap_or(0);
    // Reported totals are slightly below the marketing size (16 GB -> 15.x GiB).
    let (ram_mid, ram_high) = (15, 30);
    let memory = if hardware.apple_silicon { "unified memory" } else { "RAM" };
    if vram >= 12 || hardware.ram_gb >= ram_high {
        Tier {
            size: "14B to 32B",
            ollama_model: "qwen2.5-coder:14b",
            lmstudio_hint: "Qwen2.5-Coder-14B-Instruct (Q4_K_M)",
            alternatives: &["deepseek-r1:32b", "qwen2.5-coder:7b"],
            reason: format!("{} GB {memory}{}", hardware.ram_gb, vram_note(vram)),
        }
    } else if vram >= 6 || hardware.ram_gb >= ram_mid {
        Tier {
            size: "7B to 9B",
            ollama_model: "qwen2.5-coder:7b",
            lmstudio_hint: "Qwen2.5-Coder-7B-Instruct (Q4_K_M)",
            alternatives: &["llama3:8b", "deepseek-r1:8b"],
            reason: format!("{} GB {memory}{}", hardware.ram_gb, vram_note(vram)),
        }
    } else {
        Tier {
            size: "1B to 3B",
            ollama_model: "qwen2.5-coder:1.5b",
            lmstudio_hint: "Qwen2.5-Coder-1.5B-Instruct (Q4_K_M)",
            alternatives: &["llama3.2:3b"],
            reason: format!("{} GB {memory}{}", hardware.ram_gb, vram_note(vram)),
        }
    }
}

fn vram_note(vram: u64) -> String {
    if vram > 0 { format!(", {vram} GB GPU memory") } else { String::new() }
}

/// Whether `models` already contains `wanted` (Ollama adds `:latest`).
pub fn has_model(models: &[String], wanted: &str) -> bool {
    models.iter().any(|m| m == wanted || m.strip_suffix(":latest") == Some(wanted))
}

/// Picks the best installed model: the recommended one, else any coder
/// model, else the first one.
pub fn pick_installed(models: &[String], recommended: &str) -> Option<String> {
    if has_model(models, recommended) {
        return Some(recommended.to_owned());
    }
    models.iter().find(|m| m.contains("coder")).or_else(|| models.first()).cloned()
}

/// How to install a local server on this OS.
pub fn install_instructions() -> Vec<String> {
    let ollama = if cfg!(windows) {
        "winget install Ollama.Ollama        (or download from https://ollama.com)"
    } else if cfg!(target_os = "macos") {
        "brew install ollama                 (or download from https://ollama.com)"
    } else {
        "curl -fsSL https://ollama.com/install.sh | sh"
    };
    vec![
        format!("Ollama (recommended): {ollama}"),
        "LM Studio (graphical): https://lmstudio.ai, then Developer > Start Server".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(ram_gb: u64, vram_gb: Option<u64>) -> Hardware {
        Hardware { ram_gb, vram_gb, apple_silicon: false }
    }

    #[test]
    fn tiers_follow_the_local_llm_guide() {
        assert_eq!(recommend(hw(7, None)).ollama_model, "qwen2.5-coder:1.5b");
        assert_eq!(recommend(hw(15, None)).ollama_model, "qwen2.5-coder:7b");
        assert_eq!(recommend(hw(8, Some(8))).ollama_model, "qwen2.5-coder:7b");
        assert_eq!(recommend(hw(31, None)).ollama_model, "qwen2.5-coder:14b");
        assert_eq!(recommend(hw(16, Some(12))).ollama_model, "qwen2.5-coder:14b");
    }

    #[test]
    fn model_matching() {
        let models = vec!["llama3:8b".to_owned(), "qwen2.5-coder:7b".to_owned()];
        assert!(has_model(&models, "qwen2.5-coder:7b"));
        assert!(has_model(&["qwen2.5-coder:7b:latest".to_owned()], "qwen2.5-coder:7b"));
        assert_eq!(pick_installed(&models, "qwen2.5-coder:14b").unwrap(), "qwen2.5-coder:7b");
        assert_eq!(pick_installed(&["llama3:8b".to_owned()], "x").unwrap(), "llama3:8b");
        assert!(pick_installed(&[], "x").is_none());
    }
}
