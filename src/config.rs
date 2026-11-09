use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_REPOSITORY: &str = "micrictor/bonsai-shellai";
pub const DEFAULT_MODEL_FILE: &str = "bonsai-shellai-Q4_K_S-MLP10-PLE_Q2_K-VOCAB_Q2_K.gguf";
pub const DEFAULT_MODEL_SHA256: &str =
    "d10317a1da0965d365a232500603863761f7ff76e1e04924f691ee080203049b";
pub const DEFAULT_MODEL_BYTES: u64 = 1_926_811_424;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// A local GGUF path. When set, repository and model_file are ignored.
    pub model_path: Option<PathBuf>,
    pub repository: String,
    pub model_file: String,
    pub model_ttl_seconds: u64,
    pub context_size: u32,
    pub max_new_tokens: u32,
    pub threads: Option<i32>,
    /// Requested GPU layers: 0 forces CPU; u32::MAX requests all layers automatically.
    pub gpu_layers: u32,
    /// Disable llama.cpp extra weight buffers (equivalent to --no-repack).
    #[serde(default)]
    pub no_repack: bool,
    /// Transformers-compatible sampling controls from generation_config.json.
    pub top_k: i32,
    pub top_p: f32,
    pub temperature: f32,
    pub repeat_penalty: f32,
    /// None requests a fresh random seed from llama.cpp for each sampler.
    pub seed: Option<u32>,
}

/// Per-inference settings sent by the client to an already-running server.
///
/// The model TTL is intentionally excluded: it controls the lifetime of the
/// server loop rather than an individual request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InferenceConfig {
    pub model_path: Option<PathBuf>,
    pub repository: String,
    pub model_file: String,
    pub context_size: u32,
    pub max_new_tokens: u32,
    pub threads: Option<i32>,
    /// Requested GPU layers: 0 forces CPU; u32::MAX requests all layers automatically.
    pub gpu_layers: u32,
    /// Disable llama.cpp extra weight buffers (equivalent to --no-repack).
    #[serde(default)]
    pub no_repack: bool,
    pub top_k: i32,
    pub top_p: f32,
    pub temperature: f32,
    pub repeat_penalty: f32,
    pub seed: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model_path: None,
            repository: DEFAULT_REPOSITORY.into(),
            model_file: DEFAULT_MODEL_FILE.into(),
            model_ttl_seconds: 60,
            // Match the measured bounded Gemma4 command protocol.
            context_size: 2048,
            max_new_tokens: 64,
            threads: Some(2),
            gpu_layers: u32::MAX,
            no_repack: true,
            top_k: 50,
            top_p: 1.0,
            // Greedy command decoding.
            temperature: 0.0,
            repeat_penalty: 1.0,
            seed: Some(42),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        let mut config = if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?
        } else {
            Self::default()
        };

        if let Some(value) = std::env::var_os("SHELLAI_MODEL") {
            config.model_path = Some(value.into());
        }
        if let Ok(value) = std::env::var("SHELLAI_MODEL_TTL") {
            config.model_ttl_seconds = value
                .parse()
                .context("SHELLAI_MODEL_TTL must be an integer number of seconds")?;
        }
        if let Ok(value) = std::env::var("SHELLAI_TOP_K") {
            config.top_k = value.parse().context("SHELLAI_TOP_K must be an integer")?;
        }
        if let Ok(value) = std::env::var("SHELLAI_TEMPERATURE") {
            config.temperature = value
                .parse()
                .context("SHELLAI_TEMPERATURE must be a number")?;
        }
        if let Ok(value) = std::env::var("SHELLAI_REPEAT_PENALTY") {
            config.repeat_penalty = value
                .parse()
                .context("SHELLAI_REPEAT_PENALTY must be a number")?;
        }
        config.model_path = config.model_path.map(std::path::absolute).transpose()?;
        Ok(config)
    }

    pub fn inference_config(&self) -> InferenceConfig {
        InferenceConfig {
            model_path: self.model_path.clone(),
            repository: self.repository.clone(),
            model_file: self.model_file.clone(),
            context_size: self.context_size,
            max_new_tokens: self.max_new_tokens,
            threads: self.threads,
            gpu_layers: self.gpu_layers,
            no_repack: self.no_repack,
            top_k: self.top_k,
            top_p: self.top_p,
            temperature: self.temperature,
            repeat_penalty: self.repeat_penalty,
            seed: self.seed,
        }
    }

    pub fn path() -> Result<PathBuf> {
        Ok(project_dirs()?.config_dir().join("config.toml"))
    }

    pub fn cache_dir() -> Result<PathBuf> {
        Ok(project_dirs()?.cache_dir().to_path_buf())
    }

    pub fn write_default_if_missing() -> Result<PathBuf> {
        let path = Self::path()?;
        if !path.exists() {
            let parent = path.parent().context("config path has no parent")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            let text = toml::to_string_pretty(&Self::default())?;
            fs::write(&path, text)
                .with_context(|| format!("failed to write {}", path.display()))?;
        }
        Ok(path)
    }
}

impl InferenceConfig {
    /// Applies client settings and reports whether the loaded model must be
    /// discarded. Context and sampler settings take effect on the next
    /// generation without reloading the model weights.
    pub fn apply_to(self, config: &mut Config) -> bool {
        let reload = config.model_path != self.model_path
            || config.repository != self.repository
            || config.model_file != self.model_file
            || config.gpu_layers != self.gpu_layers
            || config.no_repack != self.no_repack;

        config.model_path = self.model_path;
        config.repository = self.repository;
        config.model_file = self.model_file;
        config.context_size = self.context_size;
        config.max_new_tokens = self.max_new_tokens;
        config.threads = self.threads;
        config.gpu_layers = self.gpu_layers;
        config.no_repack = self.no_repack;
        config.top_k = self.top_k;
        config.top_p = self.top_p;
        config.temperature = self.temperature;
        config.repeat_penalty = self.repeat_penalty;
        config.seed = self.seed;
        reload
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("dev", "micrictor", "shellai")
        .context("could not determine the per-user shellai directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_config_updates_live_settings_and_detects_model_changes() {
        let mut server = Config::default();
        let mut request = server.inference_config();
        request.temperature = 0.1;
        request.top_k = 50;
        assert!(!request.apply_to(&mut server));
        assert_eq!(server.temperature, 0.1);
        assert_eq!(server.top_k, 50);

        let mut request = server.inference_config();
        request.model_path = Some("/tmp/alternate.gguf".into());
        assert!(request.apply_to(&mut server));
        assert_eq!(
            server.model_path,
            Some(PathBuf::from("/tmp/alternate.gguf"))
        );
    }
}
