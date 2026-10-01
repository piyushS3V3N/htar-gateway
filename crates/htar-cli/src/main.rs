use clap::{Parser, Subcommand};
use htar_core::{CompressionType, HtarReader, HtarWriter};
use htar_proxy::GatewayConfig;
use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

pub mod migrate;
use migrate::MigrateEngine;

#[derive(Parser)]
#[command(name = "htar-gw")]
#[command(about = "HTAR API Gateway — Ultra-Fast Hybrid TAR Hashtable Gateway", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the API Gateway server
    Run {
        /// Path to the gateway configuration file (default: config/gateway.toml)
        #[arg(short, long, default_value = "config/gateway.toml")]
        config: PathBuf,
    },
    /// Pack a directory of assets or response snapshots into a compressed HTAR archive
    Pack {
        /// Source directory to pack
        #[arg(short, long)]
        input_dir: PathBuf,
        /// Output .htar archive path
        #[arg(short, long)]
        output: PathBuf,
        /// Prefix to prepend to entry keys (e.g. "/api/v1/static/")
        #[arg(long, default_value = "/")]
        prefix: String,
        /// Zstd compression level (1 to 22)
        #[arg(short, long, default_value_t = 3)]
        level: i32,
    },
    /// Inspect an HTAR archive and display hashtable statistics
    Inspect {
        /// Path to .htar archive
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Run performance benchmarks of HTAR O(1) lookup vs standard scanning
    Benchmark {
        /// Number of entries for test archive
        #[arg(short, long, default_value_t = 10000)]
        count: usize,
    },
    /// View all active registered endpoints and routes in HTAR Gateway
    Routes {
        /// Base URL of HTAR Gateway server (e.g. http://127.0.0.1:8443 or http://172.21.0.3:8088)
        #[arg(short, long, default_value = "http://127.0.0.1:8443")]
        server: String,
    },
    /// Migrate legacy Layer7 XML or Kong JSON bundles to HTAR Gateway format, Wasm guests & Gateway API CRDs
    Migrate {
        /// Path to input Layer7 XML policy file or Kong JSON bundle
        #[arg(short, long)]
        input: PathBuf,
        /// Directory to output generated Wasm guest bytecode modules
        #[arg(short, long, default_value = "dist/wasm")]
        wasm_dir: PathBuf,
        /// Path to output CNCF Gateway API CRD YAML
        #[arg(short, long, default_value = "dist/gateway-api-routes.yaml")]
        crd_output: PathBuf,
        /// Path to output GraphQL-over-REST SDL schema
        #[arg(short, long, default_value = "dist/schema.graphql")]
        graphql_output: PathBuf,
        /// Path to output MySQL 8.x OTK schema migration SQL
        #[arg(short, long, default_value = "dist/otk_schema.sql")]
        sql_output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { config } => {
            info!("Loading configuration from {:?}", config);
            let gateway_config = if config.exists() {
                GatewayConfig::load_from_file(&config)?
            } else {
                info!("Config file {:?} not found, using default configuration", config);
                GatewayConfig::default()
            };

            let server = htar_proxy::GatewayServer::new(gateway_config);
            server.run().await?;
        }
        Commands::Pack {
            input_dir,
            output,
            prefix,
            level,
        } => {
            info!("Packing directory {:?} into HTAR bundle {:?}", input_dir, output);
            let start = Instant::now();
            let file = File::create(&output)?;
            let mut writer = HtarWriter::new(file).with_compression_level(level);

            let mut count = 0;
            let mut raw_bytes: u64 = 0;
            pack_dir_recursive(&input_dir, &input_dir, &prefix, &mut writer, &mut count, &mut raw_bytes)?;

            writer.finish()?;
            let elapsed = start.elapsed();
            let archived_size = fs::metadata(&output)?.len();

            let ratio = if raw_bytes > 0 {
                (1.0 - (archived_size as f64 / raw_bytes as f64)) * 100.0
            } else {
                0.0
            };

            println!("Packed {} items into {:?}", count, output);
            println!("   Raw payload size : {:.2} KB", raw_bytes as f64 / 1024.0);
            println!("   HTAR archive size: {:.2} KB", archived_size as f64 / 1024.0);
            println!("   Compression ratio: {:.1}% space saved", ratio);
            println!("   Time taken       : {:?}", elapsed);
        }
        Commands::Inspect { file } => {
            println!("Inspecting HTAR Archive: {:?}", file);
            let f = File::open(&file)?;
            let reader = HtarReader::open(f)?;

            println!("--------------------------------------------------");
            println!(" Total Indexed Entries: {}", reader.len());
            println!("--------------------------------------------------");
            println!("{:<45} | {:<12} | {:<12} | {:<8}", "Cache Key", "Comp Size", "Raw Size", "Format");
            println!("--------------------------------------------------");

            for (key, entry) in reader.index().iter().take(25) {
                println!(
                    "{:<45} | {:<12} | {:<12} | {:?}",
                    truncate(key, 45),
                    entry.compressed_size,
                    entry.uncompressed_size,
                    entry.compression
                );
            }

            if reader.len() > 25 {
                println!("... and {} more entries", reader.len() - 25);
            }
        }
        Commands::Benchmark { count } => {
            println!("Running HTAR O(1) Hashtable Random Access Benchmark with {} entries...", count);
            run_benchmark(count)?;
        }
        Commands::Routes { server } => {
            let endpoint_url = format!("{}/_htar/endpoints", server.trim_end_matches('/'));
            println!("Querying HTAR Gateway Endpoints from: {}", endpoint_url);

            let res = reqwest::get(&endpoint_url).await?;
            if !res.status().is_success() {
                println!("Failed to query endpoints: HTTP {}", res.status());
                return Ok(());
            }

            let json_val: serde_json::Value = res.json().await?;
            let empty_vec = vec![];
            let endpoints = json_val.get("endpoints").and_then(|v| v.as_array()).unwrap_or(&empty_vec);

            println!("===============================================================================================================================");
            println!(" Registered HTAR Gateway Endpoints (Total: {})", endpoints.len());
            println!("===============================================================================================================================");
            println!("{:<32} | {:<20} | {:<25} | {:<7} | {:<5} | {:<28}", "Host Domain", "Path Prefix", "Service Name", "Healthy", "Cache", "Upstream Target");
            println!("-------------------------------------------------------------------------------------------------------------------------------");

            for ep in endpoints {
                let hosts = ep.get("hosts").and_then(|h| h.as_array()).map(|arr| {
                    arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
                }).unwrap_or_else(|| "*".to_string());

                let paths = ep.get("paths").and_then(|p| p.as_array()).map(|arr| {
                    arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
                }).unwrap_or_else(|| "-".to_string());

                let service_name = ep.get("service_name").and_then(|v| v.as_str()).unwrap_or("-");
                let healthy = if ep.get("healthy").and_then(|v| v.as_bool()).unwrap_or(false) { "YES" } else { "NO" };
                let cache = if ep.get("cache_enabled").and_then(|v| v.as_bool()).unwrap_or(false) { "YES" } else { "NO" };

                let upstreams = ep.get("upstreams").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter().filter_map(|u| u.get("url").and_then(|url| url.as_str())).collect::<Vec<_>>().join(", ")
                }).unwrap_or_else(|| "-".to_string());

                println!(
                    "{:<32} | {:<20} | {:<25} | {:<7} | {:<5} | {:<28}",
                    truncate(&hosts, 32),
                    truncate(&paths, 20),
                    truncate(service_name, 25),
                    healthy,
                    cache,
                    truncate(&upstreams, 28)
                );
            }
            println!("===============================================================================================================================");
        }
        Commands::Migrate {
            input,
            wasm_dir,
            crd_output,
            graphql_output,
            sql_output,
        } => {
            println!("=================================================================================");
            println!(" HTAR Migration Engine — Layer 7 to Kubernetes-Native GraphQL & OTK Architecture");
            println!("=================================================================================");
            println!(" Ingesting bundle from input file : {:?}", input);

            let mut bundle = MigrateEngine::parse_input_bundle(&input)?;
            println!(" Successfully parsed legacy config bundle:");
            println!("   - Services extracted : {}", bundle.services.len());
            println!("   - Routes extracted   : {}", bundle.routes.len());
            println!("   - Consumers extracted: {}", bundle.consumers.len());
            println!("   - Plugins extracted  : {}", bundle.plugins.len());

            println!("\n Task: Generating eBPF XDP Driver Map C rules & Wasm targets...");
            MigrateEngine::generate_wasm_target(&mut bundle, &wasm_dir)?;
            MigrateEngine::generate_ebpf_xdp_maps(&bundle, &PathBuf::from("dist/ebpf"))?;

            println!("\n Task: Generating CNCF Gateway API CRDs with custom annotations...");
            MigrateEngine::generate_gateway_api_crds(&bundle, &crd_output)?;

            println!("\n Task: Generating GraphQL-over-REST Schema (SDL) with query depth guards...");
            MigrateEngine::generate_graphql_sdl(&bundle, &graphql_output)?;

            println!("\n Task: Generating MySQL 8.x OTK Schema Persistence Statements...");
            MigrateEngine::generate_mysql_otk_migration(&bundle, &sql_output)?;

            println!("---------------------------------------------------------------------------------");
            println!(" Migration complete!");
            println!("   - Wasm output directory : {:?}", wasm_dir);
            println!("   - eBPF map directory    : dist/ebpf");
            println!("   - Gateway API CRD output: {:?}", crd_output);
            println!("   - GraphQL SDL output    : {:?}", graphql_output);
            println!("   - MySQL OTK SQL output  : {:?}", sql_output);
            println!("=================================================================================");
        }
    }

    Ok(())
}

