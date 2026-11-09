use serde::{Deserialize, Serialize};

use crate::config::InferenceConfig;

pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Generate {
        prompt: String,
        /// Request decoded completion text before command parsing/cleanup.
        #[serde(default)]
        include_raw: bool,
        /// Explicitly release resident weights before this generation.
        #[serde(default)]
        force_unload: bool,
        context: Option<String>,
        #[serde(default)]
        system_prompt: Option<String>,
        #[serde(default)]
        workflow_id: Option<String>,
        #[serde(default)]
        stage: Option<String>,
        #[serde(default)]
        assistant_prefix: Option<String>,
        #[serde(default)]
        stop_after: Option<String>,
        #[serde(default)]
        inference_config: Option<Box<InferenceConfig>>,
    },
    Status,
    Shutdown,
}

/// Intermediate IPC frame; the final frame remains a Response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Backend {
        message: String,
    },
    Download {
        filename: String,
        downloaded: u64,
        total: u64,
        finished: bool,
    },
}

impl ProgressEvent {
    pub fn display(&self) -> String {
        match self {
            Self::Backend { message } => format!("shellai: {message}"),
            Self::Download {
                filename,
                downloaded,
                total,
                finished,
            } => {
                if *total == 0 {
                    return format!(
                        "shellai: preparing model download {filename} (checking size/cache)"
                    );
                }
                let percent = if *total == 0 {
                    0
                } else {
                    downloaded
                        .saturating_mul(100)
                        .checked_div(*total)
                        .unwrap_or(0)
                        .min(100)
                };
                let state = if *finished {
                    "download complete; verifying model"
                } else {
                    "downloading model"
                };
                format!("shellai: {state} {filename}: {percent}% ({downloaded}/{total} bytes)")
            }
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_completion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub model_loaded: bool,
    /// Whether the model object was already loaded after applying settings and reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_was_loaded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<InferenceMetrics>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InferenceMetrics {
    pub request_id: String,
    pub workflow_id: Option<String>,
    pub stage: String,
    pub success: bool,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub context_limit: u32,
    pub inference_ms: u64,
    pub rss_before_bytes: u64,
    pub rss_after_bytes: u64,
    pub peak_rss_bytes: u64,
    pub recorded_at_unix_ms: u128,
}

impl Response {
    pub fn success(
        command: Option<String>,
        model_loaded: bool,
        metrics: Option<InferenceMetrics>,
    ) -> Self {
        Self {
            backend: None,
            ok: true,
            command,
            raw_completion: None,
            error: None,
            model_loaded,
            model_was_loaded: None,
            metrics,
        }
    }

    pub fn failure(
        error: impl Into<String>,
        model_loaded: bool,
        metrics: Option<InferenceMetrics>,
    ) -> Self {
        Self {
            backend: None,
            ok: false,
            command: None,
            raw_completion: None,
            error: Some(error.into()),
            model_loaded,
            model_was_loaded: None,
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_completion_round_trips_on_failure_and_is_optional() {
        let raw = " \n{broken: ‘café’}\n";
        let mut response = Response::failure("invalid JSON", true, None);
        response.raw_completion = Some(raw.into());
        response.model_was_loaded = Some(true);
        let decoded: Response =
            serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
        assert!(!decoded.ok);
        assert_eq!(decoded.model_was_loaded, Some(true));
        assert_eq!(decoded.raw_completion.as_deref(), Some(raw));
        let old: Response =
            serde_json::from_str(r#"{"ok":false,"error":"load failed","model_loaded":false}"#)
                .unwrap();
        assert!(old.raw_completion.is_none());
        assert!(old.model_was_loaded.is_none());
        let request: Request =
            serde_json::from_str(r#"{"type":"generate","prompt":"list files","context":null}"#)
                .unwrap();
        assert!(matches!(
            request,
            Request::Generate {
                include_raw: false,
                force_unload: false,
                ..
            }
        ));
    }

    #[test]
    fn request_round_trip() {
        let encoded = serde_json::to_string(&Request::Generate {
            prompt: "list files".into(),
            include_raw: false,
            force_unload: true,
            context: Some("ls".into()),
            system_prompt: None,
            workflow_id: Some("test-workflow".into()),
            stage: Some("search".into()),
            assistant_prefix: None,
            stop_after: None,
            inference_config: Some(Box::new(
                crate::config::Config::default().inference_config(),
            )),
        })
        .unwrap();
        let decoded: Request = serde_json::from_str(&encoded).unwrap();
        assert!(matches!(
            decoded,
            Request::Generate {
                force_unload: true,
                ..
            }
        ));
    }
}
