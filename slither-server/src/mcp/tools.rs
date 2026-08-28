use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{json, Value};
use slither_core::jobs::{JobStatus, JobType, ListJobsFilter};
use slither_core::models::crawl_result::CrawlResult;

use crate::AppState;

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

pub fn tool_definitions() -> Vec<Value> {
    vec![
        // 1. slither_crawl
        json!({
            "name": "slither_crawl",
            "description": "Start a website crawl for SEO analysis. Returns a job_id to track progress. Use slither_status to check when it completes, then slither_summary and slither_query to explore results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The seed URL to begin crawling (e.g. https://example.com)"
                    },
                    "max_pages": {
                        "type": "integer",
                        "description": "Maximum number of pages to crawl (default: 500)",
                        "default": 500
                    },
                    "concurrency": {
                        "type": "integer",
                        "description": "Number of concurrent requests (default: 3)",
                        "default": 3
                    },
                    "delay_ms": {
                        "type": "integer",
                        "description": "Delay between requests in milliseconds (default: 250)",
                        "default": 250
                    },
                    "backend": {
                        "type": "string",
                        "description": "Crawl backend. Only 'local' is supported by the server; for JS-rendered crawls use the CLI.",
                        "enum": ["local"],
                        "default": "local"
                    },
                    "pagespeed": {
                        "type": "boolean",
                        "description": "Enable PageSpeed/Core Web Vitals analysis (default: false)",
                        "default": false
                    },
                    "pagespeed_sample": {
                        "type": "integer",
                        "description": "Analyze only N random pages for PageSpeed (default: all)"
                    },
                    "pagespeed_strategy": {
                        "type": "string",
                        "description": "PageSpeed test strategy: 'mobile' (default) or 'desktop'",
                        "enum": ["mobile", "desktop"],
                        "default": "mobile"
                    }
                },
                "required": ["url"]
            }
        }),
        // 2. slither_inspect
        json!({
            "name": "slither_inspect",
            "description": "Run a single-page SEO audit on a URL. Returns page data (title, meta description, headings, word count, links, schema, security) and any issues found. Fast — no crawl needed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to inspect"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Inspection mode: 'static' (default, raw HTML), 'rendered' (JS-rendered via Cloudflare), or 'compare' (both side-by-side)",
                        "enum": ["static", "rendered", "compare"],
                        "default": "static"
                    }
                },
                "required": ["url"]
            }
        }),
        // 3. slither_screenshot
        json!({
            "name": "slither_screenshot",
            "description": "Take a screenshot of a webpage via Cloudflare Browser Rendering. Returns a base64-encoded image. Requires Cloudflare credentials.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to screenshot"
                    },
                    "viewport_width": {
                        "type": "integer",
                        "description": "Viewport width in pixels (default: 1920)",
                        "default": 1920
                    },
                    "viewport_height": {
                        "type": "integer",
                        "description": "Viewport height in pixels (default: 1080)",
                        "default": 1080
                    },
                    "format": {
                        "type": "string",
                        "description": "Image format: 'png' (default) or 'jpeg'",
                        "enum": ["png", "jpeg"],
                        "default": "png"
                    },
                    "full_page": {
                        "type": "boolean",
                        "description": "Capture the full page instead of just the viewport (default: false)",
                        "default": false
                    }
                },
                "required": ["url"]
            }
        }),
        // 4. slither_status
        json!({
            "name": "slither_status",
            "description": "Check the status of a crawl job, list recent jobs, or cancel a running job. If no job_id is provided, lists recent jobs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "Specific job ID to check. Omit to list recent jobs."
                    },
                    "action": {
                        "type": "string",
                        "description": "Action to perform: 'cancel' to cancel a running job",
                        "enum": ["cancel"]
                    },
                    "filter_status": {
                        "type": "string",
                        "description": "Filter jobs by status when listing (queued, running, completed, failed, cancelled)"
                    },
                    "filter_domain": {
                        "type": "string",
                        "description": "Filter jobs by domain when listing"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of jobs to return when listing (default: 20)",
                        "default": 20
                    }
                }
            }
        }),
        // 5. slither_summary
        json!({
            "name": "slither_summary",
            "description": "Get the high-level summary of a completed crawl: health score, grade, issue breakdown, top issue categories, and worst pages. Use after slither_crawl completes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID of a completed crawl"
                    }
                },
                "required": ["job_id"]
            }
        }),
        // 6. slither_query
        json!({
            "name": "slither_query",
            "description": "Query crawl results with powerful filtering. Filter by URL pattern, issue category, severity. Returns paginated issues and optional page data. This is the primary tool for exploring crawl results in detail.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID of a completed crawl"
                    },
                    "urls": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter to specific URLs"
                    },
                    "url_pattern": {
                        "type": "string",
                        "description": "Glob pattern to match URLs (e.g. '*/blog/*', '*.html')"
                    },
                    "category": {
                        "type": "string",
                        "description": "Filter issues by category",
                        "enum": [
                            "response_codes", "security", "url", "page_titles",
                            "meta_description", "headings", "content", "images",
                            "canonicals", "directives", "hreflang", "links",
                            "structured_data", "sitemaps", "performance", "javascript"
                        ]
                    },
                    "severity": {
                        "type": "string",
                        "description": "Filter issues by severity: 'critical', 'warning', or 'info'",
                        "enum": ["critical", "warning", "info"]
                    },
                    "include": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["issues", "page_data"] },
                        "description": "What to include in results (default: ['issues'])"
                    },
                    "fields": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific page_data fields to return (e.g. ['title', 'status', 'word_count']). Only used when page_data is included."
                    },
                    "page": {
                        "type": "integer",
                        "description": "Page number for pagination (default: 1)",
                        "default": 1
                    },
                    "per_page": {
                        "type": "integer",
                        "description": "Results per page (default: 20, max: 100)",
                        "default": 20
                    },
                    "max_urls_per_issue": {
                        "type": "integer",
                        "description": "Cap on affected URLs listed per issue (default: 5, max: 500). A sitewide check affects every page, so the full list can be enormous; each issue always reports affected_url_count with the true total and sets affected_urls_truncated when the list was cut. To see specific URLs, narrow with url_pattern rather than raising this.",
                        "default": 5
                    }
                },
                "required": ["job_id"]
            }
        }),
        // 7. slither_compare
        json!({
            "name": "slither_compare",
            "description": "Compare two crawl jobs to find regressions and improvements. Shows score changes, new issues, and resolved issues between a baseline and current crawl.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "baseline_job_id": {
                        "type": "string",
                        "description": "The job ID of the baseline (earlier) crawl"
                    },
                    "current_job_id": {
                        "type": "string",
                        "description": "The job ID of the current (later) crawl"
                    }
                },
                "required": ["baseline_job_id", "current_job_id"]
            }
        }),
        // 8. slither_link_graph
        json!({
            "name": "slither_link_graph",
            "description": "Analyze the internal link graph of a completed crawl: PageRank (internal authority), orphan pages (no inbound links), navigation hubs, weakly connected components, and silo distribution. Use this to answer 'which pages have the most internal authority?' or 'which pages are orphaned?'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "The job ID of a completed crawl"
                    },
                    "top_n": {
                        "type": "integer",
                        "description": "How many pages to return in the ranked lists (default 15, max 100)",
                        "default": 15
                    }
                },
                "required": ["job_id"]
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// Tool dispatcher
// ---------------------------------------------------------------------------

/// The argument names a tool accepts, read from its own published schema.
///
/// Derived from [`tool_definitions`] rather than repeated, so the accepted set
/// cannot drift from what `tools/list` advertises.
fn schema_arg_names(tool: &str) -> Option<Vec<String>> {
    tool_definitions()
        .into_iter()
        .find(|def| def.get("name").and_then(|n| n.as_str()) == Some(tool))
        .and_then(|def| {
            def.get("inputSchema")
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.as_object())
                .map(|props| props.keys().cloned().collect())
        })
}

