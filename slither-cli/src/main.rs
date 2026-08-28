use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use slither_core::{CrawlConfig, CrawlEvent, PipelineInput};
use tokio::sync::mpsc;

/// Single source of truth for the CLI version, from the crate manifest.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// True if two paths refer to the same file on disk.
///
/// Compared after canonicalisation so `c.json`, `./c.json` and an absolute path
/// are recognised as one file. Falls back to a literal comparison when the
/// output does not exist yet, which is the ordinary case.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The origin the generated sitemap files will be served from.
///
/// Only used when a crawl is large enough to need a `<sitemapindex>`, whose
/// `<loc>` entries the protocol requires to be absolute URLs. The audited site's
/// own origin is the only defensible guess; the crawl's seed URL carries it,
/// including a non-default port, and `crawl_metadata.domain` is the fallback for
/// a JSON whose seed URL will not parse.
fn sitemap_base_url(result: &slither_core::CrawlResult) -> String {
    url::Url::parse(&result.crawl_metadata.seed_url)
        .ok()
        .and_then(|u| {
            u.host_str().map(|host| {
                let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
                format!("{}://{}{}/", u.scheme(), host, port)
            })
        })
        .unwrap_or_else(|| format!("https://{}/", result.crawl_metadata.domain))
}

/// Format a count with its noun, appending "s" unless the count is exactly one
/// (e.g. `1 issue`, `3 issues`). Keeps terminal summaries grammatical.
fn count_noun(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[derive(Parser)]
#[command(name = "slither")]
#[command(version = VERSION)]
#[command(about = "Slither — fast, modular SEO toolkit, made for Scorpion", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Crawl a website and analyze its SEO attributes
    Crawl {
        /// The seed URL to begin crawling
        url: String,

        /// Maximum crawl depth from seed URL
        #[arg(short, long, default_value_t = 10)]
        depth: u32,

        /// Maximum number of pages to crawl
        #[arg(short, long, default_value_t = 500, value_parser = clap::value_parser!(u32).range(1..))]
        max_pages: u32,

        /// Maximum concurrent requests
        #[arg(short, long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
        concurrency: u32,

        /// Delay between requests to same host in ms
        #[arg(long, default_value_t = 250)]
        delay: u64,

        /// Output file path
        #[arg(short, long)]
        output: Option<String>,

        /// Skip HTML report generation (the report is written by default)
        #[arg(long)]
        no_html: bool,

        /// Custom User-Agent string
        #[arg(long, default_value = concat!("Slither/", env!("CARGO_PKG_VERSION"), " (SEO audit tool)"))]
        user_agent: String,

        /// Ignore robots.txt (not recommended)
        #[arg(long)]
        ignore_robots: bool,

        /// Also crawl subdomains of the target domain
        #[arg(long)]
        follow_subdomains: bool,

        /// Request timeout in seconds
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Suppress all output except errors
        #[arg(short, long)]
        quiet: bool,

        /// Compact (minified) JSON output
        #[arg(long)]
        json_compact: bool,

        /// Include body text in JSON output (excluded by default to reduce size)
        #[arg(long)]
        include_body_text: bool,

        /// Output only metadata, issues, and summary (no page data)
        #[arg(long)]
        summary_only: bool,

        /// Also generate a CSV export of page data
        #[arg(long)]
        csv: bool,

        /// Crawl backend: local (default) or playwright (needs the 'playwright' feature)
        #[arg(long, default_value = "local", value_parser = ["local", "playwright"])]
        backend: String,

        /// Cloudflare Account ID (or set CLOUDFLARE_ACCOUNT_ID)
        #[arg(long)]
        cf_account_id: Option<String>,

        /// Cloudflare API Token (or set CLOUDFLARE_API_TOKEN)
        #[arg(long)]
        cf_api_token: Option<String>,

        /// Skip HEAD requests for security headers in CF mode
        #[arg(long)]
        skip_header_check: bool,

        /// Path to Chrome/Chromium executable (auto-detected by default)
        #[arg(long)]
        chrome_path: Option<String>,

        /// Disable headless mode (show Chrome window)
        #[arg(long)]
        no_headless: bool,

        /// Extra wait time after page load for JS rendering (ms)
        #[arg(long, default_value_t = 2000)]
        render_wait_ms: u64,

        /// Enable PageSpeed/Core Web Vitals analysis
        #[arg(long)]
        pagespeed: bool,

        /// Google PageSpeed API key (or set PAGESPEED_API_KEY env var)
        #[arg(long)]
        pagespeed_key: Option<String>,

        /// Analyze only N random pages for PageSpeed (default: all)
        #[arg(long)]
        pagespeed_sample: Option<u32>,

        /// PageSpeed test strategy: mobile or desktop
        #[arg(long, default_value = "mobile", value_parser = ["mobile", "desktop"])]
        pagespeed_strategy: String,
    },

    /// Analyze the internal link graph of a crawl (PageRank, orphans, hubs)
    Link {
        /// Path to a crawl JSON file produced by `slither crawl`
        crawl_json: std::path::PathBuf,

        /// How many pages to show in the ranked lists
        #[arg(long, default_value_t = 15)]
        top: usize,

        /// Emit the full report as JSON instead of a terminal summary
        #[arg(long)]
        json: bool,
    },

    /// Generate an XML sitemap from a crawl's indexable pages
    Sitemap {
        /// Path to a crawl JSON file produced by `slither crawl`
        crawl_json: std::path::PathBuf,

        /// Write to this file instead of stdout
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },

    /// Take a full-page screenshot [cloudflare]
    Screenshot {
        /// URL to screenshot
        url: String,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Capture the full scrollable page
        #[arg(long)]
        full_page: bool,
        /// Image format: png or jpeg
        #[arg(long, default_value = "png", value_parser = ["png", "jpeg"])]
        format: String,
        /// JPEG quality (1-100)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        quality: Option<u32>,
        /// CSS selector to capture a specific element
        #[arg(long)]
        selector: Option<String>,
        /// Viewport size as WxH (e.g., 1920x1080)
        #[arg(long, default_value = "1920x1080")]
        viewport: String,
        /// Cloudflare Account ID
        #[arg(long)]
        cf_account_id: Option<String>,
        /// Cloudflare API Token
        #[arg(long)]
        cf_api_token: Option<String>,
    },

    /// Single-page SEO audit [cloudflare optional]
    Inspect {
        /// URL to inspect
        url: String,
        /// Use Cloudflare JS rendering
        #[arg(long)]
        rendered: bool,
        /// Compare static vs rendered side-by-side
        #[arg(long)]
        compare: bool,
        /// Wait for CSS selector before capturing (CF modes)
        #[arg(long)]
        wait_for: Option<String>,
        /// Save results as JSON
        #[arg(short, long)]
        output: Option<String>,
        /// Cloudflare Account ID
        #[arg(long)]
        cf_account_id: Option<String>,
        /// Cloudflare API Token
        #[arg(long)]
        cf_api_token: Option<String>,
    },

    /// AI-powered data extraction [cloudflare]
    Extract {
        /// URL to extract data from
        url: String,
        /// Custom extraction prompt
        #[arg(long)]
        prompt: Option<String>,
        /// File path to a custom JSON schema
        #[arg(long)]
        schema: Option<String>,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Wait for a CSS selector before extracting
        #[arg(long)]
        wait_for: Option<String>,
        /// Cloudflare Account ID
        #[arg(long)]
        cf_account_id: Option<String>,
        /// Cloudflare API Token
        #[arg(long)]
        cf_api_token: Option<String>,
    },

    /// Show installed component versions
    Version,

    /// Configure integrations
    Setup {
        /// Integration to set up (e.g., "cloudflare")
        integration: Option<String>,
    },

    /// Start the REST API and/or MCP server
    Serve {
        /// Port for the REST API (default: 3001)
        #[arg(short, long, default_value_t = 3001)]
        port: u16,
        /// Host to bind to (default: 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Run as MCP server over stdio instead of HTTP
        #[arg(long)]
        mcp: bool,
        /// API key for authentication (or set SLITHER_API_KEY env var)
        #[arg(long, env = "SLITHER_API_KEY")]
        api_key: Option<String>,
    },
}

