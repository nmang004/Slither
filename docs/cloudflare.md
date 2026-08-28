# Cloudflare Browser Rendering Integration

> **Changed in 0.3.0:** the whole-site `slither crawl --backend cloudflare` backend
> was removed. Cloudflare Browser Rendering now powers **`slither screenshot`**,
> **`slither extract`**, and **`slither inspect --rendered` / `--compare`**. For
> JS-rendering during a full crawl, build with the `playwright` feature and use
> `--backend playwright`. Sections below that describe `--backend cloudflare` are
> retained for historical context and no longer reflect the CLI. (See `ROADMAP.md`
> decision item D-5.)

## Overview

Modern websites built with React, Angular, Vue, Next.js, and other JavaScript frameworks often return skeleton HTML to traditional HTTP crawlers. The actual content only appears after JavaScript executes in a browser. This means a standard crawl sees empty `<div id="root"></div>` tags instead of real page content, and Slither's 114+ SEO analyzers have nothing meaningful to work with.

Cloudflare Browser Rendering solves this by running a full Chromium instance in the cloud. When Slither uses the Cloudflare backend, every page is rendered with JavaScript before analysis begins. The result is the same fully-hydrated DOM that a real user (and Googlebot) sees.

**This integration is completely optional.** Slither works fully without it. If your sites serve server-rendered HTML, the default local crawl mode is faster and has no external dependencies. Cloudflare Browser Rendering is for when you need to audit JS-heavy SPAs that are invisible to traditional crawlers.

The integration unlocks four capabilities:

- **JS-rendered crawling** -- crawl SPAs and get real content for all 114+ analyzers
- **Screenshots** -- full-page or element-level captures at any viewport size
- **Single-page SEO audit** -- static, JS-rendered, or side-by-side comparison audit of any URL
- **Data extraction** -- pull structured business data from pages using a built-in SEO schema or custom prompts

---

## Getting Started

### Requirements

- A free Cloudflare account (no paid plan required)
- Your Cloudflare Account ID and an API token with Browser Rendering permissions

### Step-by-step setup