pub async fn call_tool(state: &Arc<AppState>, name: &str, args: Value) -> Result<String, String> {
    // An argument outside the published schema is a mistake — usually a
    // misspelling — and answering it with default behavior is how a request for
    // three polite pages turns into a five-hundred-page crawl. Name it instead.
    if let Some(names) = schema_arg_names(name) {
        let allowed: Vec<&str> = names.iter().map(String::as_str).collect();
        crate::api::jobs::reject_unknown_keys("argument", &args, &allowed)?;
    }

    match name {
        "slither_crawl" => tool_crawl(state, args).await,
        "slither_inspect" => tool_inspect(args).await,
        "slither_screenshot" => tool_screenshot(args).await,
        "slither_status" => tool_status(state, args).await,
        "slither_summary" => tool_summary(state, args).await,
        "slither_query" => tool_query(state, args).await,
        "slither_compare" => tool_compare(state, args).await,
        "slither_link_graph" => tool_link_graph(state, args).await,
        _ => Err(format!("Unknown tool: {name}")),
    }
}

// ---------------------------------------------------------------------------
// Tool: slither_link_graph
// ---------------------------------------------------------------------------

async fn tool_link_graph(state: &AppState, args: Value) -> Result<String, String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: job_id")?;
    let top_n = crate::params::u64_or(&args, "top_n", 15)?.clamp(1, 100) as usize;

    let crawl = load_crawl_result(state, job_id)?;
    let report = slither_core::link_graph::compute_link_graph(
        &crawl.pages,
        Some(&crawl.crawl_metadata.seed_url),
        top_n,
    );

    let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("job_id".to_string(), json!(job_id));
        obj.insert("orphan_count".to_string(), json!(report.orphan_pages.len()));
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool 1: slither_crawl
// ---------------------------------------------------------------------------

async fn tool_crawl(state: &Arc<AppState>, args: Value) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: url")?
        .to_string();

    // Read every crawl setting strictly, through the same builder the REST
    // endpoint uses — including the backend check, which the server executor
    // needs because it only runs `local`. `max_pages: 3.0` used to fall through
    // `as_u64` and silently become the 500-page default; `delay_ms: 1000.0`
    // silently became 250 ms, four times the rate the caller asked for against
    // someone else's site.
    let crawl_config = crate::api::jobs::crawl_config_from_json(&url, &args)?;

    // Parse URL to extract domain.
    let parsed_url = url::Url::parse(&url).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err("Only http and https URLs are supported".to_string());
    }
    let domain = parsed_url.host_str().unwrap_or("unknown").to_string();

    // SSRF pre-check so the model gets an immediate, clear error instead of a
    // failed job. Enforcement still happens in the fetcher.
    slither_core::net_guard::check_url_allowed(&url).await?;

    // Persist exactly the settings that will run, so the stored config always
    // agrees with the crawl (and with what the caller is told below).
    let config_json = json!({
        "max_pages": crawl_config.max_pages,
        "concurrency": crawl_config.concurrency,
        "delay_ms": crawl_config.delay_ms,
        "backend": crawl_config.backend,
        "pagespeed": crawl_config.pagespeed,
        "pagespeed_sample": crawl_config.pagespeed_sample,
        "pagespeed_strategy": crawl_config.pagespeed_strategy,
    });

    // Same queue-depth ceiling the REST endpoint enforces, applied inside the
    // insert transaction — without it the MCP path could queue without bound.
    let job = state
        .job_manager
        .create_job_if_under_cap(
            JobType::Crawl,
            &url,
            &domain,
            config_json,
            crate::api::jobs::MAX_QUEUED_JOBS,
        )
        .map_err(|e| format!("Failed to create job: {e}"))?
        .ok_or_else(|| {
            "Too many queued jobs. Wait for existing jobs to complete before starting another."
                .to_string()
        })?;

    crate::executor::fire_webhook(state, &job.id, "job.queued").await;

    let effective_settings = json!({
        "max_pages": crawl_config.max_pages,
        "concurrency": crawl_config.concurrency,
        "delay_ms": crawl_config.delay_ms,
    });

    let state_clone = Arc::clone(state);
    let semaphore = Arc::clone(&state.crawl_semaphore);
    let job_id = job.id.clone();

    tokio::spawn(async move {
        crate::executor::execute_crawl_job(state_clone, job_id, crawl_config, semaphore).await;
    });

    // Echo the settings that will actually be used. A value the server clamped
    // is then visible to the caller instead of being a silent difference
    // between what was asked for and what the target site receives.
    let result = json!({
        "job_id": job.id,
        "status": "queued",
        "domain": domain,
        "settings": effective_settings,
        "message": format!("Crawl queued for {}. Use slither_status with job_id to track progress.", url),
    });

    serde_json::to_string(&result).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool 2: slither_inspect
// ---------------------------------------------------------------------------

async fn tool_inspect(args: Value) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: url")?;

    let mode_str = crate::params::str_or(&args, "mode", "static")?;

    // We always have the inspect module available (it's behind cfg(feature = "cloudflare")
    // in slither-core, but slither-server enables that feature by default).
    use slither_core::inspect::{run_inspect, InspectMode};

    let mode = match mode_str {
        "static" => InspectMode::Static,
        "rendered" => InspectMode::Rendered,
        "compare" => InspectMode::Compare,
        _ => {
            return Err(format!(
                "Invalid mode: {mode_str}. Use 'static', 'rendered', or 'compare'."
            ))
        }
    };

    // Build CF client if needed for rendered/compare modes.
    let cf_client = if mode != InspectMode::Static {
        #[cfg(feature = "cloudflare")]
        {
            use slither_core::cloudflare::CloudflareClient;
            Some(CloudflareClient::new(None, None).ok_or_else(|| {
                "Cloudflare credentials not configured. Set CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN environment variables, or use mode='static'.".to_string()
            })?)
        }
        #[cfg(not(feature = "cloudflare"))]
        {
            return Err("Cloudflare feature not enabled. Use mode='static'.".to_string());
        }
    } else {
        None
    };

    let result = run_inspect(url, mode, None, cf_client.as_ref())
        .await
        .map_err(|e| format!("Inspect failed: {e}"))?;

    // Build a structured JSON response.
    let page = &result.page;
    let mut response = json!({
        "url": result.url,
        "mode": result.mode,
        "title": page.title,
        "meta_description": page.meta_description,
        "h1s": page.h1,
        "word_count": page.word_count,
        "status": page.status,
        "response_time_ms": page.response_time_ms,
        "schema_types": page.schema_types,
        "canonical": page.canonical,
        "issues_found": result.issues.len(),
        "issues": result.issues.iter().map(|i| {
            json!({
                "category": i.category,
                "severity": i.severity,
                "check": i.check,
                "display_name": i.display_name,
                "description": i.description,
                "guidance": i.guidance,
            })
        }).collect::<Vec<_>>(),
    });

    // Add comparison data if available.
    if let Some(ref static_page) = result.static_page {
        response["static_page"] = json!({
            "title": static_page.title,
            "meta_description": static_page.meta_description,
            "h1s": static_page.h1,
            "word_count": static_page.word_count,
            "status": static_page.status,
            "internal_links": static_page.internal_links.len(),
            "external_links": static_page.external_links.len(),
            "images": static_page.images.len(),
            "schema_types": static_page.schema_types,
        });
    }
    if let Some(ref static_issues) = result.static_issues {
        response["static_issues"] = json!(static_issues
            .iter()
            .map(|i| {
                json!({
                    "category": i.category,
                    "severity": i.severity,
                    "check": i.check,
                    "display_name": i.display_name,
                })
            })
            .collect::<Vec<_>>());
    }

    serde_json::to_string(&response).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool 3: slither_screenshot
// ---------------------------------------------------------------------------

