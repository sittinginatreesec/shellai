//! Native Gemma4 command prompt and JSON response parsing.
use anyhow::{Context, Result};
use serde::Deserialize;

pub const SYSTEM: &str = concat!(
    "You translate a user request into one complete Linux shell command. ",
    "The runtime is GNU/Linux with GNU coreutils, grep, sed, awk and Bash. ",
    "Paths are relative to the current working directory unless stated otherwise. ",
    "Return exactly one JSON object with one string field: {\"command\":\"...\"}. ",
    "No Markdown or explanation. The command must satisfy the request as written. ",
    "Do not execute it. Do not add unrelated filesystem changes."
);

pub fn render_prompt(request: &str, context: Option<&str>) -> String {
    let mut user = request.to_owned();
    if let Some(context) = context.filter(|value| !value.is_empty()) {
        user.push_str("\nContext: ");
        user.push_str(context);
    }
    format!(
        "<bos><|turn>system\n{}<turn|>\n<|turn>user\n{}<turn|>\n<|turn>model\n",
        SYSTEM.trim(),
        user.trim()
    )
}

/// Completion terminator in the exact pinned Gemma4 tokenizer.
pub const END_TURN_TOKEN: i32 = 106;

pub fn check_budget(prompt_tokens: usize, max_new_tokens: u32, context_size: u32) -> Result<()> {
    anyhow::ensure!(max_new_tokens > 0, "max_new_tokens must be nonzero");
    anyhow::ensure!(
        prompt_tokens.saturating_add(max_new_tokens as usize) <= context_size as usize,
        "request plus generation budget exceeds the configured {context_size} token context"
    );
    Ok(())
}

/// Normalize a single JSON fence before parsing.
/// Only one complete lowercase json/untagged fence is unwrapped; schema remains strict.
fn normalize_command_output(output: &str) -> String {
    // Python str.strip/splitlines also recognize these ASCII record separators.
    let whitespace = |c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c);
    let trimmed = output.trim_matches(whitespace);
    let canonical = trimmed.replace("\r\n", "\n");
    let lines: Vec<&str> = canonical
        .split(|c| {
            matches!(
                c,
                '\n' | '\r'
                    | '\u{b}'
                    | '\u{c}'
                    | '\u{1c}'
                    | '\u{1d}'
                    | '\u{1e}'
                    | '\u{85}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
        })
        .collect();
    if lines.len() >= 3
        && matches!(lines[0].trim_matches(whitespace), "```json" | "```")
        && lines.last().unwrap().trim_matches(whitespace) == "```"
    {
        return lines[1..lines.len() - 1]
            .join("\n")
            .trim_matches(whitespace)
            .to_owned();
    }
    trimmed.to_owned()
}

pub fn parse_command(output: &str) -> Result<String> {
    let normalized = normalize_command_output(output);
    let output = normalized.as_str();
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Command {
        command: String,
    }
    // A typed struct rejects duplicate fields as well as unknown fields. The
    // JSON parser rejects remaining Markdown, trailing text, and non-string commands.
    // serde's struct deserializer also accepts positional arrays; the response
    // contract requires an object specifically.
    anyhow::ensure!(
        output.trim_start().starts_with('{'),
        "expected a JSON command object"
    );
    let result: Command = serde_json::from_str(output)
        .context("expected exactly one JSON object with one string field: {\"command\":\"...\"}")?;
    anyhow::ensure!(
        !result.command.trim().is_empty(),
        "the model returned an empty command"
    );
    Ok(result.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pinned_huggingface_template_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/gemma4-prompt.json")).unwrap();
        assert_eq!(SYSTEM, fixture["messages"][0]["content"].as_str().unwrap());
        assert_eq!(
            render_prompt(fixture["request"].as_str().unwrap(), None),
            fixture["rendered"].as_str().unwrap()
        );
    }

    #[test]
    fn reserves_entire_generation_budget() {
        assert!(check_budget(1792, 256, 2048).is_ok());
        assert!(check_budget(1793, 256, 2048).is_err());
        assert!(check_budget(128, 0, 2048).is_err());
        assert_eq!(END_TURN_TOKEN, 106);
    }

    #[test]
    fn renders_native_gemma4_nonthinking_protocol() {
        let prompt = render_prompt("  list files  ", None);
        assert_eq!(
            prompt,
            format!(
                "<bos><|turn>system\n{SYSTEM}<turn|>\n<|turn>user\nlist files<turn|>\n<|turn>model\n"
            )
        );
        assert!(!prompt.contains("<start_of_turn>"));
        assert!(!prompt.contains("<|think|>"));
        assert!(
            render_prompt("change it", Some("git log"))
                .contains("change it\nContext: git log<turn|>")
        );
    }

    #[test]
    fn preserves_unicode_quotes_and_command_whitespace() {
        let command = "  printf '%s\\n' \"‘café’\"  ";
        let json = serde_json::json!({"command": command}).to_string();
        assert_eq!(parse_command(&json).unwrap(), command);
    }

    #[test]
    fn normalizes_single_json_fence() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/single-json-fence-v1.json"))
                .unwrap();
        for case in fixtures.as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(
                normalize_command_output(input),
                case["normalized"].as_str().unwrap(),
                "{input:?}"
            );
            let result = parse_command(input);
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{input:?}"
            );
            if let Some(command) = case["command"].as_str() {
                assert_eq!(result.unwrap(), command);
            }
        }
    }

    #[test]
    fn rejects_invalid_command_protocol() {
        for invalid in [
            r#"{"command":"ls","command":"pwd"}"#,
            r#"{"command":"ls","extra":true}"#,
            r#"{"command":null}"#,
            r#"{"command":0}"#,
            r#"{"command":"  "}"#,
            r#"["ls"]"#,
            "ls",
            r#"{"command":"ls"} trailing"#,
        ] {
            assert!(parse_command(invalid).is_err(), "accepted {invalid}");
        }
    }
}
