//! DUCP reference node (Rust) — CLI entry point.
//!
//! Runs a node on the DUCP network. Here, "node" means a *network participant* —
//! the software you run to take part in DUCP (as a Provider, Validator, and so on).
//! It is unrelated to Node.js and contains no JavaScript.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Profile 0 reference node for spec v0.2.0.

use std::net::SocketAddr;

use clap::Parser;
use ducp_node::{start_server, NodeHandle};
use ducp_types::{keys, Identity, Ucu, UCU_SCALE};

/// Run a DUCP Profile 0 node (single-sequencer devnet).
#[derive(Parser, Debug)]
#[command(name = "ducp-node", version, about)]
struct Cli {
    /// Address to bind the JSON-RPC server to.
    #[arg(long, default_value = "127.0.0.1:8645")]
    listen: SocketAddr,

    /// 32-byte hex seed for the sequencer's Ed25519 key.
    #[arg(
        long,
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    seed: String,

    /// Genesis allocation, repeatable: `--alloc <identity_hex>:<amount_ucu>`.
    /// If none are given, three dev keys (seeds 01.., 02.., 03..) are funded.
    #[arg(long = "alloc", value_name = "IDENTITY_HEX:AMOUNT")]
    alloc: Vec<String>,
}

fn parse_seed(hex_str: &str) -> anyhow::Result<[u8; 32]> {
    let v = hex::decode(hex_str.strip_prefix("0x").unwrap_or(hex_str))?;
    let arr: [u8; 32] = v
        .try_into()
        .map_err(|_| anyhow::anyhow!("seed must be 32 bytes"))?;
    Ok(arr)
}

fn parse_alloc(s: &str) -> anyhow::Result<(Identity, Ucu)> {
    let (id_hex, amount) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("alloc must be <identity_hex>:<amount>"))?;
    let id_bytes = hex::decode(id_hex.strip_prefix("0x").unwrap_or(id_hex))?;
    let id: Identity = id_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("identity must be 32 bytes"))?;
    let amount: Ucu = amount.parse()?;
    Ok((id, amount))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let seed = parse_seed(&cli.seed)?;
    let proposer = keys::identity(&seed);

    let allocations: Vec<(Identity, Ucu)> = if cli.alloc.is_empty() {
        // Default dev allocations: fund three well-known dev keys with 1,000,000 𝕌.
        let fund = 1_000_000 * UCU_SCALE;
        vec![
            (keys::identity(&[1u8; 32]), fund),
            (keys::identity(&[2u8; 32]), fund),
            (keys::identity(&[3u8; 32]), fund),
        ]
    } else {
        cli.alloc
            .iter()
            .map(|s| parse_alloc(s))
            .collect::<anyhow::Result<_>>()?
    };

    let handle = NodeHandle::new(proposer, &allocations);

    println!(
        "DUCP node (Rust reference implementation) v{}",
        env!("CARGO_PKG_VERSION")
    );
    println!("  implements DUCP specification v0.2.0 (Profile 0)");
    println!("  sequencer  {}", hex::encode(proposer));
    println!(
        "  benchmark  v{} (fuel_per_ucu={})",
        handle.benchmark.version, handle.benchmark.fuel_per_ucu
    );
    println!("  genesis    {} account(s)", allocations.len());

    let (bound, server_handle) = start_server(handle, cli.listen).await?;
    println!("  JSON-RPC   http://{bound}");
    tracing::info!(%bound, "DUCP node listening");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    server_handle.stop()?;
    Ok(())
}