async fn tool_screenshot(args: Value) -> Result<String, String> {
    #[cfg(feature = "cloudflare")]
    {
        use slither_core::cloudflare::screenshot::{take_screenshot, ScreenshotConfig};
        use slither_core::cloudflare::CloudflareClient;

        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: url")?
            .to_string();

        let viewport_width =
            crate::params::u64_or(&args, "viewport_width", 1920)?.clamp(1, 10_000) as u32;
        let viewport_height =
            crate::params::u64_or(&args, "viewport_height", 1080)?.clamp(1, 10_000) as u32;
        let format = crate::params::one_of(
            "format",
            crate::params::str_or(&args, "format", "png")?,
            &["png", "jpeg"],
        )?
        .to_string();
        let full_page = crate::params::bool_or(&args, "full_page", false)?;

        let client = CloudflareClient::new(None, None).ok_or_else(|| {
            "Cloudflare credentials not configured. Set CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN environment variables.".to_string()
        })?;

        let config = ScreenshotConfig {
            url: url.clone(),
            full_page,
            format: format.clone(),
            quality: None,
            selector: None,
            viewport_width,
            viewport_height,
        };

        let bytes = take_screenshot(&client, &config)
            .await
            .map_err(|e| format!("Screenshot failed: {e}"))?;

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let mime = if format == "jpeg" {
            "image/jpeg"
        } else {
            "image/png"
        };

        let result = json!({
            "url": url,
            "format": format,
            "viewport": format!("{}x{}", viewport_width, viewport_height),
            "full_page": full_page,
            "size_bytes": bytes.len(),
            "mime_type": mime,
            "data_base64": b64,
        });

        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[cfg(not(feature = "cloudflare"))]
    {
        let _ = args;
        Err(
            "Screenshot requires the cloudflare feature. Rebuild with --features cloudflare."
                .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// Tool 4: slither_status
// ---------------------------------------------------------------------------

async fn tool_status(state: &Arc<AppState>, args: Value) -> Result<String, String> {
    let job_id = args.get("job_id").and_then(|v| v.as_str());
    let action = args.get("action").and_then(|v| v.as_str());

    if let Some(id) = job_id {
        // Single job mode.
        let job = state
            .job_manager
            .get_job(id)
            .map_err(|e| format!("Failed to get job: {e}"))?
            .ok_or_else(|| format!("Job not found: {id}"))?;

        // Handle cancel action.
        if action == Some("cancel") {
            if job.status == JobStatus::Running || job.status == JobStatus::Queued {
                // The store enforces the terminal-state invariant, so a false
                // here means the job finished between the read above and this
                // write. Report that rather than claiming a cancel that the
                // store refused.
                let cancelled = state
                    .job_manager
                    .update_status(id, JobStatus::Cancelled)
                    .map_err(|e| format!("Failed to cancel job: {e}"))?;

                if !cancelled {
                    let current = state
                        .job_manager
                        .get_job(id)
                        .map_err(|e| format!("Failed to get job: {e}"))?
                        .map(|j| j.status.as_str().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    return Err(format!(
                        "Job reached status '{current}' before it could be cancelled."
                    ));
                }

                // Announce it: `job.cancelled` is an advertised event that
                // nothing ever emitted, so a caller waiting on a webhook for
                // this job used to hear nothing until the crawl wound down and
                // then received `job.completed` describing a cancelled job.
                crate::executor::fire_webhook(state, id, "job.cancelled").await;

                let result = json!({
                    "job_id": id,
                    "status": "cancelled",
                    "message": "Job cancelled successfully. A queued job will not start; \
                                a running crawl finishes its in-flight work.",
                });
                return serde_json::to_string(&result).map_err(|e| e.to_string());
            } else {
                return Err(format!(
                    "Cannot cancel job with status '{}'. Only queued or running jobs can be cancelled.",
                    job.status.as_str()
                ));
            }
        }

        let result = json!({
            "job_id": job.id,
            "type": job.job_type,
            "status": job.status,
            "domain": job.domain,
            "url": job.url,
            "progress": job.progress,
            "result_summary": job.result_summary,
            "error": job.error,
            "created_at": job.created_at,
            "started_at": job.started_at,
            "completed_at": job.completed_at,
        });

        serde_json::to_string(&result).map_err(|e| e.to_string())
    } else {
        // List mode.
        let filter = ListJobsFilter {
            status: args
                .get("filter_status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            job_type: None,
            domain: args
                .get("filter_domain")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            limit: crate::params::u64_or(&args, "limit", 20)?.clamp(1, 200) as u32,
            offset: 0,
        };

        let jobs = state
            .job_manager
            .list_jobs(&filter)
            .map_err(|e| format!("Failed to list jobs: {e}"))?;

        let jobs_json: Vec<Value> = jobs
            .iter()
            .map(|j| {
                json!({
                    "job_id": j.id,
                    "type": j.job_type,
                    "status": j.status,
                    "domain": j.domain,
                    "url": j.url,
                    "created_at": j.created_at,
                    "completed_at": j.completed_at,
                })
            })
            .collect();

        let result = json!({
            "jobs": jobs_json,
            "count": jobs_json.len(),
        });

        serde_json::to_string(&result).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tool 5: slither_summary
// ---------------------------------------------------------------------------

async fn tool_summary(state: &AppState, args: Value) -> Result<String, String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: job_id")?;

    let crawl = load_crawl_result(state, job_id)?;

    let summary = &crawl.summary;

    // Top 10 issue categories by count.
    let mut categories: Vec<(
        &String,
        &slither_core::models::crawl_result::CategorySummary,
    )> = summary.issues_by_category.iter().collect();
    categories.sort_by_key(|c| std::cmp::Reverse(c.1.issues_found));
    let top_categories: Vec<Value> = categories
        .iter()
        .take(10)
        .map(|(name, cat)| {
            json!({
                "category": name,
                "issues_found": cat.issues_found,
                // `affected_url_count`, not `affected_urls`: `slither_query`
                // uses that name for the actual list of {url, detail} objects,
                // and the same key holding a bare number here read as an empty
                // list to a caller that had seen the other tool. Reported from
                // the field — the agent concluded the URLs were missing and
                // went looking for them in the per-page data instead.
                "affected_url_count": cat.affected_urls,
                "critical": cat.critical,
                "warning": cat.warning,
                "info": cat.info,
            })
        })
        .collect();

    // Worst pages (top 10 by issue count).
    //
    // Issue URLs are resolved to the page that served them before counting: a
    // redirect issue is filed against the URL that redirects, which has no page
    // row, so counting raw issue URLs both listed addresses that are not pages
    // and split one page's issues across its aliases.
    let aliases = slither_core::analysis::UrlAliases::build(&crawl.pages);
    let mut page_issue_counts: HashMap<String, u32> = HashMap::new();
    for issue in &crawl.issues.issues {
        for iu in &issue.urls {
            *page_issue_counts
                .entry(aliases.resolve(&iu.url))
                .or_insert(0) += 1;
        }
    }
    let mut worst_pages: Vec<(String, u32)> = page_issue_counts.into_iter().collect();
    worst_pages.sort_by_key(|p| std::cmp::Reverse(p.1));
    let worst_pages_json: Vec<Value> = worst_pages
        .iter()
        .take(10)
        .map(|(url, count)| json!({ "url": url, "issue_count": count }))
        .collect();

    let result = json!({
        "job_id": job_id,
        "health_score": summary.health_score,
        "grade": summary.grade,
        "grade_verdict": summary.grade_verdict,
        "pages_crawled": crawl.crawl_metadata.pages_crawled,
        "issue_breakdown": {
            "total": summary.total_issues,
            "critical": summary.critical_issues,
            "warning": summary.warning_issues,
            "info": summary.info_issues,
        },
        "top_categories": top_categories,
        "worst_pages": worst_pages_json,
        "avg_response_time_ms": summary.avg_response_time_ms,
        "response_time_p50_ms": summary.response_time_p50_ms,
        "response_time_p95_ms": summary.response_time_p95_ms,
    });

    serde_json::to_string(&result).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool 6: slither_query
// ---------------------------------------------------------------------------

/// Number of pages needed to hold `total` items at `per_page` each.
///
/// Floored at 1 so an empty result set reports one (empty) page rather than
/// zero. The previous form applied `.max(1)` to the numerator instead of the
/// quotient, which reported 0 pages for an empty set.
fn page_count(total: usize, per_page: usize) -> usize {
    total.div_ceil(per_page.max(1)).max(1)
}

async fn tool_query(state: &AppState, args: Value) -> Result<String, String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: job_id")?;

    let crawl = load_crawl_result(state, job_id)?;

    // URL filters.
    let url_list: Option<Vec<&str>> = args.get("urls").and_then(|v| {
        v.as_array()
            .map(|arr| arr.iter().filter_map(|u| u.as_str()).collect())
    });
    let url_pattern = args.get("url_pattern").and_then(|v| v.as_str());

    // Issue filters.
    let category = args.get("category").and_then(|v| v.as_str());
    let severity = args.get("severity").and_then(|v| v.as_str());

    // Include.
    let include: HashSet<String> = args
        .get("include")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            let mut set = HashSet::new();
            set.insert("issues".to_string());
            set
        });

    let fields: Option<Vec<&str>> = args.get("fields").and_then(|v| {
        v.as_array()
            .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
    });

    // Pagination.
    let page_num = crate::params::u64_or(&args, "page", 1)?.max(1) as usize;
    let per_page = crate::params::u64_or(&args, "per_page", 20)?.clamp(1, 100) as usize;

    // Deliberately small: an agent triaging a site needs the true count plus a
    // few representative URLs, then narrows with `url_pattern`. Listing every
    // affected URL by default is what made a single call cost six figures of
    // tokens on a large crawl.
    let max_urls_per_issue =
        crate::params::u64_or(&args, "max_urls_per_issue", 5)?.clamp(1, 500) as usize;

    // Build the glob regex pattern if provided.
    let pattern_regex = url_pattern.map(glob_to_pattern);
    let has_url_filter = url_list.is_some() || url_pattern.is_some();

    // Does this URL match the caller's filter on its own text?
    let direct_match = |url: &str| -> bool {
        if let Some(ref list) = url_list {
            if list.contains(&url) {
                return true;
            }
        }
        if let Some(ref pat) = pattern_regex {
            if glob_matches(pat, url) {
                return true;
            }
        }
        false
    };

    // Pages selected by the filter, keyed by the URL that served them.
    //
    // Issue URLs cannot be matched against `crawl.pages` directly: an issue
    // about a redirect is filed against the URL that *redirects*, and that URL
    // has no page row — only its destination does. Testing membership in the
    // page set therefore dropped every redirect issue silently, from filtered
    // and unfiltered queries alike, which is the worst possible answer for an
    // agent: "no redirect problems" rather than "here they are". Resolving both
    // sides through the crawl's alias map is the shared fix.
    let aliases = slither_core::analysis::UrlAliases::build(&crawl.pages);
    let page_matches = |p: &slither_core::PageData| !has_url_filter || direct_match(&p.url);
    let matching_pages: HashSet<String> = crawl
        .pages
        .iter()
        .filter(|p| page_matches(p))
        .map(|p| aliases.resolve(&p.url))
        .collect();

    // An issue URL is in scope when it matches the filter itself, or when the
    // page that answered it does.
    let issue_url_matches = |url: &str| -> bool {
        !has_url_filter || direct_match(url) || matching_pages.contains(&aliases.resolve(url))
    };

    // Filter issues.
    let mut filtered_issues: Vec<Value> = Vec::new();
    for issue in &crawl.issues.issues {
        // Category filter.
        if let Some(cat) = category {
            let issue_cat = serde_json::to_value(&issue.category)
                .ok()
                .and_then(|v| v.as_str().map(String::from));
            if let Some(ref ic) = issue_cat {
                if ic != cat {
                    continue;
                }
            }
        }

        // Severity filter.
        if let Some(sev) = severity {
            let issue_sev = serde_json::to_value(issue.severity)
                .ok()
                .and_then(|v| v.as_str().map(String::from));
            if let Some(ref is) = issue_sev {
                if is != sev {
                    continue;
                }
            }
        }

        // URL filter — keep only issue URLs that match.
        let matched: Vec<_> = issue
            .urls
            .iter()
            .filter(|iu| issue_url_matches(&iu.url))
            .collect();

        // `per_page` bounds how many *issues* come back, but a sitewide check
        // affects every crawled page, so an uncapped list here grows with the
        // crawl: a 500-page crawl could emit hundreds of thousands of tokens
        // from a single default call. Cap the list and report the true total so
        // the caller can narrow with `url_pattern` instead of guessing.
        let matched_total = matched.len();
        let matching_issue_urls: Vec<Value> = matched
            .into_iter()
            .take(max_urls_per_issue)
            .map(|iu| {
                json!({
                    "url": iu.url,
                    "detail": iu.detail,
                })
            })
            .collect();

        if matched_total > 0 || issue.urls.is_empty() {
            filtered_issues.push(json!({
                "category": issue.category,
                "check": issue.check,
                "display_name": issue.display_name,
                "severity": issue.severity,
                "description": issue.description,
                "guidance": issue.guidance,
                "affected_url_count": matched_total,
                "affected_urls_truncated": matched_total > matching_issue_urls.len(),
                "affected_urls": if issue.urls.is_empty() {
                    json!([])
                } else {
                    json!(matching_issue_urls)
                },
            }));
        }
    }

    // Pagination. Issues and page_data are counted separately: a filter can
    // match many pages but no issues (or vice versa), and paginating pages
    // against the *issue* count previously reported `total_pages: 0` while
    // hundreds of pages existed, leaving them unreachable.
    let total_issues = filtered_issues.len();
    let total_matching_pages = crawl.pages.iter().filter(|p| page_matches(p)).count();

    let issue_pages = page_count(total_issues, per_page);
    let page_data_pages = page_count(total_matching_pages, per_page);

    let start = (page_num - 1) * per_page;
    let paginated_issues: Vec<Value> = filtered_issues
        .into_iter()
        .skip(start)
        .take(per_page)
        .collect();

    let mut result = json!({
        "job_id": job_id,
        "total_issues": total_issues,
        "total_matching_pages": total_matching_pages,
        "page": page_num,
        "per_page": per_page,
        // Retained for compatibility: pages of *issues*.
        "total_pages": issue_pages,
        "issue_pages": issue_pages,
        "page_data_pages": page_data_pages,
    });

    if include.contains("issues") {
        result["issues"] = json!(paginated_issues);
    }

    // Include page_data if requested.
    if include.contains("page_data") {
        let pages_json: Vec<Value> = crawl
            .pages
            .iter()
            .filter(|p| page_matches(p))
            .skip(start)
            .take(per_page)
            .map(|p| {
                if let Some(ref field_list) = fields {
                    // Return only requested fields.
                    let full = serde_json::to_value(p).unwrap_or(json!({}));
                    let mut filtered = json!({ "url": p.url });
                    for field in field_list {
                        if let Some(val) = full.get(*field) {
                            filtered[*field] = val.clone();
                        }
                    }
                    filtered
                } else {
                    // Return a useful default subset (not the full PageData which is huge).
                    json!({
                        "url": p.url,
                        "status": p.status,
                        "title": p.title,
                        "meta_description": p.meta_description,
                        "canonical": p.canonical,
                        "word_count": p.word_count,
                        "response_time_ms": p.response_time_ms,
                        "h1": p.h1,
                        "internal_links_count": p.internal_links.len(),
                        "external_links_count": p.external_links.len(),
                        "images_count": p.images.len(),
                        "schema_types": p.schema_types,
                    })
                }
            })
            .collect();
        result["page_data"] = json!(pages_json);
    }

    serde_json::to_string(&result).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool 7: slither_compare
// ---------------------------------------------------------------------------

async fn tool_compare(state: &AppState, args: Value) -> Result<String, String> {
    let baseline_id = args
        .get("baseline_job_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: baseline_job_id")?;
    let current_id = args
        .get("current_job_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: current_job_id")?;

    let baseline = load_crawl_result(state, baseline_id)?;
    let current = load_crawl_result(state, current_id)?;

    // Build sets of (check, url) pairs for comparison.
    //
    // The URL is resolved through each crawl's own alias map, so both sides key
    // on the page that served the issue rather than on whichever address it
    // happened to be filed under. Keying on the raw URL made any change in
    // issue attribution — a redirect issue moving from the destination to the
    // URL that redirects — read as one resolved plus one new issue for every
    // affected URL, a spurious regression report in the exact feature people
    // trust for "what changed since last week".
    let baseline_aliases = slither_core::analysis::UrlAliases::build(&baseline.pages);
    let current_aliases = slither_core::analysis::UrlAliases::build(&current.pages);

    let pairs = |crawl: &CrawlResult,
                 aliases: &slither_core::analysis::UrlAliases|
     -> HashSet<(String, String)> {
        crawl
            .issues
            .issues
            .iter()
            .flat_map(|issue| {
                issue
                    .urls
                    .iter()
                    .map(move |iu| (issue.check.clone(), aliases.resolve(&iu.url)))
            })
            .collect()
    };

    let baseline_pairs = pairs(&baseline, &baseline_aliases);
    let current_pairs = pairs(&current, &current_aliases);

    // New issues = in current but not in baseline.
    let new_pairs: Vec<&(String, String)> = current_pairs.difference(&baseline_pairs).collect();
    // Resolved issues = in baseline but not in current.
    let resolved_pairs: Vec<&(String, String)> =
        baseline_pairs.difference(&current_pairs).collect();

    // Group regressions by URL.
    let mut regressions: HashMap<&str, Vec<&str>> = HashMap::new();
    for (check, url) in &new_pairs {
        regressions
            .entry(url.as_str())
            .or_default()
            .push(check.as_str());
    }
    // A sitewide regression puts every crawled URL in this map, so the list is
    // bounded for the same reason slither_query's affected_urls is: one call
    // must not be able to exhaust the caller's context.
    const MAX_REGRESSION_URLS: usize = 50;
    let regressions_total = regressions.len();
    let mut regression_entries: Vec<(&str, Vec<&str>)> = regressions.into_iter().collect();
    // Most-regressed URLs first, so a truncated list is still the useful half.
    regression_entries.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let regressions_json: Vec<Value> = regression_entries
        .into_iter()
        .take(MAX_REGRESSION_URLS)
        .map(|(url, checks)| json!({ "url": url, "new_checks": checks }))
        .collect();

    // Group improvements by category (look up each resolved check's category).
    let mut improvements: HashMap<String, u32> = HashMap::new();
    let baseline_check_categories: HashMap<&str, String> = baseline
        .issues
        .issues
        .iter()
        .map(|i| {
            (
                i.check.as_str(),
                serde_json::to_value(&i.category)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "unknown".to_string()),
            )
        })
        .collect();
    for (check, _url) in &resolved_pairs {
        let cat = baseline_check_categories
            .get(check.as_str())
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        *improvements.entry(cat).or_insert(0) += 1;
    }
    let improvements_json: Vec<Value> = improvements
        .iter()
        .map(|(cat, count)| json!({ "category": cat, "resolved_count": count }))
        .collect();

    let score_from = baseline.summary.health_score;
    let score_to = current.summary.health_score;
    let score_delta = score_to as i64 - score_from as i64;

    let result = json!({
        "baseline_job_id": baseline_id,
        "current_job_id": current_id,
        "score_change": {
            "from": score_from,
            "to": score_to,
            "delta": score_delta,
        },
        "grade_change": {
            "from": baseline.summary.grade,
            "to": current.summary.grade,
        },
        "new_issues": new_pairs.len(),
        "resolved_issues": resolved_pairs.len(),
        "regressed_url_count": regressions_total,
        "regressions_truncated": regressions_total > regressions_json.len(),
        "regressions": regressions_json,
        "improvements": improvements_json,
    });

    serde_json::to_string(&result).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Load and deserialize the crawl.json for a given job.
fn load_crawl_result(state: &AppState, job_id: &str) -> Result<CrawlResult, String> {
    let job = state
        .job_manager
        .get_job(job_id)
        .map_err(|e| format!("Failed to get job: {e}"))?
        .ok_or_else(|| format!("Job not found: {job_id}"))?;

    if job.status != JobStatus::Completed {
        return Err(format!(
            "Job is not completed (status: {}). Wait for the crawl to finish.",
            job.status.as_str()
        ));
    }

    let output_dir = job
        .output_dir
        .as_ref()
        .ok_or("Job has no output directory")?;

    let crawl_path = std::path::PathBuf::from(output_dir).join("crawl.json");
    let data = std::fs::read_to_string(&crawl_path)
        .map_err(|e| format!("Failed to read crawl.json at {}: {e}", crawl_path.display()))?;

    serde_json::from_str::<CrawlResult>(&data)
        .map_err(|e| format!("Failed to parse crawl.json: {e}"))
}

// ---------------------------------------------------------------------------
// MCP resources — crawl artifacts as slither://job/{id}/{filename}
// ---------------------------------------------------------------------------

/// Known artifact filenames and their MIME types.
const ARTIFACTS: &[(&str, &str)] = &[
    ("crawl.json", "application/json"),
    ("report.html", "text/html"),
    ("export.csv", "text/csv"),
];

/// Bytes returned by a single `resources/read` unless the caller asks for more.
///
/// ~25k tokens. A crawl artifact is raw data, not a bounded view: the whole
/// `crawl.json` of a 61-page crawl is already ~917 KB (~226k tokens), a
/// 500-page default crawl is ~6.9 MB, and the 10,000-page clamp is ~137 MB —
/// past roughly 730 pages the server would emit a message its own 10 MB
/// transport limit refuses to carry. So reads are windowed and say so.
const RESOURCE_READ_DEFAULT_BYTES: usize = 100_000;

/// Ceiling on an explicitly requested window, well under the transport's 10 MB.
const RESOURCE_READ_MAX_BYTES: usize = 1_000_000;

/// List crawl artifacts for completed jobs as MCP resources.
pub fn list_resources(state: &AppState) -> Vec<Value> {
    let filter = ListJobsFilter {
        status: Some("completed".to_string()),
        limit: 50,
        ..Default::default()
    };
    let jobs = match state.job_manager.list_jobs(&filter) {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };

    let mut resources = Vec::new();
    for job in jobs {
        let Some(dir) = job.output_dir.as_ref() else {
            continue;
        };
        for (name, mime) in ARTIFACTS {
            let path = std::path::Path::new(dir).join(name);
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let size = meta.len();
            resources.push(json!({
                "uri": format!("slither://job/{}/{}", job.id, name),
                "name": format!("{} — {}", job.domain, name),
                "description": format!(
                    "{name} for crawl {} of {} ({size} bytes). Reads are windowed to \
                     {RESOURCE_READ_DEFAULT_BYTES} bytes; append ?offset=&limit= to page. \
                     For analysis prefer slither_summary / slither_query / slither_link_graph, \
                     which return bounded, filtered views of this data.",
                    job.id, job.domain
                ),
                "mimeType": mime,
                // Advertised by the spec so a caller can see the cost before reading.
                "size": size,
            }));
        }
    }
    resources
}

/// Read a `slither://job/{id}/{filename}` resource. Validates the filename
/// against the known artifact allowlist (no path traversal possible).
///
/// The response is **windowed**, for the same reason `slither_query`,
/// `slither_link_graph` and `slither_compare` bound theirs: this is an MCP server
/// for token-constrained agents, and an artifact grows with the crawl. Reading
/// one used to emit the entire file as a single unbounded line — 917 KB
/// (~226k tokens) for a 61-page crawl, ~6.9 MB at the default 500 pages, and
/// ~137 MB at the 10,000-page clamp, which the 10 MB transport limit would
/// refuse to carry at all.
///
/// A window is `?offset=<bytes>&limit=<bytes>` on the URI, since `resources/read`
/// carries no parameters of its own. A truncated read says exactly how much was
/// left out and hands back the URI for the next chunk.
pub fn read_resource(state: &AppState, uri: &str) -> Result<Value, String> {
    let rest = uri
        .strip_prefix("slither://job/")
        .ok_or_else(|| format!("Unsupported resource URI: {uri}"))?;
    let (locator, query) = match rest.split_once('?') {
        Some((locator, query)) => (locator, Some(query)),
        None => (rest, None),
    };
    let (job_id, filename) = locator
        .split_once('/')
        .ok_or_else(|| format!("Malformed resource URI: {uri}"))?;

    let mime = ARTIFACTS
        .iter()
        .find(|(name, _)| *name == filename)
        .map(|(_, m)| *m)
        .ok_or_else(|| format!("Unknown artifact: {filename}"))?;

    let (offset, limit) = parse_read_window(query)?;

    let job = state
        .job_manager
        .get_job(job_id)
        .map_err(|e| format!("Failed to get job: {e}"))?
        .ok_or_else(|| format!("Job not found: {job_id}"))?;
    let dir = job.output_dir.ok_or("Job has no output directory")?;

    // Filename is from the fixed allowlist above, so this join is safe.
    let path = std::path::Path::new(&dir).join(filename);
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let total = bytes.len();

    if offset > total {
        return Err(format!(
            "offset {offset} is past the end of {filename} ({total} bytes)."
        ));
    }

    // Snap both ends to UTF-8 boundaries so a window never splits a character
    // and every offset handed back is a valid place to resume from.
    let start = floor_char_boundary(&bytes, offset);
    let end = floor_char_boundary(&bytes, total.min(start.saturating_add(limit)));
    let mut text = String::from_utf8_lossy(&bytes[start..end]).into_owned();
    let truncated = end < total;

    if truncated {
        let next_uri = format!("slither://job/{job_id}/{filename}?offset={end}&limit={limit}");
        text.push_str(&format!(
            "\n\n[TRUNCATED: bytes {start}–{end} of {total}. \
             This artifact is raw crawl data and does not fit in one response. \
             Read the next chunk with uri \"{next_uri}\" (raise the window with \
             &limit=<bytes>, max {RESOURCE_READ_MAX_BYTES}), or — usually better — \
             use slither_summary, slither_query (filter with url_pattern/category/severity) \
             or slither_link_graph, which return bounded views of this same crawl.]"
        ));
        Ok(json!({
            "uri": uri,
            "mimeType": mime,
            "text": text,
            "totalBytes": total,
            "byteOffset": start,
            "byteLength": end - start,
            "truncated": true,
            "nextUri": next_uri,
        }))
    } else {
        Ok(json!({
            "uri": uri,
            "mimeType": mime,
            "text": text,
            "totalBytes": total,
            "byteOffset": start,
            "byteLength": end - start,
            "truncated": false,
        }))
    }
}

/// Parse `offset` / `limit` out of a resource URI query string.
fn parse_read_window(query: Option<&str>) -> Result<(usize, usize), String> {
    let mut offset = 0usize;
    let mut limit = RESOURCE_READ_DEFAULT_BYTES;

    for pair in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("Malformed resource URI parameter: '{pair}'."))?;
        let parsed: usize = value.parse().map_err(|_| {
            format!(
                "Resource URI parameter '{key}' must be a whole number of bytes, got '{value}'."
            )
        })?;
        match key {
            "offset" => offset = parsed,
            "limit" => {
                if parsed == 0 {
                    return Err("Resource URI parameter 'limit' must be at least 1.".to_string());
                }
                limit = parsed.min(RESOURCE_READ_MAX_BYTES);
            }
            other => {
                return Err(format!(
                    "Unknown resource URI parameter '{other}'. Supported: offset, limit."
                ))
            }
        }
    }

    Ok((offset, limit))
}

/// Largest index `<= i` that starts a UTF-8 character.
fn floor_char_boundary(bytes: &[u8], i: usize) -> usize {
    let mut i = i.min(bytes.len());
    // The end of the slice is always a boundary.
    if i == bytes.len() {
        return i;
    }
    // Continuation bytes are 0b10xxxxxx; step back off them.
    while i > 0 && (bytes[i] & 0b1100_0000) == 0b1000_0000 {
        i -= 1;
    }
    i
}

/// Convert a simple glob pattern to a tuple of (prefix, suffix, contains)
/// for matching. Supports `*` as a wildcard for any characters.
fn glob_to_pattern(glob: &str) -> Vec<String> {
    glob.split('*').map(|s| s.to_string()).collect()
}

/// Check if a URL matches a glob pattern (split into parts by `*`).
fn glob_matches(parts: &[String], url: &str) -> bool {
    if parts.is_empty() {
        return true;
    }

    let mut remaining = url;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if i == 0 {
            // First segment must be a prefix match.
            if !remaining.starts_with(part.as_str()) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if i == parts.len() - 1 {
            // Last segment must be a suffix match.
            if !remaining.ends_with(part.as_str()) {
                return false;
            }
            return true;
        } else {
            // Middle segments — find them anywhere in the remaining string.
            match remaining.find(part.as_str()) {
                Some(pos) => remaining = &remaining[pos + part.len()..],
                None => return false,
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::page_count;

    /// Regression: an empty result set reported 0 pages because `.max(1)` was
    /// applied to the numerator rather than the quotient.
    #[test]
    fn empty_result_reports_one_page() {
        assert_eq!(page_count(0, 20), 1);
    }

    #[test]
    fn partial_pages_round_up() {
        assert_eq!(page_count(1, 20), 1);
        assert_eq!(page_count(20, 20), 1);
        assert_eq!(page_count(21, 20), 2);
        assert_eq!(page_count(500, 20), 25);
    }

    /// Issues and page_data are counted independently: a filter matching no
    /// issues must not hide the pages it does match.
    #[test]
    fn page_data_pages_are_independent_of_issue_pages() {
        let issues = 0;
        let pages = 500;
        assert_eq!(page_count(issues, 20), 1);
        assert_eq!(page_count(pages, 20), 25);
    }

    #[test]
    fn a_zero_per_page_cannot_divide_by_zero() {
        assert_eq!(page_count(10, 0), 10);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod resource_and_identity_tests {
    use super::*;
    use slither_core::jobs::{db, JobManager, JobStatus};

    fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = db::open_db(&tmp.path().join("slither.db")).unwrap();
        let jobs_dir = tmp.path().join("jobs");
        std::fs::create_dir_all(&jobs_dir).unwrap();
        let state = Arc::new(AppState {
            job_manager: JobManager::new(conn, jobs_dir),
            crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        (state, tmp)
    }

    fn page(url: &str, redirect_chain: Value) -> Value {
        json!({
            "url": url, "status": 200, "redirect_chain": redirect_chain,
            "response_time_ms": 42, "content_type": "text/html; charset=utf-8",
            "depth": 1, "title": "A representative page title",
            "meta_description": "A representative meta description for the page.",
            "meta_robots": null, "canonical": url, "h1": ["Heading"],
            "headings": [], "word_count": 500, "body_text": "",
            "internal_links": [], "external_links": [], "images": [],
            "schema_types": [], "og_tags": {}, "content_hash": "abc123"
        })
    }

    fn issue(check: &str, urls: &[&str]) -> Value {
        json!({
            "category": "response_codes",
            "check": check,
            "display_name": check,
            "severity": "info",
            "description": "d",
            "guidance": "g",
            "urls": urls.iter().map(|u| json!({ "url": u, "detail": "x" })).collect::<Vec<_>>(),
        })
    }

    /// Write a completed job whose crawl.json holds exactly `pages`/`issues`.
    fn completed_job(state: &Arc<AppState>, pages: Vec<Value>, issues: Vec<Value>) -> String {
        let job = state
            .job_manager
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                json!({}),
            )
            .unwrap();
        let n = pages.len();
        let crawl = json!({
            "slither_version": "0.3.0",
            "crawl_metadata": {
                "domain": "example.com", "seed_url": "https://example.com",
                "crawl_date": "2026-08-15T00:00:00Z", "duration_ms": 1000,
                "pages_discovered": n, "pages_crawled": n,
                "pages_skipped_robots": 0, "pages_errored": 0,
                "settings": slither_core::CrawlConfig::default(), "backend": "local"
            },
            "export_settings": { "include_body_text": false, "summary_only": false, "format": "pretty" },
            "pages": pages,
            "issues": { "issues": issues },
            "summary": {
                "total_pages": n, "by_status": { "200": n },
                "avg_response_time_ms": 42, "avg_word_count": 500,
                "total_internal_links": 0, "total_external_links": 0, "total_images": 0,
                "images_without_alt": 0, "pages_with_schema": 0,
                "total_issues": 1, "critical_issues": 0, "warning_issues": 0, "info_issues": 1,
                "issues_by_category": {}, "health_score": 80, "grade": "B",
                "grade_verdict": "Good", "response_time_p50_ms": 42,
                "response_time_p95_ms": 42, "cwv_pages_tested": 0, "cwv_pages_good": 0,
                "cwv_pages_needs_work": 0, "cwv_pages_poor": 0
            }
        });
        let dir = job.output_dir.clone().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join("crawl.json"),
            serde_json::to_string(&crawl).unwrap(),
        )
        .unwrap();
        state
            .job_manager
            .update_status(&job.id, JobStatus::Completed)
            .unwrap();
        job.id
    }

    /// A crawl where /blog/old 301s to /blog/new, and the redirect issue is
    /// filed — as the analyzers now file it — against the URL that redirects,
    /// which has no page row of its own.
    fn redirect_crawl(state: &Arc<AppState>) -> String {
        completed_job(
            state,
            vec![
                page("https://example.com/", json!(null)),
                page("https://example.com/other", json!(null)),
                page(
                    "https://example.com/blog/new",
                    json!([{ "status": 301, "url": "https://example.com/blog/old" }]),
                ),
            ],
            vec![
                issue("internal_redirect", &["https://example.com/blog/old"]),
                issue("some_page_issue", &["https://example.com/other"]),
            ],
        )
    }

    async fn query(state: &Arc<AppState>, args: Value) -> Value {
        let text = call_tool(state, "slither_query", args).await.unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn checks_in(result: &Value) -> Vec<String> {
        result["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["check"].as_str().unwrap().to_string())
            .collect()
    }

    // -- redirect identity ------------------------------------------------

    /// An issue filed against a URL with no page row must still be returned.
    /// Matching issue URLs against `crawl.pages` dropped every redirect issue —
    /// silently, and even with no filter at all, so the agent was told a site
    /// had no redirect problems.
    #[tokio::test]
    async fn a_redirect_issue_survives_an_unfiltered_query() {
        let (state, _tmp) = test_state();
        let job = redirect_crawl(&state);
        let result = query(&state, json!({ "job_id": job })).await;
        assert!(
            checks_in(&result).contains(&"internal_redirect".to_string()),
            "unfiltered query lost the redirect issue: {result}"
        );
    }

    /// A filter naming the destination must reach the issue filed against the
    /// URL that redirects to it.
    #[tokio::test]
    async fn a_url_filter_reaches_issues_through_the_redirect() {
        let (state, _tmp) = test_state();
        let job = redirect_crawl(&state);

        let by_pattern = query(&state, json!({ "job_id": job, "url_pattern": "*/blog/*" })).await;
        assert!(
            checks_in(&by_pattern).contains(&"internal_redirect".to_string()),
            "url_pattern lost the redirect issue: {by_pattern}"
        );

        // And filing address itself still matches directly.
        let by_url = query(
            &state,
            json!({ "job_id": job, "urls": ["https://example.com/blog/old"] }),
        )
        .await;
        assert!(checks_in(&by_url).contains(&"internal_redirect".to_string()));
    }

    /// Resolving must not turn the filter into a sieve that matches everything.
    #[tokio::test]
    async fn a_url_filter_still_excludes_unrelated_issues() {
        let (state, _tmp) = test_state();
        let job = redirect_crawl(&state);
        let result = query(&state, json!({ "job_id": job, "url_pattern": "*/other" })).await;
        let checks = checks_in(&result);
        assert!(checks.contains(&"some_page_issue".to_string()));
        assert!(
            !checks.contains(&"internal_redirect".to_string()),
            "the redirect issue belongs to /blog/, not /other: {result}"
        );
    }

    /// "Worst pages" must name pages, not the addresses that redirect to them.
    #[tokio::test]
    async fn worst_pages_are_attributed_to_the_page_that_served_them() {
        let (state, _tmp) = test_state();
        let job = redirect_crawl(&state);
        let text = call_tool(&state, "slither_summary", json!({ "job_id": job }))
            .await
            .unwrap();
        let summary: Value = serde_json::from_str(&text).unwrap();
        let urls: Vec<&str> = summary["worst_pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["url"].as_str().unwrap())
            .collect();
        assert!(
            !urls.contains(&"https://example.com/blog/old"),
            "worst_pages listed a URL with no page record: {urls:?}"
        );
        assert!(urls.contains(&"https://example.com/blog/new"));
    }

    /// The regression that would otherwise hit every user once: a baseline
    /// crawled before redirect issues were re-attributed, diffed against one
    /// crawled after, must not report the same issue as resolved *and* new.
    #[tokio::test]
    async fn a_diff_across_the_attribution_change_reports_nothing() {
        let (state, _tmp) = test_state();

        // Baseline: the pre-change shape — the issue filed on the destination.
        let baseline = completed_job(
            &state,
            vec![page(
                "https://example.com/blog/new",
                json!([{ "status": 301, "url": "https://example.com/blog/old" }]),
            )],
            vec![issue(
                "internal_redirect",
                &["https://example.com/blog/new"],
            )],
        );
        // Current: the post-change shape — filed on the URL that redirects.
        let current = completed_job(
            &state,
            vec![page(
                "https://example.com/blog/new",
                json!([{ "status": 301, "url": "https://example.com/blog/old" }]),
            )],
            vec![issue(
                "internal_redirect",
                &["https://example.com/blog/old"],
            )],
        );

        let text = call_tool(
            &state,
            "slither_compare",
            json!({ "baseline_job_id": baseline, "current_job_id": current }),
        )
        .await
        .unwrap();
        let diff: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(diff["new_issues"], 0, "spurious regression: {diff}");
        assert_eq!(diff["resolved_issues"], 0, "spurious fix: {diff}");
    }

    /// A genuine regression must still be reported.
    #[tokio::test]
    async fn a_real_new_issue_is_still_reported() {
        let (state, _tmp) = test_state();
        let baseline = completed_job(
            &state,
            vec![page("https://example.com/a", json!(null))],
            vec![],
        );
        let current = completed_job(
            &state,
            vec![page("https://example.com/a", json!(null))],
            vec![issue("broken_link", &["https://example.com/a"])],
        );
        let text = call_tool(
            &state,
            "slither_compare",
            json!({ "baseline_job_id": baseline, "current_job_id": current }),
        )
        .await
        .unwrap();
        let diff: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(diff["new_issues"], 1);
        assert_eq!(diff["resolved_issues"], 0);
    }

    // -- unknown arguments -------------------------------------------------

    /// A misspelled argument is a mistake, not a request for default behavior.
    #[tokio::test]
    async fn an_unknown_tool_argument_is_refused() {
        let (state, _tmp) = test_state();
        let err = call_tool(
            &state,
            "slither_crawl",
            json!({ "url": "https://example.com", "maxpages": 3 }),
        )
        .await
        .unwrap_err();
        assert!(err.contains("maxpages"), "{err}");
        assert!(
            err.contains("max_pages"),
            "the error should list the real names: {err}"
        );
    }

    /// The accepted set is read from the published schema, so it cannot drift.
    #[test]
    fn every_tool_reports_its_schema_arguments() {
        for def in tool_definitions() {
            let name = def["name"].as_str().unwrap();
            let names = schema_arg_names(name).unwrap_or_default();
            let props = def["inputSchema"]["properties"].as_object().unwrap();
            assert_eq!(names.len(), props.len(), "{name}");
        }
    }

    // -- resources ---------------------------------------------------------

    fn big_artifact(state: &Arc<AppState>, bytes: usize) -> (String, String) {
        let job = state
            .job_manager
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                json!({}),
            )
            .unwrap();
        let dir = job.output_dir.clone().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let body: String = "a".repeat(bytes);
        std::fs::write(std::path::Path::new(&dir).join("crawl.json"), &body).unwrap();
        state
            .job_manager
            .update_status(&job.id, JobStatus::Completed)
            .unwrap();
        (
            job.id.clone(),
            format!("slither://job/{}/crawl.json", job.id),
        )
    }

    /// The B18 regression: a read used to return the whole artifact on one
    /// unbounded line — 917 KB for a 61-page crawl, and more than the 10 MB
    /// transport limit past ~730 pages.
    #[tokio::test]
    async fn a_large_artifact_read_is_bounded() {
        let (state, _tmp) = test_state();
        let (_job, uri) = big_artifact(&state, 5_000_000);

        let contents = read_resource(&state, &uri).unwrap();
        let text = contents["text"].as_str().unwrap();

        assert!(
            text.len() < RESOURCE_READ_DEFAULT_BYTES + 2_000,
            "read returned {} bytes; it must be windowed",
            text.len()
        );
        assert_eq!(contents["truncated"], true);
        assert_eq!(contents["totalBytes"], 5_000_000);
        assert!(
            text.contains("TRUNCATED"),
            "a truncated read must say so in the text the model actually reads"
        );
        assert!(
            text.contains("slither_query"),
            "the notice should point at the bounded tools: {}",
            &text[text.len().saturating_sub(400)..]
        );
        assert!(contents["nextUri"].is_string(), "paging must be possible");
    }

    /// The documented way to page through the rest actually works, and the
    /// windows reassemble into the original file.
    #[tokio::test]
    async fn the_next_uri_pages_through_the_whole_artifact() {
        let (state, _tmp) = test_state();
        let (_job, uri) = big_artifact(&state, 250_000);

        let mut assembled = String::new();
        let mut next = Some(uri);
        let mut reads = 0;
        while let Some(current) = next {
            let contents = read_resource(&state, &current).unwrap();
            let text = contents["text"].as_str().unwrap();
            let truncated = contents["truncated"].as_bool().unwrap();
            let payload = match text.find("\n\n[TRUNCATED:") {
                Some(cut) => &text[..cut],
                None => text,
            };
            assembled.push_str(payload);
            next = if truncated {
                Some(contents["nextUri"].as_str().unwrap().to_string())
            } else {
                None
            };
            reads += 1;
            assert!(reads < 20, "paging did not terminate");
        }
        assert_eq!(assembled.len(), 250_000, "paging must cover the whole file");
        assert_eq!(reads, 3);
    }

    /// A small artifact is returned whole — the bound is invisible in the
    /// common case, and the payload stays valid JSON.
    #[tokio::test]
    async fn a_small_artifact_is_returned_whole() {
        let (state, _tmp) = test_state();
        let job = redirect_crawl(&state);
        let contents = read_resource(&state, &format!("slither://job/{job}/crawl.json")).unwrap();
        assert_eq!(contents["truncated"], false);
        let text = contents["text"].as_str().unwrap();
        assert!(!text.contains("TRUNCATED"));
        serde_json::from_str::<Value>(text).expect("an untruncated artifact is still valid JSON");
    }

    /// An explicit window is honored and capped.
    #[tokio::test]
    async fn an_explicit_window_is_honored_and_capped() {
        let (state, _tmp) = test_state();
        let (job, _uri) = big_artifact(&state, 3_000_000);

        let contents = read_resource(
            &state,
            &format!("slither://job/{job}/crawl.json?offset=10&limit=50"),
        )
        .unwrap();
        assert_eq!(contents["byteOffset"], 10);
        assert_eq!(contents["byteLength"], 50);

        let contents = read_resource(
            &state,
            &format!("slither://job/{job}/crawl.json?limit=99999999"),
        )
        .unwrap();
        assert_eq!(contents["byteLength"], RESOURCE_READ_MAX_BYTES);
    }

    #[tokio::test]
    async fn a_malformed_window_is_an_error_not_a_silent_default() {
        let (state, _tmp) = test_state();
        let (job, _uri) = big_artifact(&state, 1_000);

        for bad in ["?offset=abc", "?limit=0", "?bogus=1", "?offset=99999"] {
            assert!(
                read_resource(&state, &format!("slither://job/{job}/crawl.json{bad}")).is_err(),
                "'{bad}' must be refused"
            );
        }
    }

    /// The allowlist still runs before any path join, window or not.
    #[tokio::test]
    async fn traversal_is_still_refused() {
        let (state, _tmp) = test_state();
        let (job, _uri) = big_artifact(&state, 10);
        assert!(read_resource(&state, &format!("slither://job/{job}/../../etc/passwd")).is_err());
        assert!(read_resource(
            &state,
            &format!("slither://job/{job}/../../etc/passwd?offset=0")
        )
        .is_err());
    }

    /// Multi-byte characters must not be split across a window boundary.
    #[tokio::test]
    async fn windows_land_on_character_boundaries() {
        let (state, _tmp) = test_state();
        let job = state
            .job_manager
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                json!({}),
            )
            .unwrap();
        let dir = job.output_dir.clone().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        // 3-byte characters, so most byte offsets are mid-character.
        let body: String = "→".repeat(100);
        std::fs::write(std::path::Path::new(&dir).join("crawl.json"), &body).unwrap();
        state
            .job_manager
            .update_status(&job.id, JobStatus::Completed)
            .unwrap();

        let contents = read_resource(
            &state,
            &format!("slither://job/{}/crawl.json?limit=10", job.id),
        )
        .unwrap();
        let text = contents["text"].as_str().unwrap();
        let payload = &text[..text.find("\n\n[TRUNCATED:").unwrap()];
        assert!(
            !payload.contains('\u{FFFD}'),
            "a window split a character: {payload:?}"
        );
        assert_eq!(
            payload.chars().count(),
            3,
            "10 bytes holds three 3-byte chars"
        );
    }

    /// The listing advertises size and points at the bounded tools, so a caller
    /// can see the cost before spending it.
    #[tokio::test]
    async fn the_listing_advertises_artifact_size() {
        let (state, _tmp) = test_state();
        let (job, _uri) = big_artifact(&state, 4_242);
        let listed = list_resources(&state);
        let entry = listed
            .iter()
            .find(|r| r["uri"] == format!("slither://job/{job}/crawl.json"))
            .expect("artifact should be listed");
        assert_eq!(entry["size"], 4_242);
        assert!(entry["description"]
            .as_str()
            .unwrap()
            .contains("slither_query"));
    }
}