fn show_dashboard() -> Result<()> {
    let green = console::Style::new().green();
    let green_bold = console::Style::new().green().bold();
    let dim = console::Style::new().dim();
    let cyan = console::Style::new().cyan();
    let bold = console::Style::new().bold();

    let border = green.apply_to("\u{2500}".repeat(58));
    let top_left = green.apply_to("\u{250c}");
    let top_right = green.apply_to("\u{2510}");
    let bot_left = green.apply_to("\u{2514}");
    let bot_right = green.apply_to("\u{2518}");
    let vert = green.apply_to("\u{2502}");
    let tee_left = green.apply_to("\u{251c}");
    let tee_right = green.apply_to("\u{2524}");

    // Header
    println!("  {}{}{}", top_left, border, top_right);
    println!(
        "  {}  {}{: >width$}{}",
        vert,
        green_bold.apply_to(format!("Slither v{VERSION}  \u{1F40D}")),
        "",
        vert,
        width = 34,
    );
    println!(
        "  {}  {}{: >width$}{}",
        vert,
        dim.apply_to("SEO audit toolkit"),
        "",
        vert,
        width = 39,
    );
    println!("  {}{}{}", tee_left, border, tee_right);

    // Commands section
    println!(
        "  {}  {}{: >width$}{}",
        vert,
        bold.apply_to("Commands"),
        "",
        vert,
        width = 48,
    );
    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);

    let commands = [
        ("crawl     ", "Crawl a site & generate audit data"),
        (
            "screenshot",
            "Full-page screenshot               [cloudflare]",
        ),
        ("inspect   ", "Single-page SEO audit  [cloudflare optional]"),
        (
            "extract   ",
            "AI-powered data extraction         [cloudflare]",
        ),
        (
            "link      ",
            "Internal link graph (PageRank, orphans, hubs)",
        ),
        ("sitemap   ", "Generate sitemap.xml from a crawl"),
        ("serve     ", "Start the REST API / MCP server"),
    ];
    for (name, desc) in &commands {
        let line = format!(
            "  {}  {}   {}",
            vert,
            cyan.apply_to(name),
            dim.apply_to(desc),
        );
        println!("{}", line);
    }

    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);
    println!("  {}{}{}", tee_left, border, tee_right);

    // Recent crawls
    println!(
        "  {}  {}{: >width$}{}",
        vert,
        bold.apply_to("Recent Crawls"),
        "",
        vert,
        width = 43,
    );
    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);

    // Try reading recent jobs from SQLite first, fall back to filesystem scanning
    let db_file = slither_core::jobs::db_path();
    let db_jobs: Option<Vec<slither_core::jobs::Job>> = if db_file.exists() {
        slither_core::jobs::db::open_db(&db_file)
            .ok()
            .and_then(|conn| {
                let jobs_dir = slither_core::jobs::jobs_dir();
                let mgr = slither_core::jobs::JobManager::new(conn, jobs_dir);
                let filter = slither_core::jobs::ListJobsFilter {
                    limit: 3,
                    ..Default::default()
                };
                mgr.list_jobs(&filter).ok()
            })
            .filter(|jobs| !jobs.is_empty())
    } else {
        None
    };

    if let Some(jobs) = db_jobs {
        for job in &jobs {
            let domain = &job.domain;
            let summary = job.result_summary.as_ref();
            let pages = summary
                .and_then(|s| s["pages_crawled"].as_u64())
                .unwrap_or(0);
            let grade = summary.and_then(|s| s["grade"].as_str()).unwrap_or("?");
            let score = summary
                .and_then(|s| s["health_score"].as_u64())
                .unwrap_or(0);

            let age = chrono::DateTime::parse_from_rfc3339(&job.created_at)
                .ok()
                .map(|dt| {
                    let elapsed = chrono::Utc::now().signed_duration_since(dt);
                    let hours = elapsed.num_hours();
                    if hours < 1 {
                        "just now".to_string()
                    } else if hours == 1 {
                        "1 hour ago".to_string()
                    } else if hours < 24 {
                        format!("{} hours ago", hours)
                    } else if hours < 48 {
                        "1 day ago".to_string()
                    } else if hours < 168 {
                        format!("{} days ago", hours / 24)
                    } else {
                        format!("{} weeks ago", hours / 168)
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());

            let grade_styled = match grade {
                "A" | "B" => style(grade).green().bold(),
                "C" => style(grade).yellow().bold(),
                _ => style(grade).red().bold(),
            };

            println!(
                "  {}  {:<20} {:>3} pages   {} ({})   {}",
                vert,
                cyan.apply_to(domain),
                pages,
                grade_styled,
                score,
                dim.apply_to(&age),
            );
        }
    } else {
        // Fallback: scan current directory for crawl-*.json files
        let mut crawl_files: Vec<_> = match std::fs::read_dir(".") {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    name.starts_with("crawl-") && name.ends_with(".json")
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        crawl_files.sort_by(|a, b| {
            let time_a = a
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let time_b = b
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            time_b.cmp(&time_a)
        });

        if crawl_files.is_empty() {
            println!(
                "  {}  {}",
                vert,
                dim.apply_to("No crawls yet -- run slither crawl <url> to get started"),
            );
        } else {
            for entry in crawl_files.iter().take(3) {
                let path = entry.path();

                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                        let domain = json["crawl_metadata"]["domain"]
                            .as_str()
                            .unwrap_or("unknown");
                        let pages = json["summary"]["total_pages"].as_u64().unwrap_or(0);
                        let grade = json["summary"]["grade"].as_str().unwrap_or("?");
                        let score = json["summary"]["health_score"].as_u64().unwrap_or(0);

                        let age = entry
                            .metadata()
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.elapsed().ok())
                            .map(|d| {
                                let hours = d.as_secs() / 3600;
                                if hours < 1 {
                                    "just now".to_string()
                                } else if hours == 1 {
                                    "1 hour ago".to_string()
                                } else if hours < 24 {
                                    format!("{} hours ago", hours)
                                } else if hours < 48 {
                                    "1 day ago".to_string()
                                } else if hours < 168 {
                                    format!("{} days ago", hours / 24)
                                } else {
                                    format!("{} weeks ago", hours / 168)
                                }
                            })
                            .unwrap_or_else(|| "unknown".to_string());

                        let grade_styled = match grade {
                            "A" | "B" => style(grade).green().bold(),
                            "C" => style(grade).yellow().bold(),
                            _ => style(grade).red().bold(),
                        };

                        println!(
                            "  {}  {:<20} {:>3} pages   {} ({})   {}",
                            vert,
                            cyan.apply_to(domain),
                            pages,
                            grade_styled,
                            score,
                            dim.apply_to(&age),
                        );
                        continue;
                    }
                }
                let name = entry.file_name().to_string_lossy().to_string();
                println!("  {}  {}", vert, dim.apply_to(&name));
            }
        }
    }

    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);
    println!("  {}{}{}", tee_left, border, tee_right);

    // Toolkit status
    println!(
        "  {}  {}{: >width$}{}",
        vert,
        bold.apply_to("Toolkit"),
        "",
        vert,
        width = 49,
    );

    let cf_available = std::env::var("CLOUDFLARE_ACCOUNT_ID").is_ok()
        && std::env::var("CLOUDFLARE_API_TOKEN").is_ok();

    let tools = [
        ("crawl", true),
        ("link", true),
        ("mcp", true),
        ("cloudflare", cf_available),
    ];

    let mut toolkit_line = format!("  {}  ", vert);
    for (name, installed) in &tools {
        if *installed {
            toolkit_line.push_str(&format!("{} {} ", name, green.apply_to("\u{2713}")));
        } else {
            toolkit_line.push_str(&format!("{} {} ", name, dim.apply_to("\u{2717}")));
        }
        toolkit_line.push_str("  ");
    }
    println!("{}", toolkit_line);

    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);

    // Rotating tip
    let tips = [
        "Connect to Claude: claude mcp add slither -- slither serve --mcp",
        "Add --csv to any crawl for spreadsheet-friendly output.",
        "Run slither link <crawl.json> for PageRank, orphans, and hubs.",
        "Ask Claude: \"crawl my site and tell me what to fix first.\"",
        "Set up Cloudflare for JS rendering: slither setup cloudflare",
        "Use slither extract <url> to pull business info with AI.",
        "Take screenshots: slither screenshot <url> --full-page",
    ];
    let tip_idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as usize % tips.len())
        .unwrap_or(0);

    println!(
        "  {}  {} {}",
        vert,
        bold.apply_to("Tip:"),
        dim.apply_to(tips[tip_idx]),
    );

    println!("  {}{: >width$}{}", vert, "", vert, width = 59,);
    println!("  {}{}{}", bot_left, border, bot_right);

    Ok(())
}

