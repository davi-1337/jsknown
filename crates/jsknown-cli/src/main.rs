use anyhow::Result;
use clap::{Parser, Subcommand};
use jsknown_core::config::Config;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "jsknown")]
#[command(
    about = "JavaScript asset recovery, chunk fetching, source map reversal, and AST analysis"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve(ServeArgs),
}

#[derive(Parser, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 3333)]
    port: u16,
    #[arg(long, default_value = "default")]
    project: String,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long = "scope")]
    scopes: Vec<String>,
    #[arg(long, default_value_t = 2)]
    rate_per_second: u32,
    #[arg(long, default_value_t = 0)]
    rate_per_minute: u32,
    #[arg(long, default_value_t = 5)]
    fetch_concurrency: usize,
    #[arg(long, default_value_t = 25_000_000)]
    max_body_bytes: usize,
    #[arg(long)]
    debug: bool,
}

impl From<ServeArgs> for Config {
    fn from(args: ServeArgs) -> Self {
        Config {
            host: args.host,
            port: args.port,
            project: args.project,
            output_root: args.output,
            scope_patterns: args.scopes,
            rate_per_second: args.rate_per_second,
            rate_per_minute: args.rate_per_minute,
            fetch_concurrency: args.fetch_concurrency,
            max_body_bytes: args.max_body_bytes,
            debug: args.debug,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve(args) => {
            let config: Config = args.into();
            let default_filter = if config.debug { "jsknown=debug,info" } else { "info" };
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
                )
                .init();
            jsknown_server::serve(config).await
        }
    }
}
