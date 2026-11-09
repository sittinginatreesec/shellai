use std::{
    io::Read,
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use crate::protocol::ProgressEvent;
use anyhow::{Context, Result};
use hf_hub::api::{Progress, sync::ApiBuilder};
use llama_cpp_2::{
    LlamaBackendDevice, LlamaBackendDeviceType,
    context::params::LlamaContextParams,
    list_llama_ggml_backend_devices,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
};

use crate::config::{Config, InferenceConfig};

pub struct ModelService {
    config: Config,
    backend: LlamaBackend,
    model: Option<LlamaModel>,
    active_gpu: bool,
    backend_description: Option<String>,
}

pub struct Generation {
    pub text: Option<String>,
    pub raw_completion: String,
    pub parse_error: Option<String>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug)]
pub struct PartialCompletion {
    pub raw_completion: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl std::fmt::Display for PartialCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("inference failed after partial completion decoding")
    }
}

#[derive(Clone, Copy)]
pub struct GenerationOptions<'a> {
    pub current_line: Option<&'a str>,
    pub system_prompt: Option<&'a str>,
    pub assistant_prefix: Option<&'a str>,
    pub stop_after: Option<&'a str>,
}

impl ModelService {
    pub fn new(config: Config) -> Result<Self> {
        let backend = LlamaBackend::init().context("failed to initialize libllama")?;
        Ok(Self {
            config,
            backend,
            model: None,
            active_gpu: false,
            backend_description: None,
        })
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub fn context_size(&self) -> u32 {
        self.config.context_size
    }

    pub fn backend_description(&self) -> Option<&str> {
        self.backend_description.as_deref()
    }

    pub fn unload(&mut self) {
        self.model.take();
        self.active_gpu = false;
        self.backend_description = None;
    }

    pub fn apply_inference_config(&mut self, config: InferenceConfig) {
        if config.apply_to(&mut self.config) {
            self.unload();
        }
    }

    pub fn generate(
        &mut self,
        request: &str,
        options: GenerationOptions<'_>,
        progress: &mut dyn FnMut(ProgressEvent),
    ) -> Result<Generation> {
        self.ensure_loaded(progress)?;
        let model = self.model.as_ref().context("model was not loaded")?;
        let result = generate_command(model, &self.backend, &self.config, request, options);
        if let Err(error) = &result {
            if self.active_gpu && error.downcast_ref::<PartialCompletion>().is_none() {
                progress(ProgressEvent::Backend {
                    message: format!("GPU inference failed ({error:#}); retrying on CPU"),
                });
                self.unload();
                let path = resolve_model_path_with_progress(&self.config, progress)?;
                self.load_cpu(&path)?;
                return generate_command(
                    self.model.as_ref().unwrap(),
                    &self.backend,
                    &self.config,
                    request,
                    options,
                );
            }
        }
        result
    }

    fn load_cpu(&mut self, path: &Path) -> Result<()> {
        let params = model_params(&self.config)
            .with_n_gpu_layers(0)
            .with_devices(&[])?;
        self.model = Some(
            LlamaModel::load_from_file(&self.backend, path, &params)
                .with_context(|| format!("failed to load GGUF {} on CPU", path.display()))?,
        );
        self.active_gpu = false;
        self.backend_description = Some("CPU".into());
        Ok(())
    }

    fn ensure_loaded(&mut self, progress: &mut dyn FnMut(ProgressEvent)) -> Result<()> {
        if self.model.is_none() {
            let path = resolve_model_path_with_progress(&self.config, progress)?;
            let devices = list_llama_ggml_backend_devices();
            let device = select_gpu(&devices, self.config.gpu_layers);
            if let Some(device) = device {
                let params = model_params(&self.config).with_devices(&[device.index])?;
                progress(ProgressEvent::Backend {
                    message: format!(
                        "loading model on {} / {} (requested GPU layers: {})",
                        device.backend, device.description, self.config.gpu_layers
                    ),
                });
                match LlamaModel::load_from_file(&self.backend, &path, &params) {
                    Ok(model) => {
                        self.model = Some(model);
                        self.active_gpu = true;
                        self.backend_description = Some(format!(
                            "{} / {} (requested GPU layers: {})",
                            device.backend, device.description, self.config.gpu_layers
                        ));
                    }
                    Err(error) => {
                        progress(ProgressEvent::Backend {
                            message: format!(
                                "GPU model load failed ({error}); falling back to CPU"
                            ),
                        });
                        self.load_cpu(&path)?;
                    }
                }
            } else {
                let reason = if self.config.gpu_layers == 0 {
                    "GPU offload disabled"
                } else {
                    "no supported GPU available to this build"
                };
                progress(ProgressEvent::Backend {
                    message: format!("using CPU: {reason}"),
                });
                self.load_cpu(&path)?;
            }
        }
        Ok(())
    }
}

fn select_gpu(devices: &[LlamaBackendDevice], layers: u32) -> Option<&LlamaBackendDevice> {
    if layers == 0 {
        return None;
    }
    devices
        .iter()
        .filter(|device| {
            matches!(
                device.device_type,
                LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
            )
        })
        .filter(|device| {
            !device.description.to_lowercase().contains("llvmpipe")
                && !device.description.to_lowercase().contains("lavapipe")
        })
        .max_by_key(|device| device.memory_free)
}

pub fn print_devices() -> Result<()> {
    let _backend = LlamaBackend::init().context("failed to initialize compute backends")?;
    for device in list_llama_ggml_backend_devices() {
        println!(
            "{}: {} / {} ({:?}, {:.0} MiB free / {:.0} MiB total)",
            device.name,
            device.backend,
            device.description,
            device.device_type,
            device.memory_free as f64 / 1048576.0,
            device.memory_total as f64 / 1048576.0
        );
    }
    Ok(())
}

fn model_params(config: &Config) -> LlamaModelParams {
    LlamaModelParams::default()
        .with_n_gpu_layers(config.gpu_layers)
        .with_use_extra_bufts(!config.no_repack)
}

pub fn resolve_model_path(config: &Config) -> Result<PathBuf> {
    resolve_model_path_with_progress(config, &mut |event| eprintln!("{}", event.display()))
}

struct DownloadProgress<'a> {
    callback: &'a mut dyn FnMut(ProgressEvent),
    filename: String,
    downloaded: u64,
    total: u64,
    last_report: Option<std::time::Instant>,
}