async fn setup_cloudflare() -> Result<()> {
    use dialoguer::Input;

    println!();
    println!(
        "  {} {}",
        style("Cloudflare Browser Rendering Setup").green().bold(),
        style("☁").cyan()
    );
    println!("  {}", style("─".repeat(40)).dim());
    println!();
    println!(
        "  {}",
        style("This connects Slither to Cloudflare's browser rendering API,").dim()
    );
    println!(
        "  {}",
        style("enabling JS rendering, screenshots, and AI extraction.").dim()
    );
    println!();
    println!(
        "  {}",
        style("Free tier includes 10 minutes of browser time per day.").dim()
    );
    println!();

    let account_id: String = Input::new()
        .with_prompt("  Cloudflare Account ID")
        .interact_text()?;

    println!(
        "  {}",
        style("(Create token at: dash.cloudflare.com/profile/api-tokens)").dim()
    );
    println!(
        "  {}",
        style("Required permission: Browser Rendering - Edit").dim()
    );

    let api_token: String = Input::new().with_prompt("  API Token").interact_text()?;

    // Validate — just check auth works
    print!("  Validating...");
    let validation = reqwest::Client::new()
        .get(format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/browser-rendering",
            account_id
        ))
        .header("Authorization", format!("Bearer {}", api_token))
        .send()
        .await;

    match validation {
        Ok(resp) if resp.status().is_success() => {
            println!(" {}", style("✓ Connected!").green());
        }
        // A 404 on the account-scoped endpoint means the token authenticated
        // but the account ID does not exist. Accepting it as success saved
        // credentials that fail on every later command.
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            println!(
                " {}",
                style("✗ Account not found — check your Account ID.").red()
            );
            std::process::exit(1);
        }
        Ok(resp)
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED
                || resp.status() == reqwest::StatusCode::FORBIDDEN =>
        {
            println!(
                " {}",
                style("✗ Auth failed — check your token and permissions.").red()
            );
            std::process::exit(1);
        }
        Ok(resp) => {
            println!(
                " {}",
                style(format!("✗ Unexpected response: {}", resp.status())).red()
            );
            std::process::exit(1);
        }
        Err(e) => {
            println!(
                " {}",
                style(format!("✗ Could not reach Cloudflare: {}", e)).red()
            );
            std::process::exit(1);
        }
    }

    // Write credentials to $SLITHER_HOME/.env with 0600 (not the CWD, which is
    // often a git repo), merging with any existing keys.
    let env_path = write_credentials(&[
        ("CLOUDFLARE_ACCOUNT_ID", &account_id),
        ("CLOUDFLARE_API_TOKEN", &api_token),
    ])?;
    println!(
        "  {} Credentials saved to {} (permissions 0600)",
        style("✓").green(),
        style(env_path.display()).cyan()
    );

    println!();
    println!(
        "  {} Try: {}",
        style("You're all set!").green().bold(),
        style("slither screenshot https://example.com").cyan()
    );
    println!();

    Ok(())
}

/// Get the user's home directory.
fn get_home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Run the internal link-graph analysis on a crawl JSON file.
fn run_link_graph(crawl_json: &std::path::Path, top: usize, as_json: bool) -> Result<()> {
    let data = std::fs::read_to_string(crawl_json)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", crawl_json.display()))?;
    let crawl: slither_core::CrawlResult = serde_json::from_str(&data)
        .map_err(|e| anyhow::anyhow!("Failed to parse crawl JSON: {e}"))?;

    let report = slither_core::link_graph::compute_link_graph(
        &crawl.pages,
        Some(&crawl.crawl_metadata.seed_url),
        top,
    );

    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "\n  {} \u{2014} internal link graph",
        style(format!("Slither v{VERSION}")).green().bold()
    );
    println!(
        "  {} nodes \u{00b7} {} edges \u{00b7} {} component(s) \u{00b7} avg out-degree {:.1}\n",
        report.total_nodes, report.total_edges, report.components, report.avg_out_degree,
    );

    println!(
        "  {}",
        style("Top pages by internal authority (PageRank)").bold()
    );
    for (i, p) in report.top_by_pagerank.iter().enumerate() {
        println!(
            "    {:>2}. {:.4}  {} ({} in / {} out)",
            i + 1,
            p.pagerank,
            style(&p.url).cyan(),
            p.in_degree,
            p.out_degree
        );
    }

    println!(
        "\n  {}",
        style("Orphan pages (no internal inbound links)").bold()
    );
    if report.orphan_pages.is_empty() {
        println!(
            "    {}",
            style("None \u{2014} every page is linked to.").green()
        );
    } else {
        for url in report.orphan_pages.iter().take(top) {
            println!("    {} {}", style("\u{2717}").red(), url);
        }
        if report.orphan_pages.len() > top {
            println!("    \u{2026} and {} more", report.orphan_pages.len() - top);
        }
    }

    println!("\n  {}", style("Silo distribution").bold());
    for s in report.silo_distribution.iter().take(top) {
        println!("    {:>4}  /{}", s.pages, s.silo);
    }
    println!();
    Ok(())
}

/// Resolve the Slither home directory (`$SLITHER_HOME`, else `~/.slither` /
/// `~/slither` on Windows). Credentials and the jobs DB live here.
fn slither_home_dir() -> std::path::PathBuf {
    std::env::var("SLITHER_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                get_home_dir().join("slither")
            } else {
                get_home_dir().join(".slither")
            }
        })
}