1. **Sign up** at [cloudflare.com](https://cloudflare.com) if you don't have an account.

2. **Create an API token** with the "Browser Rendering - Edit" permission:
   - Go to [dash.cloudflare.com/profile/api-tokens](https://dash.cloudflare.com/profile/api-tokens)
   - Click "Create Token"
   - Under "Custom token", click "Get started"
   - Add the permission: **Account > Browser Rendering > Edit**
   - Complete the token creation and copy the token value

3. **Find your Account ID:**
   - Log into the Cloudflare dashboard
   - Your Account ID is visible in the URL of any dashboard page: `dash.cloudflare.com/<ACCOUNT_ID>/...`
   - It's also shown on the right sidebar of your account's main page

4. **Configure Slither** using either the setup command or environment variables:

   **Option A: Interactive setup (recommended)**
   ```bash
   slither setup cloudflare
   ```
   This walks you through entering your Account ID and API Token, validates them against the Cloudflare API, and stores the credentials locally.

   **Option B: Environment variables**
   ```bash
   export CLOUDFLARE_ACCOUNT_ID="your-account-id"
   export CLOUDFLARE_API_TOKEN="your-api-token"
   ```

   **Option C: CLI flag overrides**

   Every Cloudflare command accepts `--cf-account-id` and `--cf-api-token` flags that override environment variables and stored credentials:
   ```bash
   slither crawl https://example.com --backend cloudflare \
     --cf-account-id abc123 --cf-api-token your-token
   ```

### Environment variables

| Variable | Description |
|----------|-------------|
| `CLOUDFLARE_ACCOUNT_ID` | Your Cloudflare account ID |
| `CLOUDFLARE_API_TOKEN` | API token with "Browser Rendering - Edit" permission |

### Verifying the setup

After configuring, the Slither dashboard (`slither` with no arguments) shows `cloudflare` with a checkmark in the Toolkit section:

```
  Toolkit:
    crawl  entity  link  cloudflare
```

---

## Free Tier & Pricing

Cloudflare Browser Rendering includes a free tier:

- **10 minutes of browser rendering time per day**
- Resets at midnight UTC

### What that means in practice

Browser time depends on page complexity. Simple marketing pages render in 1-2 seconds. Heavy SPAs with lots of API calls and animations can take 5-10 seconds. As a rough guide:

| Page complexity | Time per page | Pages per day (free tier) |
|----------------|---------------|--------------------------|
| Simple static/SSR | ~1s | ~600 |
| Typical SPA | ~3-5s | ~120-200 |
| Heavy JS app | ~6-10s | ~60-100 |

### Tracking usage

Slither displays browser time consumed after every Cloudflare crawl:

```
  Browser time: 42.3s used
```

All `render: true` requests are billed under Cloudflare's [Browser Rendering pricing](https://developers.cloudflare.com/browser-rendering/platform/pricing/). The free tier is generous enough for most small-to-medium site audits.

---

## Commands Reference

### `slither crawl --backend cloudflare`

Crawl a website using Cloudflare Browser Rendering. Every page is rendered in Chromium before Slither's analyzers run, so JS-rendered content is fully visible.

```bash
slither crawl https://my-react-app.com --backend cloudflare --max-pages 100
```

**How it works:**

1. Slither submits a crawl job to the Cloudflare `/crawl` API
2. Cloudflare discovers and renders pages in Chromium, following internal links
3. Slither polls the job status, showing progress as pages complete
4. Once finished, Slither retrieves all rendered HTML via cursor pagination
5. Slither runs the same 114+ analyzers on the rendered HTML
6. Unless `--skip-header-check` is set, Slither makes parallel GET requests to fill in security headers (HSTS, CSP, X-Frame-Options, etc.) since Cloudflare doesn't return HTTP response headers

**Full flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--backend` | `local` | Set to `cloudflare` to use CF rendering |
| `--max-pages` / `-m` | 500 | Maximum pages to crawl |
| `--depth` / `-d` | 10 | Maximum crawl depth |
| `--follow-subdomains` | false | Also crawl subdomains |
| `--skip-header-check` | false | Skip parallel GET requests for security headers |
| `--cf-account-id` | env | Override `CLOUDFLARE_ACCOUNT_ID` |
| `--cf-api-token` | env | Override `CLOUDFLARE_API_TOKEN` |
| `-o, --output` | auto | Output file path |
| `--no-html` | false | Skip HTML report generation |
| `--json-compact` | false | Minified JSON output |
| `--include-body-text` | false | Include page body text in JSON |
| `--summary-only` | false | Omit page data from JSON |
| `--csv` | false | Also generate CSV export |
| `-v, --verbose` | false | Verbose output |
| `-q, --quiet` | false | Suppress output except errors |

**Flags that do not apply in Cloudflare mode:**

These flags are accepted but have no effect when `--backend cloudflare` is set, because Cloudflare controls the crawl execution:

| Flag | Why it doesn't apply |
|------|---------------------|
| `--delay` | Cloudflare manages request pacing internally |
| `--concurrency` | Cloudflare manages parallelism internally |
| `--user-agent` | Cloudflare uses its own fixed user-agent |
| `--ignore-robots` | Cloudflare always respects robots.txt |

**Example with all relevant options:**

```bash
slither crawl https://my-spa.com \
  --backend cloudflare \
  --max-pages 200 \
  --depth 5 \
  --follow-subdomains \
  --include-body-text \
  --csv
```

---

### `slither screenshot <url>`

Capture a screenshot of any web page via Cloudflare Browser Rendering. The page is fully rendered in Chromium before the screenshot is taken.

```bash
slither screenshot https://example.com --full-page
```

**Flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--output` / `-o` | auto | Output file path |
| `--full-page` | false | Capture the full scrollable page, not just the viewport |
| `--format` | `png` | Image format: `png` or `jpeg` |
| `--quality` | -- | JPEG quality (1-100), only applies when format is `jpeg` |
| `--selector` | -- | CSS selector to capture a specific element |
| `--viewport` | `1920x1080` | Viewport size as `WxH` |
| `--cf-account-id` | env | Override `CLOUDFLARE_ACCOUNT_ID` |
| `--cf-api-token` | env | Override `CLOUDFLARE_API_TOKEN` |

**Examples:**

```bash
# Full-page screenshot at default 1920x1080 viewport
slither screenshot https://example.com --full-page

# Mobile viewport, JPEG format with quality control
slither screenshot https://example.com --viewport 375x812 --format jpeg --quality 80

# Capture a specific element
slither screenshot https://example.com --selector "#hero-section" --output hero.png

# Save to a specific path
slither screenshot https://example.com --full-page --output screenshots/homepage.png
```

---

### `slither inspect <url>`

Single-page SEO audit with three modes. Static mode works without Cloudflare credentials. Rendered and compare modes use Cloudflare Browser Rendering.

**Modes:**

| Mode | Flag | Cloudflare required? | Description |
|------|------|---------------------|-------------|
| Static | *(default)* | No | Fetch HTML via HTTP and run all SEO analyzers |
| Rendered | `--rendered` | Yes | Render in Chromium, then run all SEO analyzers |
| Compare | `--compare` | Yes | Run both static and rendered audits, then diff the results |

```bash
slither inspect https://example.com                    # static single-page audit
slither inspect https://example.com --rendered         # CF JS-rendered audit
slither inspect https://example.com --compare          # side-by-side static vs rendered
```

**Compare mode output example:**

```
  Slither v0.2.0 — inspect (compare)
  https://spa-site.com

  ● Fetching static HTML...                                [00:01]
  ● Rendering via Cloudflare...                            [00:04]
  ● Running analyzers...                                   [00:01]
  ● Comparing results...                                   [00:00]

  ✓ Inspect complete — 4 differences found

  ┌────────────────────┬──────────────┬──────────────┐
  │ Check              │ Static       │ Rendered     │
  ├────────────────────┼──────────────┼──────────────┤
  │ Title              │ (empty)      │ My SPA App   │
  │ Meta description   │ (empty)      │ Welcome to…  │
  │ H1 count           │ 0            │ 1            │
  │ Word count          │ 12           │ 847          │
  └────────────────────┴──────────────┴──────────────┘

  Output:
    HTML  inspect-spa-site.com-2026-04-03.html
```

**Flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--rendered` | false | Run audit on JS-rendered HTML via Cloudflare |
| `--compare` | false | Side-by-side static vs rendered audit |
| `--wait-for` | -- | CSS selector to wait for before capturing (rendered/compare modes) |
| `--wait-timeout` | 10 | Seconds to wait for selector |
| `--output` / `-o` | auto | Output file path |
| `--cf-account-id` | env | Override `CLOUDFLARE_ACCOUNT_ID` |
| `--cf-api-token` | env | Override `CLOUDFLARE_API_TOKEN` |
| `-q, --quiet` | false | Suppress output except errors and file paths |

**Examples:**

```bash
# Static audit (no Cloudflare needed)
slither inspect https://example.com

# Rendered audit, wait for dynamic content
slither inspect https://spa-site.com --rendered --wait-for "#app-loaded"

# Compare mode with custom output path
slither inspect https://spa-site.com --compare --output audit-comparison.html

# Override Cloudflare credentials inline
slither inspect https://spa-site.com --rendered \
  --cf-account-id abc123 --cf-api-token your-token
```

---

### `slither extract <url>`

Extract structured data from a web page using Cloudflare's Browser Rendering `/json` endpoint. The page is rendered in Chromium, then data is extracted based on a schema or prompt.

**Default behavior (SEO preset):**

With no `--prompt` or `--schema` flag, Slither uses a built-in SEO extraction schema that pulls:

- Business name and type
- Phone number and email
- Address (street, city, state, ZIP)
- Services offered
- Service areas
- Social media profiles
- Business hours

```bash
slither extract https://joesplumbing.com
```

**Example output:**

```
  Business Name:  Joe's Plumbing & Drain
  Business Type:  Plumber
  Phone:          (555) 123-4567
  Email:          info@joesplumbing.com
  Address:        123 Main St, Springfield, IL, 62701
  Services:       Drain cleaning, Water heater repair, Pipe installation, Emergency plumbing
  Service Areas:  Springfield, Decatur, Champaign, Bloomington
  Hours:          Mon-Fri 7am-6pm, Sat 8am-2pm
  Social:
    facebook: https://facebook.com/joesplumbing
    yelp: https://yelp.com/biz/joes-plumbing-springfield
```

**Flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--prompt` | -- | Custom extraction prompt (overrides SEO preset) |
| `--schema` | -- | Path to a custom JSON schema file |
| `--output` / `-o` | stdout | Save extracted data to a file |
| `--wait-for` | -- | Wait for a CSS selector before extracting |
| `--cf-account-id` | env | Override `CLOUDFLARE_ACCOUNT_ID` |
| `--cf-api-token` | env | Override `CLOUDFLARE_API_TOKEN` |

**Examples:**

```bash
# Default SEO preset extraction
slither extract https://joesplumbing.com

# Custom prompt
slither extract https://example.com --prompt "Extract all product names and prices"

# Custom schema from file
slither extract https://store.com --schema product-schema.json --output products.json

# Wait for dynamic content to load before extracting
slither extract https://spa-app.com --wait-for ".product-list"
```

**Using a custom schema:**

Create a JSON file following the Cloudflare Browser Rendering response format schema:

```json
{
  "type": "json_schema",
  "schema": {
    "type": "object",
    "properties": {
      "product_name": { "type": "string" },
      "price": { "type": "string" },
      "description": { "type": "string" },
      "availability": { "type": "string" }
    }
  }
}
```

Then pass it with `--schema`:

```bash
slither extract https://store.com --schema product-schema.json
```

---

## How It Works

### Local crawl (default)

```
Slither  --(HTTP GET)-->  Website
        <--(HTML)------
        --> Parse HTML
        --> Run 114+ analyzers
        --> Generate report
```

Slither fetches raw HTML directly using reqwest. Fast, lightweight, no external dependencies. Works perfectly for server-rendered sites. No JavaScript is executed.

### Cloudflare crawl

```
Slither  --(submit job)-->  Cloudflare Browser Rendering API
        <--(job ID)------
        --(poll status)-->  CF renders pages in Chromium (JS executes)
        <--(progress)----
        --(retrieve)---->  CF returns rendered HTML
        <--(HTML)--------
        --> Run same 114+ analyzers
        --> Generate report
```

1. **Job submission:** Slither sends the seed URL, max pages, depth, and options to the Cloudflare `/crawl` endpoint. Cloudflare returns a job ID.

2. **Polling:** Slither polls every 5 seconds until the job reaches a terminal state (completed, errored, cancelled). During polling, Slither emits progress updates showing pages crawled vs. total discovered.

3. **Retrieval:** After completion, Slither paginates through all results using cursor-based pagination, deduplicating by URL.

4. **Security headers:** Cloudflare returns rendered HTML but not HTTP response headers. Slither makes parallel GET requests to each page's URL to fill in security headers (HSTS, CSP, X-Content-Type-Options, X-Frame-Options, Referrer-Policy). This step can be skipped with `--skip-header-check`.

5. **Analysis:** The rendered HTML is passed through the same parser and 114+ analyzers used in local mode. Reports are identical in format.

### JS rendering detection

After a local crawl, Slither checks the results for signs of JS-rendered content. If more than 25% of pages returned HTTP 200 but have zero word count and empty body text, Slither flags this in the report and suggests re-crawling with `--backend cloudflare`.

---

## Known Limitations

- **No redirect chain data.** Cloudflare returns the final URL after all redirects, but does not expose the redirect chain. Redirect-related issues (e.g., long redirect chains) cannot be detected in CF mode.

- **Response times are approximate.** In CF mode, per-page response times come from the parallel header fetch, not from the actual page render. They reflect server response time, not the full JS rendering time.

- **`--ignore-robots` is not supported.** Cloudflare always respects robots.txt. Pages disallowed by robots.txt will be skipped and reported as "disallowed" in the crawl results.

- **Fixed user-agent.** Cloudflare uses `CloudflareBrowserRenderingCrawler/1.0` as the user-agent for rendered requests. The `--user-agent` flag does not apply in CF mode (it is used for the parallel header requests only).

- **Screenshots and extract are standalone commands.** They are not embedded into crawl reports. You run them separately against individual URLs.

- **10-minute daily limit on the free tier.** Large sites may require multiple days or a paid plan. Slither reports browser time used so you can plan accordingly.

---

## Troubleshooting

### "Cloudflare auth failed"

Your API token is missing, expired, or lacks the correct permission.

**Fix:**
- Verify your token at [dash.cloudflare.com/profile/api-tokens](https://dash.cloudflare.com/profile/api-tokens)
- Ensure it has the **"Browser Rendering - Edit"** permission
- Run `slither setup cloudflare` to re-enter and validate credentials
- If using environment variables, check they are exported in your current shell

### "Daily browser time limit reached"

You've used all 10 minutes of free-tier browser rendering time for the day.

**Fix:**
- The limit resets at **midnight UTC**
- Reduce `--max-pages` to stay within the budget on future crawls
- Use the local backend (`slither crawl <url>` without `--backend cloudflare`) as a fallback for server-rendered pages
- Consider a paid Cloudflare plan for higher limits

### "Crawl timed out"

The crawl job did not complete within the 10-minute deadline.

**Fix:**
- Reduce `--max-pages` (try 50-100 for large sites)
- Reduce `--depth` to limit how far from the seed URL the crawl goes
- Check that the target site is reachable and not blocking Cloudflare's IP ranges

### "Cloudflare API unreachable"

Slither cannot connect to the Cloudflare API.

**Fix:**
- Check your internet connection
- Verify that `api.cloudflare.com` is not blocked by a firewall or proxy
- Use the local backend as a fallback: `slither crawl <url>`

### "Rate limited by Cloudflare"

This is typically transient. Slither automatically retries with exponential backoff when rate-limited.

**Fix:**
- If it persists, wait a few minutes and try again
- Reduce `--max-pages` to generate less API traffic

### Cloudflare credentials not detected

If `slither` shows `cloudflare` without a checkmark in the Toolkit section, credentials are not configured.

**Fix:**
- Run `slither setup cloudflare` to configure interactively
- Or set `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` environment variables
- Or pass `--cf-account-id` and `--cf-api-token` flags directly to commands