impl DownloadProgress<'_> {
    fn report(&mut self, finished: bool) {
        (self.callback)(ProgressEvent::Download {
            filename: self.filename.clone(),
            downloaded: self.downloaded,
            total: self.total,
            finished,
        });
        self.last_report = Some(std::time::Instant::now());
    }
}

impl Progress for DownloadProgress<'_> {
    fn init(&mut self, size: usize, filename: &str) {
        self.filename = filename.into();
        self.total = size as u64;
        self.downloaded = 0;
        if self.last_report.is_none() {
            self.report(false);
        }
    }
    fn update(&mut self, size: usize) {
        self.downloaded = self.downloaded.saturating_add(size as u64).min(self.total);
        if self
            .last_report
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(1))
        {
            self.report(false);
        }
    }
    fn finish(&mut self) {
        self.downloaded = self.total;
        self.report(true);
    }
}

fn resolve_model_path_with_progress(
    config: &Config,
    progress: &mut dyn FnMut(ProgressEvent),
) -> Result<PathBuf> {
    if let Some(path) = &config.model_path {
        anyhow::ensure!(
            path.is_file(),
            "configured GGUF does not exist: {}",
            path.display()
        );
        return Ok(path.clone());
    }

    let (owner, name) = config.repository.split_once('/').with_context(|| {
        format!(
            "repository must have owner/name form, got {:?}",
            config.repository
        )
    })?;
    let client = ApiBuilder::from_env()
        .with_progress(true)
        .build()
        .context("failed to initialize the Hugging Face client")?;
    let repo_id = format!("{owner}/{name}");
    let cached = hf_hub::Cache::from_env()
        .model(repo_id.clone())
        .get(&config.model_file);
    let path = cached.map(Ok).unwrap_or_else(|| {
        // Announce before the metadata request, which can itself take time.
        progress(ProgressEvent::Download {
            filename: config.model_file.clone(), downloaded: 0, total: 0, finished: false,
        });
        client.model(repo_id).download_with_progress(&config.model_file, DownloadProgress {
            callback: progress, filename: config.model_file.clone(), downloaded: 0,
            total: 0, last_report: None,
        })
    })
        .with_context(|| {
            format!(
                "failed to download {}/{}; check network access and authenticate with HF_TOKEN or `hf auth login` if required",
                config.repository, config.model_file
            )
        })?;
    if config.repository == crate::config::DEFAULT_REPOSITORY
        && config.model_file == crate::config::DEFAULT_MODEL_FILE
    {
        verify_model_identity(
            &path,
            crate::config::DEFAULT_MODEL_BYTES,
            crate::config::DEFAULT_MODEL_SHA256,
        )?;
    }
    Ok(path)
}