/// Write credentials to `$SLITHER_HOME/.env` with `0600` permissions, merging
/// with any existing keys instead of clobbering the file. Returns the path.
fn write_credentials(pairs: &[(&str, &str)]) -> Result<std::path::PathBuf> {
    let home = slither_home_dir();
    std::fs::create_dir_all(&home)?;
    let env_path = home.join(".env");

    // Load existing key=value lines so unrelated secrets are preserved.
    let mut lines: Vec<(String, String)> = Vec::new();
    if let Ok(existing) = std::fs::read_to_string(&env_path) {
        for line in existing.lines() {
            if let Some((k, v)) = line.split_once('=') {
                lines.push((k.trim().to_string(), v.to_string()));
            }
        }
    }
    for (key, val) in pairs {
        if let Some(entry) = lines.iter_mut().find(|(k, _)| k == key) {
            entry.1 = (*val).to_string();
        } else {
            lines.push((key.to_string(), (*val).to_string()));
        }
    }
    let contents: String = lines.iter().map(|(k, v)| format!("{k}={v}\n")).collect();

    // Create with 0600 up front on Unix so the token is never briefly world-readable.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&env_path)?;
        f.write_all(contents.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&env_path, contents.as_bytes())?;
    }
    Ok(env_path)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load credentials: a CWD .env first, then $SLITHER_HOME/.env. dotenvy does
    // not overwrite variables that are already set, so loading the CWD file
    // first is what gives it precedence for local overrides.
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_path(slither_home_dir().join(".env"));

    let cli = Cli::parse();

    match cli.command {
        None => {
            show_dashboard()?;
        }
        Some(Commands::Crawl {
            url,
            depth,
            max_pages,
            concurrency,
            delay,
            output,
            no_html,
            user_agent,
            ignore_robots,
            follow_subdomains,
            timeout,
            verbose,
            quiet,
            json_compact,
            include_body_text,
            summary_only,
            csv,
            backend,
            cf_account_id,
            cf_api_token,
            skip_header_check,
            chrome_path,
            no_headless,
            render_wait_ms,
            pagespeed,
            pagespeed_key,
            pagespeed_sample,
            pagespeed_strategy,
        }) => {
            // Logs go to stderr so they never interleave with the styled
            // summary or corrupt piped stdout (e.g. JSON output). At the
            // default level only warnings and errors surface — the pretty
            // summary already narrates normal progress; --verbose opens up
            // the full DEBUG trace for troubleshooting.
            if verbose {
                tracing_subscriber::fmt()
                    .with_writer(std::io::stderr)
                    .with_max_level(tracing::Level::DEBUG)
                    .init();
            } else if !quiet {
                tracing_subscriber::fmt()
                    .with_writer(std::io::stderr)
                    .with_max_level(tracing::Level::WARN)
                    .init();
            }

            let config = CrawlConfig {
                seed_url: url,
                max_depth: depth,
                max_pages,
                concurrency,
                delay_ms: delay,
                user_agent,
                ignore_robots,
                follow_subdomains,
                timeout_seconds: timeout,
                generate_html: !no_html,
                output_path: output,
                json_compact,
                include_body_text,
                summary_only,
                generate_csv: csv,
                backend,
                cf_account_id,
                cf_api_token,
                skip_header_check,
                chrome_path: chrome_path.clone(),
                headless: !no_headless,
                render_wait_ms,
                pagespeed,
                pagespeed_key: pagespeed_key.clone(),
                pagespeed_sample,
                pagespeed_strategy: pagespeed_strategy.clone(),
            };

            if !quiet {
                println!(
                    "  {} \u{1F40D}",
                    style(format!("Slither v{VERSION} \u{2014} crawl"))
                        .green()
                        .bold()
                );
                println!(
                    "  {} \u{00b7} {} \u{00b7} {} \u{00b7} {}",
                    style(&config.seed_url).cyan(),
                    style(format!("{} concurrent", config.concurrency)).dim(),
                    style(format!("{}ms delay", config.delay_ms)).dim(),
                    style(format!(
                        "max {} pages \u{00b7} depth {}",
                        config.max_pages, config.max_depth
                    ))
                    .dim(),
                );
                println!();
            }

            // ── Playwright backend ─────────────────────────────────
            if config.backend == "playwright" {
                #[cfg(feature = "playwright")]
                {
                    if !quiet {
                        println!(
                            "  {} Playwright backend \u{2014} local Chrome rendering",
                            style("\u{1f3ad}").cyan(),
                        );
                        println!();
                    }

                    let client = slither_core::playwright::PlaywrightClient::launch(
                        config.chrome_path.as_deref(),
                        config.headless,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                    let start_time = std::time::Instant::now();

                    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<CrawlEvent>();

                    // Progress bar
                    let pb = ProgressBar::new(0);
                    pb.set_style(
                        ProgressStyle::with_template(
                            "  {spinner:.green} Rendering {pos}/{len} pages  [{elapsed_precise}]",
                        )
                        .unwrap()
                        .tick_chars("\u{28cb}\u{28d9}\u{28f9}\u{28f8}\u{28fc}\u{28f4}\u{28e6}\u{28e7}\u{28c7}\u{280f}"),
                    );
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));

                    let config_clone = config.clone();
                    let seed = config.seed_url.clone();

                    let crawl_handle = tokio::spawn(async move {
                        slither_core::playwright::crawl::run_playwright_crawl(
                            &client,
                            &seed,
                            &config_clone,
                            Some(event_tx),
                        )
                        .await
                    });

                    // Listen for events
                    while let Some(event) = event_rx.recv().await {
                        match event {
                            CrawlEvent::PageCrawled { url, status, .. } => {
                                if !quiet {
                                    pb.println(format!(
                                        "  {} {} [{}]",
                                        style("\u{25cf}").green(),
                                        url,
                                        status,
                                    ));
                                }
                            }
                            CrawlEvent::PageError { url, error } => {
                                if !quiet {
                                    pb.println(format!(
                                        "  {} {} \u{2014} {}",
                                        style("\u{2717}").red(),
                                        url,
                                        style(&error).red(),
                                    ));
                                }
                            }
                            CrawlEvent::QueueUpdate {
                                crawled,
                                estimated_total,
                                ..
                            } => {
                                pb.set_length(estimated_total as u64);
                                pb.set_position(crawled as u64);
                            }
                            _ => {}
                        }
                    }

                    pb.finish_and_clear();
                    let (mut pages, pages_crawled, pages_errored, pages_skipped) =
                        crawl_handle.await??;

                    let duration_ms = start_time.elapsed().as_millis() as u64;

                    // Run header requests for security headers
                    if !config.skip_header_check && !pages.is_empty() {
                        if !quiet {
                            println!(
                                "  {} Fetching security headers...",
                                style("\u{1f512}").dim()
                            );
                        }
                        slither_core::crawler::head_requests::run_header_requests(
                            &mut pages,
                            &config.user_agent,
                            config.timeout_seconds,
                            config.concurrency,
                        )
                        .await;
                    }

                    // PageSpeed enrichment
                    slither_core::enrich_pagespeed(&mut pages, &config).await;

                    // The playwright backend drives its own browser fetches, so
                    // robots.txt is never captured during the crawl. Fetch it
                    // here or the AI-crawler analyzer silently reports nothing.
                    let robots_txt = slither_core::crawler::robots::RobotsChecker::fetch(
                        &config.seed_url,
                        &config.user_agent,
                        config.timeout_seconds,
                    )
                    .await
                    .ok()
                    .map(|r| r.raw().to_string())
                    .filter(|raw| !raw.is_empty());

                    let pages_discovered = pages_crawled + pages_errored + pages_skipped;
                    let mut result = slither_core::run_post_crawl_pipeline(PipelineInput {
                        pages,
                        config: config.clone(),
                        duration_ms,
                        pages_discovered,
                        pages_crawled,
                        pages_skipped_robots: pages_skipped,
                        pages_errored,
                        backend: "playwright".to_string(),
                        sitemap_data: None,
                        robots_txt,
                    })?;

                    let write_config =
                        slither_core::output::WriteConfig::from_crawl_config(&config);
                    let paths =
                        slither_core::output::write_crawl_output(&mut result, &write_config)?;

                    if !quiet {
                        let duration = duration_ms as f64 / 1000.0;
                        println!(
                            "  {} Crawl complete \u{2014} {} pages in {:.1}s (Playwright)",
                            style("\u{2713}").green(),
                            result.summary.total_pages,
                            duration,
                        );
                        println!();
                        println!("  {}", style("Output:").bold());
                        println!("    {}", style(paths.json.display()).cyan());
                        if let Some(ref html) = paths.html {
                            println!("    {}", style(html.display()).cyan());
                        }
                        if let Some(ref csv) = paths.csv {
                            println!("    {}", style(csv.display()).cyan());
                        }
                        println!();
                    }

                    return Ok(());
                }

                #[cfg(not(feature = "playwright"))]
                {
                    eprintln!(
                        "  {} Playwright backend requires the 'playwright' feature.",
                        style("\u{2717}").red(),
                    );
                    eprintln!("    Rebuild with: cargo build --features playwright");
                    std::process::exit(1);
                }
            }

            // Clone config before moving into the spawn, for post-crawl PageSpeed enrichment
            let config_for_output = config.clone();

            // Create event channel
            let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);

            // Spawn the crawler
            let crawl_handle =
                tokio::spawn(async move { slither_core::crawler::crawl(config, event_tx).await });

            // Progress bar
            let pb = ProgressBar::new(0);
            pb.set_style(
                ProgressStyle::with_template(
                    "  {spinner:.green} Crawling {pos}/{len} pages  [{elapsed_precise}]  {msg}",
                )
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(100));

            // Listen for events
            while let Some(event) = event_rx.recv().await {
                match event {
                    CrawlEvent::Started { domain, .. } => {
                        if !quiet {
                            pb.set_message(domain.to_string());
                        }
                    }
                    CrawlEvent::RobotsFetched {
                        rules_count,
                        crawl_delay,
                    } => {
                        if !quiet {
                            let delay_msg = crawl_delay
                                .map(|d| format!(", crawl-delay: {}s", d))
                                .unwrap_or_default();
                            pb.println(format!(
                                "  {} robots.txt: {} rules{}",
                                style("\u{25cf}").green(),
                                rules_count,
                                delay_msg
                            ));
                        }
                    }
                    CrawlEvent::PageCrawled { url, status, .. } => {
                        if verbose {
                            let status_style = if status >= 400 {
                                style(status).red()
                            } else if status >= 300 {
                                style(status).yellow()
                            } else {
                                style(status).green()
                            };
                            pb.println(format!(
                                "  {} {} {}",
                                status_style,
                                url,
                                style("\u{2713}").green()
                            ));
                        }
                    }
                    CrawlEvent::PageError { url, error } => {
                        if !quiet {
                            pb.println(format!(
                                "  {} {} \u{2014} {}",
                                style("\u{2717}").red(),
                                url,
                                style(&error).red()
                            ));
                        }
                    }
                    CrawlEvent::QueueUpdate {
                        crawled,
                        estimated_total,
                        ..
                    } => {
                        pb.set_length(estimated_total as u64);
                        pb.set_position(crawled as u64);
                    }
                    CrawlEvent::Completed { result } => {
                        pb.finish_and_clear();
                        if !quiet {
                            let duration = result.crawl_metadata.duration_ms as f64 / 1000.0;

                            println!(
                                "  {} Crawl complete \u{2014} {} pages in {:.1}s",
                                style("\u{2713}").green(),
                                result.summary.total_pages,
                                duration
                            );
                            // A URL that never returned a response appears in no
                            // page row and no issue, so without this line a host
                            // that refused 50 of 51 connections printed a green
                            // "Crawl complete — 1 pages" and nothing else.
                            let errored = result.crawl_metadata.pages_errored;
                            let skipped = result.crawl_metadata.pages_skipped_robots;
                            if errored > 0 || skipped > 0 {
                                let mut parts: Vec<String> = Vec::new();
                                if errored > 0 {
                                    parts.push(format!("{} unreachable", errored));
                                }
                                if skipped > 0 {
                                    parts.push(format!("{} blocked by robots.txt", skipped));
                                }
                                println!(
                                    "  {} {} \u{2014} not included in the results below",
                                    style("\u{26a0}").yellow(),
                                    parts.join(", "),
                                );
                            }
                            println!();

                            // Summary section
                            println!("  {}", style("Summary:").bold());

                            // Health score with colored grade
                            let grade = &result.summary.grade;
                            let score = result.summary.health_score;
                            match grade.as_str() {
                                "A" | "B" => println!(
                                    "    Health   {} ({}/100)",
                                    style(grade).green().bold(),
                                    score,
                                ),
                                "C" => println!(
                                    "    Health   {} ({}/100)",
                                    style(grade).yellow().bold(),
                                    score,
                                ),
                                _ => println!(
                                    "    Health   {} ({}/100)",
                                    style(grade).red().bold(),
                                    score,
                                ),
                            }

                            // Response time
                            println!(
                                "    Response {}ms avg \u{00b7} p50: {}ms \u{00b7} p95: {}ms",
                                result.summary.avg_response_time_ms,
                                result.summary.response_time_p50_ms,
                                result.summary.response_time_p95_ms,
                            );

                            // Issues
                            let total_issues = result.summary.total_issues;
                            if total_issues > 0 {
                                println!(
                                    "    Issues   {} found ({} critical \u{00b7} {} warning \u{00b7} {} info)",
                                    style(total_issues).yellow(),
                                    style(result.summary.critical_issues).red(),
                                    result.summary.warning_issues,
                                    result.summary.info_issues,
                                );

                                // Top categories by issue count
                                let mut cats: Vec<_> = result
                                    .summary
                                    .issues_by_category
                                    .iter()
                                    .filter(|(_, s)| s.issues_found > 0)
                                    .collect();
                                cats.sort_by(|a, b| {
                                    b.1.critical
                                        .cmp(&a.1.critical)
                                        .then(b.1.warning.cmp(&a.1.warning))
                                        .then(b.1.affected_urls.cmp(&a.1.affected_urls))
                                });
                                for (name, summary) in cats.iter().take(3) {
                                    println!(
                                        "             {} \u{2014} {}, {}",
                                        style(name).dim(),
                                        count_noun(summary.issues_found, "issue"),
                                        count_noun(summary.affected_urls, "URL"),
                                    );
                                }
                            } else {
                                println!("    Issues   {}", style("None found").green(),);
                            }
                            println!();
                        }
                    }
                }
            }

            // Wait for crawler to finish and get the result
            let mut result = crawl_handle.await??;

            // PageSpeed enrichment (re-scores after enrichment)
            if config_for_output.pagespeed {
                slither_core::enrich_pagespeed(&mut result.pages, &config_for_output).await;
                let robots_txt = result.robots_txt.clone();
                let sitemap_data = result.sitemap_data.clone();
                // Re-run pipeline to recalculate scores with CWV data
                result = slither_core::run_post_crawl_pipeline(PipelineInput {
                    pages: result.pages,
                    config: config_for_output.clone(),
                    duration_ms: result.crawl_metadata.duration_ms,
                    pages_discovered: result.crawl_metadata.pages_discovered,
                    pages_crawled: result.crawl_metadata.pages_crawled,
                    pages_skipped_robots: result.crawl_metadata.pages_skipped_robots,
                    pages_errored: result.crawl_metadata.pages_errored,
                    backend: "local".to_string(),
                    sitemap_data,
                    robots_txt,
                })?;
            }

            // Detect JS-rendered pages
            if let Some((js_count, total_count)) =
                slither_core::crawler::detect_js_rendering(&result.pages)
            {
                println!(
                    "\n  {} {} of {} pages appear to be JavaScript-rendered.",
                    style("\u{26a0}").yellow(),
                    js_count,
                    total_count,
                );
                println!(
                    "    {}",
                    style("Re-run with --backend cloudflare for better results.").dim()
                );
                println!();
            }

            let write_config = slither_core::output::WriteConfig::from_crawl_config(
                &result.crawl_metadata.settings,
            );
            let paths = slither_core::output::write_crawl_output(&mut result, &write_config)?;

            if !quiet {
                println!("  {}", style("Output:").bold());
                println!("    JSON  {}", style(paths.json.display()).cyan());
                if let Some(csv_path) = &paths.csv {
                    println!("    CSV   {}", style(csv_path.display()).cyan());
                }
                if let Some(html_path) = &paths.html {
                    println!("    HTML  {}", style(html_path.display()).cyan());
                }
                println!();
                println!("  {}", style("Next:").dim());
                println!(
                    "    {}",
                    style(format!("slither link {}", paths.json.display())).cyan()
                );
                println!(
                    "    {}",
                    style("Connect to Claude: slither serve --mcp").cyan()
                );
                println!();
            }
        }
        Some(Commands::Link {
            crawl_json,
            top,
            json,
        }) => {
            run_link_graph(&crawl_json, top, json)?;
        }
        Some(Commands::Sitemap { crawl_json, output }) => {
            use slither_core::report::sitemap_gen::{generate_sitemap_set, MAX_URLS_PER_SITEMAP};

            let data = std::fs::read_to_string(&crawl_json)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", crawl_json.display()))?;
            let result: slither_core::CrawlResult = serde_json::from_str(&data)
                .map_err(|e| anyhow::anyhow!("Failed to parse crawl JSON: {e}"))?;

            // `generate_sitemap_set` is the guarded entry point: it returns no
            // files when nothing qualifies. The unguarded `generate_sitemap`
            // used here before emitted an empty `<urlset>` — invalid against the
            // sitemaps.org schema, which requires at least one `<url>`, and
            // reported by Search Console as "Sitemap is empty" — while the
            // command printed a tick and exited 0. A `--summary-only` crawl
            // JSON carries no pages array at all, so a perfectly healthy site
            // produced exactly that file.
            let files = generate_sitemap_set(&result.pages, &sitemap_base_url(&result));
            if files.is_empty() {
                if result.pages.is_empty() {
                    anyhow::bail!(
                        "{} carries no page data, so there is nothing to put in a sitemap. \
                         A crawl written with --summary-only omits the pages array; re-run \
                         `slither crawl` without that flag and generate the sitemap from the \
                         full JSON.",
                        crawl_json.display()
                    );
                }
                anyhow::bail!(
                    "no indexable pages in {} — every crawled page is noindex, non-HTML, an \
                     error response, or canonicalized elsewhere. Refusing to write an empty \
                     <urlset>: it fails the sitemaps.org schema and search engines reject it \
                     as \"Sitemap is empty\".",
                    crawl_json.display()
                );
            }

            match output {
                Some(path) => {
                    // Writing the sitemap over its own input destroys the crawl
                    // it was generated from, and a crawl can represent hours of
                    // work. Refuse rather than report success on a data loss.
                    if same_file(&crawl_json, &path) {
                        anyhow::bail!(
                            "refusing to write the sitemap over its own input ({}). \
                             Choose a different --output path.",
                            path.display()
                        );
                    }
                    // files[0] is the single `<urlset>`, or the `<sitemapindex>`
                    // when the crawl exceeded the protocol's per-file URL limit.
                    // The index references its chunks by bare filename, so they
                    // have to land in the same directory for it to resolve.
                    std::fs::write(&path, &files[0].xml)?;
                    eprintln!("  {} Wrote {}", style("✓").green(), path.display());

                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    for f in &files[1..] {
                        let chunk = dir.join(&f.filename);
                        if same_file(&crawl_json, &chunk) {
                            anyhow::bail!(
                                "refusing to write the sitemap over its own input ({}). \
                                 Choose a different --output path.",
                                chunk.display()
                            );
                        }
                        std::fs::write(&chunk, &f.xml)?;
                        eprintln!("  {} Wrote {}", style("✓").green(), chunk.display());
                    }
                }
                None => {
                    if files.len() > 1 {
                        anyhow::bail!(
                            "this crawl needs {} sitemap files (a single file may hold at most \
                             {MAX_URLS_PER_SITEMAP} URLs), which cannot be written to stdout. \
                             Re-run with --output <path>.",
                            files.len()
                        );
                    }
                    print!("{}", files[0].xml);
                }
            }
        }
        Some(Commands::Screenshot {
            url,
            output,
            full_page,
            format,
            quality,
            selector,
            viewport,
            cf_account_id,
            cf_api_token,
        }) => {
            #[cfg(feature = "cloudflare")]
            {
                use slither_core::cloudflare::screenshot::{take_screenshot, ScreenshotConfig};
                use slither_core::cloudflare::CloudflareClient;

                let client = match CloudflareClient::new(cf_account_id, cf_api_token) {
                    Some(c) => c,
                    None => {
                        println!("{}", CloudflareClient::not_configured_message());
                        std::process::exit(1);
                    }
                };

                // Parse viewport "WxH"
                let (vw, vh) = match viewport.split_once('x') {
                    Some((w, h)) => {
                        let w: u32 = w.parse().unwrap_or_else(|_| {
                            eprintln!(
                                "  {} Invalid viewport width: {}",
                                style("\u{2717}").red(),
                                w,
                            );
                            std::process::exit(1);
                        });
                        let h: u32 = h.parse().unwrap_or_else(|_| {
                            eprintln!(
                                "  {} Invalid viewport height: {}",
                                style("\u{2717}").red(),
                                h,
                            );
                            std::process::exit(1);
                        });
                        (w, h)
                    }
                    None => {
                        eprintln!(
                            "  {} Invalid viewport format: {} (expected WxH, e.g. 1920x1080)",
                            style("\u{2717}").red(),
                            viewport,
                        );
                        std::process::exit(1);
                    }
                };

                let config = ScreenshotConfig {
                    url: url.clone(),
                    full_page,
                    format: format.clone(),
                    quality,
                    selector,
                    viewport_width: vw,
                    viewport_height: vh,
                };

                let bytes = take_screenshot(&client, &config).await?;

                // Build default output path: screenshot-<domain>-<date>.<format>
                let out_path = match output {
                    Some(p) => p,
                    None => {
                        let domain = url::Url::parse(&url)
                            .ok()
                            .and_then(|u| u.host_str().map(|h| h.to_string()))
                            .unwrap_or_else(|| "unknown".to_string());
                        let date = chrono::Utc::now().format("%Y-%m-%d");
                        format!("screenshot-{}-{}.{}", domain, date, format)
                    }
                };

                std::fs::write(&out_path, &bytes)?;

                let size_kb = bytes.len() as f64 / 1024.0;
                println!(
                    "  {} Saved screenshot to {} ({:.1} KB)",
                    style("\u{2713}").green(),
                    style(&out_path).cyan(),
                    size_kb,
                );
            }

            #[cfg(not(feature = "cloudflare"))]
            {
                let _ = (
                    url,
                    output,
                    full_page,
                    format,
                    quality,
                    selector,
                    viewport,
                    cf_account_id,
                    cf_api_token,
                );
                eprintln!(
                    "  {} The screenshot command requires the 'cloudflare' feature.",
                    style("\u{2717}").red(),
                );
                eprintln!("    Rebuild with: cargo build --features cloudflare");
                std::process::exit(1);
            }
        }
        Some(Commands::Extract {
            url,
            prompt,
            schema,
            output,
            wait_for,
            cf_account_id,
            cf_api_token,
        }) => {
            #[cfg(feature = "cloudflare")]
            {
                use slither_core::cloudflare::extract;
                use slither_core::cloudflare::CloudflareClient;

                let client = match CloudflareClient::new(cf_account_id, cf_api_token) {
                    Some(c) => c,
                    None => {
                        println!("{}", CloudflareClient::not_configured_message());
                        std::process::exit(1);
                    }
                };

                // Load custom schema from file if provided
                let schema_str = match &schema {
                    Some(path) => {
                        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
                            eprintln!(
                                "  {} Failed to read schema file {}: {}",
                                style("\u{2717}").red(),
                                path,
                                e,
                            );
                            std::process::exit(1);
                        });
                        Some(content)
                    }
                    None => None,
                };

                let is_seo_preset = prompt.is_none() && schema_str.is_none();

                let result = extract::extract(
                    &client,
                    &url,
                    prompt.as_deref(),
                    schema_str.as_deref(),
                    wait_for.as_deref(),
                )
                .await?;

                if is_seo_preset {
                    // Formatted SEO output
                    let domain = url::Url::parse(&url)
                        .ok()
                        .and_then(|u| u.host_str().map(|h| h.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());

                    println!(
                        "  {} {}",
                        style("Extract").green().bold(),
                        style(&url).cyan(),
                    );
                    println!();
                    println!("{}", extract::format_seo_result(&result));
                    println!();

                    // Auto-save or use --output
                    let out_path = match output {
                        Some(p) => p,
                        None => {
                            let date = chrono::Utc::now().format("%Y-%m-%d");
                            format!("extract-{}-{}.json", domain, date)
                        }
                    };

                    let json = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| result.to_string());
                    std::fs::write(&out_path, &json)?;

                    println!(
                        "  {} Saved to {}",
                        style("\u{2713}").green(),
                        style(&out_path).cyan(),
                    );
                } else {
                    // Custom prompt/schema — raw pretty JSON to stdout
                    let json = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| result.to_string());

                    if let Some(path) = output {
                        std::fs::write(&path, &json)?;
                        println!(
                            "  {} Saved to {}",
                            style("\u{2713}").green(),
                            style(&path).cyan(),
                        );
                    } else {
                        println!("{}", json);
                    }
                }
            }

            #[cfg(not(feature = "cloudflare"))]
            {
                let _ = (
                    url,
                    prompt,
                    schema,
                    output,
                    wait_for,
                    cf_account_id,
                    cf_api_token,
                );
                eprintln!(
                    "  {} The extract command requires the 'cloudflare' feature.",
                    style("\u{2717}").red(),
                );
                eprintln!("    Rebuild with: cargo build --features cloudflare");
                std::process::exit(1);
            }
        }
        Some(Commands::Inspect {
            url,
            rendered,
            compare,
            wait_for,
            output,
            cf_account_id,
            cf_api_token,
        }) => {
            use slither_core::inspect::{
                format_compare_table, format_inspect_output, run_inspect, InspectMode,
            };

            // Determine mode: --compare wins over --rendered, default is Static
            let mode = if compare {
                InspectMode::Compare
            } else if rendered {
                InspectMode::Rendered
            } else {
                InspectMode::Static
            };

            // For Rendered/Compare modes we need a Cloudflare client
            let cf_client = if mode == InspectMode::Rendered || mode == InspectMode::Compare {
                #[cfg(feature = "cloudflare")]
                {
                    use slither_core::cloudflare::CloudflareClient;

                    match CloudflareClient::new(cf_account_id.clone(), cf_api_token.clone()) {
                        Some(c) => Some(c),
                        None => {
                            println!("{}", CloudflareClient::not_configured_message());
                            std::process::exit(1);
                        }
                    }
                }
                #[cfg(not(feature = "cloudflare"))]
                {
                    let _ = (&cf_account_id, &cf_api_token);
                    eprintln!(
                        "  {} Rendered/compare modes require the 'cloudflare' feature.",
                        style("\u{2717}").red(),
                    );
                    eprintln!("    Rebuild with: cargo build --features cloudflare");
                    std::process::exit(1);
                }
            } else {
                None
            };

            let result =
                run_inspect(&url, mode.clone(), wait_for.as_deref(), cf_client.as_ref()).await?;

            // For Compare mode, print comparison table first
            if mode == InspectMode::Compare {
                if let (Some(sp), Some(si)) = (&result.static_page, &result.static_issues) {
                    println!(
                        "{}",
                        format_compare_table(&url, sp, &result.page, si, &result.issues)
                    );
                }
                println!();
                println!("  {}", style("Full audit (rendered):").bold(),);
            }

            // Always print the primary inspection output
            println!(
                "{}",
                format_inspect_output(&url, &result.page, &result.issues)
            );

            // Save JSON if --output provided
            if let Some(path) = output {
                let json = serde_json::to_string_pretty(&result)?;
                std::fs::write(&path, &json)?;
                println!(
                    "  {} Saved to {}",
                    style("\u{2713}").green(),
                    style(&path).cyan(),
                );
            }
        }
        Some(Commands::Setup { integration }) => match integration.as_deref() {
            Some("cloudflare") | Some("cf") => {
                setup_cloudflare().await?;
            }
            Some(other) => {
                eprintln!("  {} Unknown integration: {}", style("✗").red(), other);
                println!("  Available: cloudflare");
                std::process::exit(1);
            }
            None => {
                println!("  Available integrations:");
                println!(
                    "    {}  {}",
                    style("cloudflare").cyan(),
                    style("Browser Rendering for screenshot/extract/inspect").dim()
                );
                println!("\n  Run: slither setup cloudflare");
            }
        },
        Some(Commands::Serve {
            port,
            host,
            mcp,
            api_key,
        }) => {
            #[cfg(feature = "server")]
            {
                let config = slither_server::ServerConfig {
                    host,
                    port,
                    api_key,
                };
                slither_server::run(mcp, config).await?;
            }
            #[cfg(not(feature = "server"))]
            {
                let _ = (port, host, mcp, api_key);
                eprintln!("The serve command requires the 'server' feature.");
                eprintln!("Rebuild with: cargo build --features server");
                std::process::exit(1);
            }
        }
        Some(Commands::Version) => {
            println!(
                "  {} \u{1F40D}",
                style(format!("Slither v{VERSION}")).green().bold()
            );

            println!("    crawl    {VERSION}  {}", style("(built-in)").dim());
            println!("    link     {VERSION}  {}", style("(built-in)").dim());
            println!("    mcp      {VERSION}  {}", style("(built-in)").dim());
        }
    }

    Ok(())
}

