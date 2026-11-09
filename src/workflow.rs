use anyhow::{Context, Result};
use serde::Serialize;
use std::time::{Duration, Instant};

use crate::{
    config::{Config, InferenceConfig},
    metrics,
    protocol::{InferenceMetrics, Request, Response},
    transport,
};

pub fn run(
    prompt: &str,
    context: Option<&str>,
    cold_start: bool,
    config: &Config,
) -> Result<String> {
    let mut run = WorkflowRun::new(config.inference_config());
    let result = run.zero_shot(prompt, context, cold_start);
    run.finish(result.is_ok());
    result
}

struct WorkflowRun {
    workflow_id: String,
    started: Instant,
    inference_count: u32,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    total_context_tokens: u64,
    server_inference_ms: u64,
    peak_server_rss_bytes: u64,
    discovery_command_ms: u64,
    help_command_ms: u64,
    inference_config: InferenceConfig,
}

struct InferenceOptions<'a> {
    context: Option<&'a str>,
    system_prompt: Option<&'a str>,
    assistant_prefix: Option<&'a str>,
    stop_after: Option<&'a str>,
    stage: &'a str,
    cold_start: bool,
}

#[derive(Serialize)]
struct ClientMetrics<'a> {
    workflow_id: &'a str,
    mode: &'static str,
    success: bool,
    inference_count: u32,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    total_context_tokens: u64,
    server_inference_ms: u64,
    peak_server_rss_bytes: u64,
    discovery_command_ms: u64,
    help_command_ms: u64,
    end_to_end_ms: u64,
    recorded_at_unix_ms: u128,
}

impl WorkflowRun {
    fn new(inference_config: InferenceConfig) -> Self {
        Self {
            workflow_id: metrics::new_id("workflow"),
            started: Instant::now(),
            inference_count: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_context_tokens: 0,
            server_inference_ms: 0,
            peak_server_rss_bytes: 0,
            discovery_command_ms: 0,
            help_command_ms: 0,
            inference_config,
        }
    }

    fn zero_shot(
        &mut self,
        prompt: &str,
        context: Option<&str>,
        cold_start: bool,
    ) -> Result<String> {
        self.infer(
            prompt,
            InferenceOptions {
                context,
                system_prompt: None,
                assistant_prefix: None,
                stop_after: None,
                stage: "zero_shot",
                cold_start,
            },
        )
    }

    fn infer_response(
        &mut self,
        prompt: &str,
        options: InferenceOptions<'_>,
        include_raw: bool,
        force_unload: bool,
    ) -> Result<Response> {
        let request = Request::Generate {
            prompt: prompt.to_owned(),
            include_raw,
            force_unload,
            context: options.context.map(str::to_owned),
            system_prompt: options.system_prompt.map(str::to_owned),
            workflow_id: Some(self.workflow_id.clone()),
            stage: Some(options.stage.to_owned()),
            assistant_prefix: options.assistant_prefix.map(str::to_owned),
            stop_after: options.stop_after.map(str::to_owned),
            inference_config: Some(Box::new(self.inference_config.clone())),
        };
        let response = transport::request(&request, options.cold_start)?;
        if let Some(metric) = &response.metrics {
            self.record_inference(metric);
        }
        Ok(response)
    }

    fn infer(&mut self, prompt: &str, options: InferenceOptions<'_>) -> Result<String> {
        let response = self.infer_response(prompt, options, false, false)?;
        anyhow::ensure!(
            response.ok,
            "{}",
            response.error.unwrap_or_else(|| "inference failed".into())
        );
        response
            .command
            .filter(|command| !command.trim().is_empty())
            .context("the model returned an empty response")
    }

    fn record_inference(&mut self, metric: &InferenceMetrics) {
        self.inference_count += 1;
        self.total_prompt_tokens += u64::from(metric.prompt_tokens);
        self.total_completion_tokens += u64::from(metric.completion_tokens);
        self.total_context_tokens += u64::from(metric.total_tokens);
        self.server_inference_ms = self.server_inference_ms.saturating_add(metric.inference_ms);
        self.peak_server_rss_bytes = self.peak_server_rss_bytes.max(metric.peak_rss_bytes);
    }

    fn finish(&self, success: bool) {
        let value = ClientMetrics {
            workflow_id: &self.workflow_id,
            mode: "zero_shot",
            success,
            inference_count: self.inference_count,
            total_prompt_tokens: self.total_prompt_tokens,
            total_completion_tokens: self.total_completion_tokens,
            total_context_tokens: self.total_context_tokens,
            server_inference_ms: self.server_inference_ms,
            peak_server_rss_bytes: self.peak_server_rss_bytes,
            discovery_command_ms: self.discovery_command_ms,
            help_command_ms: self.help_command_ms,
            end_to_end_ms: duration_ms(self.started.elapsed()),
            recorded_at_unix_ms: metrics::unix_time_ms(),
        };
        if let Err(error) = metrics::append_client(&value) {
            eprintln!("shellai: failed to record client metrics: {error:#}");
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}
