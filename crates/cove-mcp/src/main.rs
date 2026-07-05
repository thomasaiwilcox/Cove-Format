use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Parser, ValueEnum};
use cove_mcp::{
    serve_http, serve_stdio, AllowedRoot, AuthConfig, CoveMcpConfig, CoveMcpError, HttpConfig,
    TransportMode,
};

#[derive(Debug, Parser)]
#[command(
    name = "cove-mcp",
    about = "Serve COVE query discovery and CoveQL over MCP"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    Serve(ServeArgs),
}

#[derive(Debug, Parser)]
struct ServeArgs {
    #[arg(long, value_enum, default_value_t = TransportArg::Stdio)]
    transport: TransportArg,
    #[arg(long = "root", value_name = "ID=PATH")]
    roots: Vec<String>,
    #[arg(long, default_value = "127.0.0.1:8765")]
    bind: SocketAddr,
    #[arg(long, default_value = "COVE_MCP_TOKEN")]
    bearer_token_env: String,
    #[arg(long)]
    allow_no_auth_local: bool,
    #[arg(long)]
    allowed_origin: Vec<String>,
    #[arg(long, default_value_t = 50)]
    default_take: usize,
    #[arg(long, default_value_t = 500)]
    max_take: usize,
    #[arg(long, default_value_t = 100)]
    page_size: usize,
    #[arg(long, default_value_t = 1_048_576)]
    max_response_bytes: usize,
    #[arg(long, default_value_t = 600)]
    result_ttl_seconds: u64,
    #[arg(long, default_value_t = 128)]
    max_result_handles: usize,
    #[arg(long)]
    developer_mode: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TransportArg {
    Stdio,
    Http,
}

impl From<TransportArg> for TransportMode {
    fn from(value: TransportArg) -> Self {
        match value {
            TransportArg::Stdio => Self::Stdio,
            TransportArg::Http => Self::Http,
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cove_mcp=info,warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("cove-mcp: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), CoveMcpError> {
    match cli.command {
        Command::Serve(args) => serve(args).await,
    }
}

async fn serve(args: ServeArgs) -> Result<(), CoveMcpError> {
    if args.default_take == 0 || args.max_take == 0 || args.page_size == 0 {
        return Err(CoveMcpError::Config(
            "default_take, max_take, and page_size must be positive".into(),
        ));
    }
    if args.default_take > args.max_take {
        return Err(CoveMcpError::Config(
            "default_take must not exceed max_take".into(),
        ));
    }
    let mut config = CoveMcpConfig::new(parse_roots(&args.roots)?)?;
    config.default_take = args.default_take;
    config.max_take = args.max_take;
    config.page_size = args.page_size;
    config.max_response_bytes = args.max_response_bytes;
    config.result_ttl = std::time::Duration::from_secs(args.result_ttl_seconds);
    config.max_result_handles = args.max_result_handles;
    config.developer_mode = args.developer_mode;
    let config = Arc::new(config);
    match TransportMode::from(args.transport) {
        TransportMode::Stdio => serve_stdio(config).await,
        TransportMode::Http => {
            let auth = http_auth(&args)?;
            serve_http(
                config,
                HttpConfig {
                    bind: args.bind,
                    auth,
                    allowed_origins: args.allowed_origin,
                },
            )
            .await
        }
    }
}

fn parse_roots(values: &[String]) -> Result<Vec<AllowedRoot>, CoveMcpError> {
    values
        .iter()
        .map(|value| {
            let Some((id, path)) = value.split_once('=') else {
                return Err(CoveMcpError::Config(format!(
                    "root `{value}` must use ID=PATH syntax"
                )));
            };
            AllowedRoot::new(id, PathBuf::from(path))
        })
        .collect()
}

fn http_auth(args: &ServeArgs) -> Result<AuthConfig, CoveMcpError> {
    let token = std::env::var(&args.bearer_token_env).ok();
    if token.is_none() && !args.allow_no_auth_local {
        return Err(CoveMcpError::Config(format!(
            "HTTP transport requires bearer token env {} or --allow-no-auth-local",
            args.bearer_token_env
        )));
    }
    Ok(AuthConfig {
        bearer_token: token,
        allow_no_auth_local: args.allow_no_auth_local,
    })
}