#[cfg(test)]
mod sitemap_cli_tests {
    use super::{sitemap_base_url, CrawlConfig, CrawlEvent};
    use slither_core::report::sitemap_gen::generate_sitemap_set;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    fn result_with(seed_url: &str, domain: &str) -> slither_core::CrawlResult {
        let settings = serde_json::to_value(CrawlConfig {
            seed_url: seed_url.to_string(),
            ..Default::default()
        })
        .unwrap();
        let summary = serde_json::to_value(slither_core::crawler::build_summary(&[])).unwrap();
        let json = serde_json::json!({
            "slither_version": "0",
            "crawl_metadata": {
                "domain": domain,
                "seed_url": seed_url,
                "crawl_date": "2020-01-01T00:00:00Z",
                "duration_ms": 0,
                "pages_discovered": 0,
                "pages_crawled": 0,
                "pages_skipped_robots": 0,
                "pages_errored": 0,
                "settings": settings,
                "backend": "local",
            },
            "export_settings": serde_json::to_value(
                slither_core::models::crawl_result::ExportSettings::default(),
            )
            .unwrap(),
            "pages": [],
            "issues": { "issues": [] },
            "summary": summary,
        });
        serde_json::from_value(json).expect("fixture crawl JSON")
    }

    /// A `<sitemapindex>` must reference its chunk files by absolute URL, and the
    /// origin includes a non-default port — a crawl of a site served on :8080
    /// would otherwise publish an index pointing at port 80.
    #[test]
    fn the_index_base_is_the_audited_origin() {
        assert_eq!(
            sitemap_base_url(&result_with("https://example.com/blog/", "example.com")),
            "https://example.com/"
        );
        assert_eq!(
            sitemap_base_url(&result_with("http://127.0.0.1:9871/", "127.0.0.1")),
            "http://127.0.0.1:9871/"
        );
        // An unparseable seed falls back to the recorded domain.
        assert_eq!(
            sitemap_base_url(&result_with("not a url", "example.com")),
            "https://example.com/"
        );
    }