fn verify_model_identity(path: &Path, bytes: u64, expected_sha256: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() == bytes,
        "published Bonsai model byte count differs"
    );
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual: String = digest
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    anyhow::ensure!(
        actual == expected_sha256,
        "published Bonsai model SHA256 differs"
    );
    Ok(())
}

fn generate_command(
    model: &LlamaModel,
    backend: &LlamaBackend,
    config: &Config,
    request: &str,
    options: GenerationOptions<'_>,
) -> Result<Generation> {
    {
        anyhow::ensure!(
            options.system_prompt.is_none()
                && options.assistant_prefix.is_none()
                && options.stop_after.is_none(),
            "Shellai support only the JSON command protocol"
        );
        anyhow::ensure!(
            model.meta_val_str("general.architecture").ok().as_deref() == Some("gemma4"),
            "Shellai requires a Gemma4 GGUF"
        );
        let turn = model.str_to_token("<turn|>", AddBos::Never)?;
        anyhow::ensure!(
            turn.len() == 1
                && turn[0].0 == crate::gemma4::END_TURN_TOKEN
                && model.is_eog_token(turn[0]),
            "Gemma4 GGUF must recognize <turn|> as an end-of-generation token"
        );
    }
    let prompt = crate::gemma4::render_prompt(request, options.current_line);
    let tokens = model
        .str_to_token(&prompt, AddBos::Never)
        .context("failed to tokenize the request")?;
    let prompt_tokens = u32::try_from(tokens.len()).context("prompt token count exceeds u32")?;
    let prefix = options.assistant_prefix.filter(|prefix| !prefix.is_empty());
    let prefix_tokens = prefix
        .map(|value| {
            model
                .str_to_token(value, AddBos::Never)
                .context("failed to tokenize the assistant prefix")
        })
        .transpose()?
        .unwrap_or_default();

    anyhow::ensure!(config.context_size > 0, "context_size must be nonzero");
    anyhow::ensure!(
        tokens.len() + prefix_tokens.len() < config.context_size as usize,
        "request is too long for the configured {} token context",
        config.context_size
    );
    {
        crate::gemma4::check_budget(tokens.len(), config.max_new_tokens, config.context_size)?;
        anyhow::ensure!(
            tokens.first() == Some(&model.token_bos()),
            "Gemma4 prompt must begin with its native BOS token"
        );
    }
    let required_context = tokens
        .len()
        .saturating_add(prefix_tokens.len())
        .saturating_add(config.max_new_tokens as usize);
    let rounded_context = required_context.saturating_add(511) / 512 * 512;
    let active_context = rounded_context.max(512).min(config.context_size as usize);
    let context_size = NonZeroU32::new(u32::try_from(active_context)?)
        .context("active context size must be nonzero")?;

    let mut context_params = LlamaContextParams::default().with_n_ctx(Some(context_size));
    if let Some(threads) = config.threads {
        context_params = context_params
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
    }
    let mut context = model
        .new_context(backend, context_params)
        .context("failed to create an inference context")?;
    const PROMPT_BATCH_TOKENS: usize = 2048;
    let mut batch = LlamaBatch::new(PROMPT_BATCH_TOKENS, 1);
    let last_index = tokens.len() - 1;
    for (chunk_index, chunk) in tokens.chunks(PROMPT_BATCH_TOKENS).enumerate() {
        batch.clear();
        let offset = chunk_index * PROMPT_BATCH_TOKENS;
        for (index, token) in chunk.iter().copied().enumerate() {
            let position = offset + index;
            batch.add(token, position as i32, &[0], position == last_index)?;
        }
        context
            .decode(&mut batch)
            .context("failed to evaluate the prompt")?;
    }

    anyhow::ensure!(config.top_k > 0, "top_k must be greater than zero");
    anyhow::ensure!(
        config.top_p > 0.0 && config.top_p <= 1.0,
        "top_p must be greater than zero and at most one"
    );
    anyhow::ensure!(
        config.temperature >= 0.0,
        "temperature must be zero or greater"
    );
    anyhow::ensure!(
        config.repeat_penalty > 0.0,
        "repeat_penalty must be greater than zero"
    );
    let mut sampler = if config.temperature == 0.0 {
        // Match Transformers' do_sample=False rather than relying on the
        // interaction between a zero temperature and a distribution sampler.
        LlamaSampler::chain_simple([
            LlamaSampler::penalties(-1, config.repeat_penalty, 0.0, 0.0),
            LlamaSampler::greedy(),
        ])
    } else {
        LlamaSampler::chain_simple([
            LlamaSampler::penalties(-1, config.repeat_penalty, 0.0, 0.0),
            LlamaSampler::top_k(config.top_k),
            LlamaSampler::top_p(config.top_p, 1),
            LlamaSampler::temp(config.temperature),
            // LLAMA_DEFAULT_SEED asks llama.cpp to select a random seed.
            LlamaSampler::dist(config.seed.unwrap_or(u32::MAX)),
        ])
    };
    for token in tokens.iter().copied() {
        sampler.accept(token);
    }
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut output = String::new();
    let mut position = i32::try_from(tokens.len()).context("prompt position exceeds i32")?;
    let mut completion_tokens = 0;

    if let Some(prefix) = prefix {
        anyhow::ensure!(
            position as usize + prefix_tokens.len() < active_context,
            "assistant prefix is too long for the configured context"
        );
        for token in prefix_tokens.iter().copied() {
            sampler.accept(token);
            batch.clear();
            batch.add(token, position, &[0], true)?;
            position += 1;
            completion_tokens += 1;
            context
                .decode(&mut batch)
                .context("failed to evaluate the assistant prefix")?;
        }
        output.push_str(prefix);
    }

    let generated_start = output.len();
    let available = u32::try_from(active_context)?.saturating_sub(position as u32);
    let token_limit = config
        .max_new_tokens
        .saturating_sub(completion_tokens)
        .min(available);

    for _ in 0..token_limit {
        let token = sampler.sample(&context, batch.n_tokens() - 1);
        sampler.accept(token);
        if (token.0 == crate::gemma4::END_TURN_TOKEN) || model.is_eog_token(token) {
            break;
        }
        completion_tokens += 1;
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .context("failed to decode a completion token")
            .map_err(|error| {
                error.context(PartialCompletion {
                    raw_completion: output[generated_start..].to_owned(),
                    prompt_tokens,
                    completion_tokens,
                })
            })?;
        output.push_str(&piece);
        if let Some(stop) = options.stop_after.filter(|stop| !stop.is_empty())
            && let Some(index) = output[generated_start..].find(stop)
        {
            output.truncate(generated_start + index + stop.len());
            break;
        }
        batch.clear();
        batch
            .add(token, position, &[0], true)
            .context("failed to queue a completion token")
            .map_err(|error| {
                error.context(PartialCompletion {
                    raw_completion: output[generated_start..].to_owned(),
                    prompt_tokens,
                    completion_tokens,
                })
            })?;
        position += 1;
        context
            .decode(&mut batch)
            .context("failed to generate a token")
            .map_err(|error| {
                error.context(PartialCompletion {
                    raw_completion: output[generated_start..].to_owned(),
                    prompt_tokens,
                    completion_tokens,
                })
            })?;
    }

    Ok(finish_generation(
        output,
        generated_start,
        prompt_tokens,
        completion_tokens,
    ))
}

