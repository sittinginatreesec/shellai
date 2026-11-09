# Shellai

Shellai turns a natural-language request into an editable shell command on your local machine. It uses Bonsai with a single zero-shot inference and a strict JSON command response.

## Install

Install a release package for your platform, or build from source with Rust and the native build dependencies required by llama.cpp:

```sh
cargo build --release
```

Put `shellai` on your PATH. For Zsh integration, add this to `.zshrc`:

```zsh
source /path/to/shellai.plugin.zsh
```

Model weights download automatically on first use into the Hugging Face cache. To download ahead of time:

```sh
shellai download
```

The default model is hosted at [micrictor/bonsai-shellai](https://huggingface.co/micrictor/bonsai-shellai). Downloads of the default artifact are checked for size and SHA256 integrity.

## Usage

```sh
shellai ask -- 'find files larger than 100 MB in this directory'
shellai ask --context 'ls' -- 'include hidden files'
```

In interactive Zsh, type `ai, <request>` and press Enter, or press Alt-A to enter a request. The generated command replaces the editable buffer. Review it before pressing Enter to execute it.

The client starts a local server automatically. The model unloads after 60 idle seconds while the server stays available. Communication uses a per-user Unix socket or a Windows named pipe.

```sh
shellai status
shellai stop
shellai devices
shellai metrics
```

Use `shellai ask --no-start` to require an already-running server. Use `shellai --gpu-layers 0 ask -- 'your request'` to force CPU inference. GPU availability depends on build features and installed drivers.

## Configuration

```sh
shellai config --init
shellai config
```

These commands create a default configuration if missing and print its location. Settings include model location, idle timeout, context and output budgets, CPU threads, and GPU layers.

`SHELLAI_MODEL=/path/to/model.gguf` selects an alternative local model. It must support the same Gemma4 tokenizer and JSON command protocol. A local `model_path` can also be set in the config; otherwise `repository` and `model_file` select a Hugging Face artifact. `SHELLAI_MODEL_TTL` overrides the idle timeout in seconds. `HF_TOKEN` can provide Hugging Face authentication.

The default context budget is 2048 tokens, with up to 64 generated tokens and greedy decoding. Requests exceeding the context budget return an error.

The Zsh plugin stores request history under `${XDG_STATE_HOME:-$HOME/.local/state}/shellai/history`. Customize it with `SHELLAI_HISTORY_FILE`, `SHELLAI_HISTORY_SIZE`, or `SHELLAI_HOTKEY`.

## Development

```sh
cargo test
cargo build
python3 tests/cli_smoke.py target/debug/shellai
```

The tokenizer integration test requires a compatible local GGUF and is ignored by default. Generated commands are suggestions: review them before execution, especially operations that modify files.

Shellai is MIT licensed. Model weights and third-party dependencies retain their respective licenses.