    /// Regression: `slither sitemap` called the unguarded `generate_sitemap`, so a
    /// crawl JSON with no usable pages — a `--summary-only` export strips the
    /// pages array entirely — produced an empty `<urlset>`. That document fails
    /// the sitemaps.org schema, which requires at least one `<url>`, and Search
    /// Console reports it as "Sitemap is empty"; the command printed a tick and
    /// exited 0. The guarded entry point returns no files instead, which is what
    /// the command now turns into a non-zero exit.
    #[test]
    fn a_pageless_crawl_yields_no_sitemap_file() {
        let result = result_with("https://example.com/", "example.com");
        assert!(result.pages.is_empty());
        assert!(
            generate_sitemap_set(&result.pages, &sitemap_base_url(&result)).is_empty(),
            "an empty crawl must not produce a sitemap file to write"
        );
    }

    /// Serve a small site whose internal links all carry fragments, and one page
    /// that is `noindex` for Google News only.
    ///
    /// This is the sqlite.org shape: the first time the crawler meets a page it
    /// is through an anchored link. Lives in the CLI's test binary rather than
    /// slither-core's because it must set `SLITHER_ALLOW_PRIVATE_TARGETS`, which is
    /// process-global — setting it inside the library's unit-test binary
    /// disables the SSRF guard that `net_guard`'s own tests assert is armed.
    async fn spawn_anchored_site() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();