fn pack_dir_recursive<P: AsRef<Path>>(
    root: &Path,
    dir: P,
    prefix: &str,
    writer: &mut HtarWriter<File>,
    count: &mut usize,
    raw_bytes: &mut u64,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            pack_dir_recursive(root, &path, prefix, writer, count, raw_bytes)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            let key = format!("GET:{}{}", prefix, rel).replace("//", "/");
            let data = fs::read(&path)?;

            *raw_bytes += data.len() as u64;
            *count += 1;

            let ct = match path.extension().and_then(|e| e.to_str()) {
                Some("json") => "application/json",
                Some("html") => "text/html",
                Some("js") => "application/javascript",
                Some("css") => "text/css",
                Some("png") => "image/png",
                _ => "application/octet-stream",
            };

            writer.add_entry(key, &data, ct, CompressionType::Zstd)?;
        }
    }
    Ok(())
}

fn run_benchmark(count: usize) -> anyhow::Result<()> {
    let mut buffer = Vec::new();
    let mut writer = HtarWriter::new(&mut buffer);

    for i in 0..count {
        let key = format!("GET:/api/v1/resource/item_{}", i);
        let payload = format!("{{\"id\": {}, \"name\": \"item_{}\", \"payload\": \"sample_data_content\"}}", i, i);
        writer.add_entry(key, payload.as_bytes(), "application/json", CompressionType::Zstd)?;
    }
    writer.finish()?;

    let total_size = buffer.len();
    println!("Created benchmark HTAR archive with {} entries ({:.2} KB).", count, total_size as f64 / 1024.0);

    let mut reader = HtarReader::open(Cursor::new(&buffer))?;

    // Benchmark O(1) lookups
    let target_key = format!("GET:/api/v1/resource/item_{}", count - 1);
    let iterations = 100_000;

    let start = Instant::now();
    for _ in 0..iterations {
        let _ = reader.read_payload(&target_key)?;
    }
    let elapsed = start.elapsed();

    let ns_per_op = elapsed.as_nanos() as f64 / iterations as f64;
    let ops_per_sec = (iterations as f64 / elapsed.as_secs_f64()) as u64;

    println!("--------------------------------------------------");
    println!(" RESULTS: HTAR Hashtable O(1) Payload Retrieval");
    println!("--------------------------------------------------");
    println!(" Total random reads : {}", iterations);
    println!(" Total elapsed time : {:?}", elapsed);
    println!(" Average latency    : {:.2} ns / lookup", ns_per_op);
    println!(" Throughput         : {} ops / second", ops_per_sec);
    println!("--------------------------------------------------");

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}
