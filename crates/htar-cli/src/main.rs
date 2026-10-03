use clap::{Parser, Subcommand};
use htar_core::{CompressionType, HtarReader, HtarWriter};
use htar_proxy::GatewayConfig;
use std::fs::{self, File};
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
    /// Migrate legacy Layer7 XML or Kong JSON bundles to HTAR Gateway configuration & CNCF Gateway API CRDs
    Migrate {
        /// Path to input Layer7 XML policy file or Kong JSON bundle
        #[arg(short, long)]
        input: PathBuf,
        /// Path to output HTAR Gateway TOML configuration (e.g. dist/gateway.toml)
        #[arg(short, long, default_value = "dist/gateway.toml")]
        config_output: PathBuf,
        /// Path to output CNCF Gateway API CRD YAML
        #[arg(long, default_value = "dist/gateway-api-routes.yaml")]
        crd_output: PathBuf,
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
            config_output,
            crd_output,
        } => {
            println!("=================================================================================");
            println!(" HTAR Migration Engine — Layer 7 & Kong to Native HTAR Gateway Configuration");
            println!("=================================================================================");
            println!(" Ingesting bundle from input file : {:?}", input);

            let bundle = MigrateEngine::parse_input_bundle(&input)?;
            println!(" Successfully parsed legacy config bundle:");
            println!("   - Services extracted : {}", bundle.services.len());
            println!("   - Routes extracted   : {}", bundle.routes.len());
            println!("   - Consumers extracted: {}", bundle.consumers.len());
            println!("   - Plugins extracted  : {}", bundle.plugins.len());

            println!("\n Task: Generating runnable HTAR Gateway TOML configuration...");
            MigrateEngine::generate_gateway_config(&bundle, &config_output)?;

            println!("\n Task: Generating CNCF Gateway API CRDs with Kubernetes annotations...");
            MigrateEngine::generate_gateway_api_crds(&bundle, &crd_output)?;

            println!("---------------------------------------------------------------------------------");
            println!(" Migration complete!");
            println!("   - Gateway Config output : {:?}", config_output);
            println!("   - Gateway API CRD output: {:?}", crd_output);
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
    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    println!("Generating benchmark HTAR archive on disk ({} entries)...", count);
    {
        let file = File::create(&temp_path)?;
        let mut writer = HtarWriter::new(file).with_compression_level(3);

        for i in 0..count {
            let key = format!("GET:/api/v1/resource/item_{}", i);
            let payload = format!(
                "{{\"id\": {}, \"name\": \"item_{}\", \"payload\": \"sample_data_content_{}\"}}",
                i, i, i
            );
            writer.add_entry(key, payload.as_bytes(), "application/json", CompressionType::Zstd)?;
        }
        writer.finish()?;
    }

    let file_size = fs::metadata(&temp_path)?.len();
    println!("Archived file size on disk: {:.2} KB", file_size as f64 / 1024.0);

    let mut reader = HtarReader::open(File::open(&temp_path)?)?;
    let iterations = 50_000;

    // Generate genuine pseudo-random keys across the entire index space
    println!("Pre-generating {} uniform random query keys across all {} entries...", iterations, count);
    let mut rng: u64 = 0x853c49e6748fea9b;
    let query_keys: Vec<String> = (0..iterations)
        .map(|_| {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            let idx = (rng as usize) % count;
            format!("GET:/api/v1/resource/item_{}", idx)
        })
        .collect();

    // -------------------------------------------------------------
    // BENCHMARK 1: O(1) In-Memory Hashtable Index Resolution
    // -------------------------------------------------------------
    println!("Running Benchmark 1: In-Memory O(1) Hashtable Index Lookup...");
    let mut index_latencies_ns = Vec::with_capacity(iterations);
    let start_index = Instant::now();
    for key in &query_keys {
        let t0 = Instant::now();
        let entry = reader.get_entry(key);
        index_latencies_ns.push(t0.elapsed().as_nanos() as u64);
        assert!(entry.is_some());
    }
    let total_index_time = start_index.elapsed();
    index_latencies_ns.sort_unstable();

    let p50_index = index_latencies_ns[iterations * 50 / 100];
    let p95_index = index_latencies_ns[iterations * 95 / 100];
    let p99_index = index_latencies_ns[iterations * 99 / 100];
    let avg_index_ns = total_index_time.as_nanos() as f64 / iterations as f64;
    let index_throughput = (iterations as f64 / total_index_time.as_secs_f64()) as u64;

    // -------------------------------------------------------------
    // BENCHMARK 2: Full Random Read (File Seek + Read + Zstd Decompression)
    // -------------------------------------------------------------
    println!("Running Benchmark 2: End-to-End File Seek, Read & Zstd Decompression...");
    let mut payload_latencies_ns = Vec::with_capacity(iterations);
    let start_payload = Instant::now();
    for key in &query_keys {
        let t0 = Instant::now();
        let payload = reader.read_payload(key)?;
        payload_latencies_ns.push(t0.elapsed().as_nanos() as u64);
        assert!(!payload.is_empty());
    }
    let total_payload_time = start_payload.elapsed();
    payload_latencies_ns.sort_unstable();

    let p50_payload = payload_latencies_ns[iterations * 50 / 100];
    let p95_payload = payload_latencies_ns[iterations * 95 / 100];
    let p99_payload = payload_latencies_ns[iterations * 99 / 100];
    let avg_payload_ns = total_payload_time.as_nanos() as f64 / iterations as f64;
    let payload_throughput = (iterations as f64 / total_payload_time.as_secs_f64()) as u64;

    println!("\n================================================================================");
    println!(" REAL HTAR PERFORMANCE BENCHMARK RESULTS (N = {} indexed items)", count);
    println!("================================================================================");
    println!(" Benchmark Environment  : Real Disk File ({:.2} KB on disk)", file_size as f64 / 1024.0);
    println!(" Access Distribution   : Uniform Pseudo-Random across all keys (zero key pinning)");
    println!(" Total Queries Run      : {} lookups per test", iterations);
    println!("--------------------------------------------------------------------------------");
    println!(" TEST 1: In-Memory O(1) Hashtable Index Resolution (Zero-I/O Metadata Lookup)");
    println!("--------------------------------------------------------------------------------");
    println!("   Throughput           : {:>10} lookups / sec", index_throughput);
    println!("   Average Latency      : {:>10.2} ns", avg_index_ns);
    println!("   P50 Latency (median) : {:>10} ns", p50_index);
    println!("   P95 Latency          : {:>10} ns", p95_index);
    println!("   P99 Latency          : {:>10} ns", p99_index);
    println!("--------------------------------------------------------------------------------");
    println!(" TEST 2: End-to-End File Retrieval (Disk Seek + Read + Zstandard Decompression)");
    println!("--------------------------------------------------------------------------------");
    println!("   Throughput           : {:>10} payloads / sec", payload_throughput);
    println!("   Average Latency      : {:>10.2} µs ({:.0} ns)", avg_payload_ns / 1000.0, avg_payload_ns);
    println!("   P50 Latency (median) : {:>10.2} µs", p50_payload as f64 / 1000.0);
    println!("   P95 Latency          : {:>10.2} µs", p95_payload as f64 / 1000.0);
    println!("   P99 Latency          : {:>10.2} µs", p99_payload as f64 / 1000.0);
    println!("================================================================================");

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}