                    let body = match path.as_str() {
                        "/" => "<!doctype html><html><head>\
                             <title>Home page of the anchored fixture</title></head><body>\
                             <h1>Home</h1><p>Every link below carries a fragment.</p>\
                             <a href=\"/aggfunc.html#aggfunclist\">agg</a>\
                             <a href=\"/toc.html#toc\">toc</a>\
                             <a href=\"/about/#a%20b\">about</a>\
                             <a href=\"/news.html#q?x=1\">news</a></body></html>"
                            .to_string(),
                        // Excluded from Google News, fully indexable in Search.
                        "/news.html" => "<!doctype html><html><head>\
                             <title>An article that is not in Google News</title>\
                             <meta name=\"googlebot-news\" content=\"noindex\"></head><body>\
                             <h1>News</h1><p>Body copy.</p><a href=\"/\">home</a></body></html>"
                            .to_string(),
                        _ => format!(
                            "<!doctype html><html><head><title>Fixture page {path}</title></head>\
                             <body><h1>{path}</h1><p>Body copy.</p>\
                             <a href=\"/\">home</a></body></html>"
                        ),
                    };
                    let response = if path == "/robots.txt" {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };

                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });

        (port, handle)
    }

    /// End-to-end cover for the two defects that reached `<loc>`:
    ///
    /// * A page first discovered through an anchored link adopted the fragment
    ///   as its identity, so it shipped in the sitemap as `/lang_aggfunc.html
    ///   #aggfunclist` — 3 of 25 entries on a real sqlite.org crawl. The URL
    ///   checks then measured the fragment too: "URL contains spaces" for a
    ///   `%20` that was only in the fragment, an inflated character count, and
    ///   "has parameters" for a `?` inside one.
    /// * `<meta name="googlebot-news" content="noindex">` scopes to Google News
    ///   alone, but was merged into the general robots rules, which dropped a
    ///   Search-indexable article out of the generated sitemap entirely.
    #[tokio::test]
    async fn a_crawled_sitemap_carries_clean_urls_and_news_scoped_pages() {
        std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
        let (port, _server) = spawn_anchored_site().await;

        let config = CrawlConfig {
            seed_url: format!("http://127.0.0.1:{port}/"),
            max_depth: 3,
            max_pages: 20,
            concurrency: 2,
            delay_ms: 0,
            ignore_robots: true,
            generate_html: false,
            ..Default::default()
        };

        let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);
        let drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
        let result = slither_core::crawler::crawl(config, event_tx)
            .await
            .expect("crawl should succeed");
        drain.await.unwrap();

        let urls: Vec<&str> = result.pages.iter().map(|p| p.url.as_str()).collect();
        assert!(urls.len() >= 5, "expected the whole fixture, got {urls:?}");
        for p in &result.pages {
            assert!(
                !p.url.contains('#'),
                "page identity kept its fragment: {}",
                p.url
            );
            assert_eq!(
                p.url_length as usize,
                p.url.len(),
                "url_length disagrees with the recorded URL for {}",
                p.url
            );
            assert_eq!(
                p.has_parameters,
                p.url.contains('?'),
                "has_parameters disagrees with the recorded URL for {}",
                p.url
            );
        }
        // The trailing slash is deliberate and must survive fragment stripping.
        assert!(
            urls.iter().any(|u| u.ends_with("/about/")),
            "the trailing slash was lost: {urls:?}"
        );

        let files = generate_sitemap_set(&result.pages, &sitemap_base_url(&result));
        assert_eq!(files.len(), 1, "one small file, no <sitemapindex>");
        let xml = &files[0].xml;
        assert!(!xml.contains('#'), "a fragment reached <loc>:\n{xml}");
        assert!(
            xml.contains("<loc>http://127.0.0.1:") && xml.contains("/news.html</loc>"),
            "a News-only noindex dropped a Search-indexable page from the sitemap:\n{xml}"
        );
        assert_eq!(xml.matches("<loc>").count(), result.pages.len());
    }
}