fn finish_generation(
    output: String,
    generated_start: usize,
    prompt_tokens: u32,
    completion_tokens: u32,
) -> Generation {
    let raw_completion = output[generated_start..].to_owned();
    let parsed = crate::gemma4::parse_command(&output);
    let (text, parse_error) = match parsed {
        Ok(command) => (Some(command), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    Generation {
        text,
        raw_completion,
        parse_error,
        prompt_tokens,
        completion_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_model_integrity_rejects_wrong_size_or_hash() {
        let path = std::env::temp_dir().join(format!("shellai-integrity-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_model_identity(&path, 3, sha).is_ok());
        assert!(verify_model_identity(&path, 4, sha).is_err());
        assert!(verify_model_identity(&path, 3, "bad").is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn backend_failure_keeps_partial_completion_as_structured_context() {
        let raw = " \npartial ‘café’";
        let error = anyhow::anyhow!("backend failed").context(PartialCompletion {
            raw_completion: raw.to_owned(),
            prompt_tokens: 12,
            completion_tokens: 3,
        });
        let partial = error.downcast_ref::<PartialCompletion>().unwrap();
        assert_eq!(partial.raw_completion, raw);
        assert_eq!(partial.completion_tokens, 3);
        assert!(!error.to_string().contains(raw));
    }

    #[test]
    fn completion_survives_json_failure_without_whitespace_cleanup() {
        let raw = " \n{broken: ‘café’}\n\n";
        let generated = finish_generation(raw.to_owned(), 0, 12, 8);
        assert_eq!(generated.raw_completion, raw);
        assert!(generated.text.is_none());
        assert!(generated.parse_error.is_some());
        assert_eq!(generated.completion_tokens, 8);
    }

    #[test]
    fn successful_json_keeps_original_completion_separate_from_command() {
        let raw = " \n{\"command\":\"printf '‘café’'\"}\n\n";
        let generated = finish_generation(raw.to_owned(), 0, 10, 7);
        assert_eq!(generated.text.as_deref(), Some("printf '‘café’'"));
        assert_eq!(generated.raw_completion, raw);
        assert!(generated.parse_error.is_none());
        let empty = finish_generation(String::new(), 0, 10, 0);
        assert_eq!(empty.raw_completion, "");
        assert!(empty.parse_error.is_some());
    }

    /// Run after installing a compatible GGUF. Vocabulary-only loading keeps this
    /// exact HF/native tokenizer parity check small and CPU-only.
    #[test]
    #[ignore = "requires SHELLAI_MODEL local GGUF"]
    fn gemma4_gguf_tokenizer_matches_pinned_huggingface_ids() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/gemma4-prompt.json")).unwrap();
        let backend = LlamaBackend::init().unwrap();
        let path = std::env::var_os("SHELLAI_MODEL").expect("set SHELLAI_MODEL");
        let params = LlamaModelParams::default()
            .with_vocab_only(true)
            .with_n_gpu_layers(0);
        let model = LlamaModel::load_from_file(&backend, PathBuf::from(path), &params).unwrap();
        let prompt = crate::gemma4::render_prompt(fixture["request"].as_str().unwrap(), None);
        let actual: Vec<i32> = model
            .str_to_token(&prompt, AddBos::Never)
            .unwrap()
            .into_iter()
            .map(|token| token.0)
            .collect();
        let expected: Vec<i32> = serde_json::from_value(fixture["input_ids"].clone()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            model.str_to_token("<turn|>", AddBos::Never).unwrap()[0].0,
            crate::gemma4::END_TURN_TOKEN
        );
    }
}

#[cfg(test)]
mod repack_tests {
    #[test]
    fn no_repack_disables_native_extra_buffers() {
        let mut config = crate::config::Config::default();
        assert!(!super::model_params(&config).use_extra_bufts());
        config.no_repack = false;
        assert!(super::model_params(&config).use_extra_bufts());
        config.no_repack = true;
        assert!(!super::model_params(&config).use_extra_bufts());
        let mut daemon = crate::config::Config {
            no_repack: false,
            ..Default::default()
        };
        assert!(config.inference_config().apply_to(&mut daemon));
        assert!(daemon.no_repack);
        assert!(!config.inference_config().apply_to(&mut daemon));
    }
}

#[cfg(test)]
mod download_progress_tests {
    use super::*;
    #[test]
    fn resume_and_retry_reset_byte_counts_and_finish_at_total() {
        let mut events = Vec::new();
        {
            let mut callback = |event| events.push(event);
            let mut progress = DownloadProgress {
                callback: &mut callback,
                filename: String::new(),
                downloaded: 0,
                total: 0,
                last_report: None,
            };
            progress.init(100, "model");
            progress.update(40); // hf-hub reports already downloaded bytes on resume
            progress.update(10);
            assert_eq!(progress.downloaded, 50);
            progress.init(100, "model"); // retry starts again from the current file offset
            progress.update(50);
            assert_eq!(progress.downloaded, 50);
            progress.finish();
        }
        assert!(matches!(
            events.last(),
            Some(ProgressEvent::Download {
                downloaded: 100,
                total: 100,
                finished: true,
                ..
            })
        ));
    }
}

#[cfg(test)]
mod gpu_selection_tests {
    use super::*;

    fn device(
        index: usize,
        kind: LlamaBackendDeviceType,
        description: &str,
        free: usize,
    ) -> LlamaBackendDevice {
        LlamaBackendDevice {
            index,
            name: format!("device{index}"),
            description: description.into(),
            backend: "Vulkan".into(),
            memory_total: free,
            memory_free: free,
            device_type: kind,
        }
    }

    #[test]
    fn selects_real_gpu_with_most_available_memory_and_honors_cpu_override() {
        let devices = vec![
            device(0, LlamaBackendDeviceType::Cpu, "CPU", 1000),
            device(1, LlamaBackendDeviceType::Gpu, "llvmpipe (software)", 999),
            device(
                2,
                LlamaBackendDeviceType::IntegratedGpu,
                "Intel Iris Xe",
                100,
            ),
            device(3, LlamaBackendDeviceType::Gpu, "AMD Radeon", 200),
        ];
        assert_eq!(select_gpu(&devices, u32::MAX).unwrap().index, 3);
        assert_eq!(select_gpu(&devices[0..3], 1).unwrap().index, 2);
        assert!(select_gpu(&devices, 0).is_none());
        assert!(select_gpu(&devices[0..2], u32::MAX).is_none());
        assert!(select_gpu(&[], u32::MAX).is_none());
    }
}
