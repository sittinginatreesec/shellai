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

Keep `shellai-capture.py` beside the plugin and install Python 3 for the
experimental previous-command capture feature.

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

At the Alt-A prompt, press **Alt-F** to suggest a fix for the last command without
typing a request. Alt-F retains its normal binding outside that prompt. Capture
starts with commands executed after the plugin is loaded; earlier output cannot
be recovered.
If the model repeats the failed command unchanged, the plugin reports that no
fix was found and leaves the editable buffer alone. Fix quality depends on the
model; describing the intended result can work better than asking it to diagnose
an error.

The plugin includes the previous command, working directory, exit status, and
limited output tails with interactive requests. Each shell keeps its own record,
replaced by the next command. It retains at most 4 KiB per stream in relay memory
and sends at most 512 bytes of stderr and 256 bytes of stdout, with terminal
escapes removed. Command and directory context are also capped. Shellai calls
do not replace the record, and commands are never rerun to obtain output.
Long user requests can still exceed the model's context limit.

**Capture is experimental:** stdout and stderr pass through pipes, so programs
can change buffering, colors, paging, or interactive behavior. Set
`SHELLAI_CAPTURE=0` to disable capture (and restore normal terminal output for
subsequent commands); set it to `1` to enable it again. Output context can contain
sensitive command output and is passed to the local model. Raw output is held
in memory, not written to a transcript. Private temporary FIFOs and a socket are
removed after each command. Background output after the foreground command
finishes remains visible but is excluded from its completed context record.

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
python3 tests/zsh_context.py
```

The tokenizer integration test requires a compatible local GGUF and is ignored by default. Generated commands are suggestions: review them before execution, especially operations that modify files.

Shellai is MIT licensed. Model weights and third-party dependencies retain their respective licenses.
