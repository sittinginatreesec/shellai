use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "shellai",
    version,
    about = "Local AI shell command generation",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Override GPU offload layers (0 forces CPU; default requests all available layers).
    #[arg(long, global = true)]
    pub gpu_layers: Option<u32>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate a command, starting the local server if necessary.
    Ask {
        /// Existing command line to take into account.
        #[arg(long)]
        context: Option<String>,

        /// Fail instead of cold-starting the server.
        #[arg(long)]
        no_start: bool,

        /// Natural-language request.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },

    /// Run the local inference server in the foreground.
    Server {
        /// Override the model's idle TTL in seconds (0 unloads after every request).
        #[arg(long)]
        model_ttl: Option<u64>,

        /// Internal marker used by the cold-start client.
        #[arg(long, hide = true)]
        background_child: bool,
    },

    /// List the compute devices available to this build without loading a model.
    Devices,

    /// Report whether the server and model are resident.
    Status,

    /// Ask the local server to exit.
    Stop,

    /// Download the configured GGUF into the Hugging Face cache.
    Download,

    /// Print the config path or create a documented default config.
    Config {
        #[arg(long)]
        init: bool,
    },

    /// Print the bundled zsh plugin to stdout.
    Plugin,

    /// Print the JSONL metric log locations.
    Metrics,
}
