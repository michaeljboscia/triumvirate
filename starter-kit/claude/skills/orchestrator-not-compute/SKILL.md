---
name: orchestrator-not-compute
description: Use when about to use Playwright browser_snapshot, browser_wait_for, or any browser MCP tool on a data-heavy page (ArcGIS, government GIS portals, SharePoint), or when writing a new script for a task that existing homebox infrastructure already handles (Prefect flows, Municipal Bloodhound, Docker containers).
---

# Orchestrator, Not Compute

## Core Principle

The context window is a **control plane**, not a data plane. You are an orchestrator who dispatches work to external infrastructure. You are NOT a compute engine that processes raw data in-context.

---

## Rule 1: Never Snapshot Data-Heavy Pages

Never use `browser_snapshot` or `browser_wait_for` on ArcGIS Hub, OpenData portals, government GIS viewers, SharePoint sites, or any page that may contain embedded map data, GeoJSON, or SVG tiles. **Default assumption: any government/GIS/portal page is data-heavy until proven otherwise.** If unsure, it's data-heavy.

**You will be tempted to:** "I just need to see the page to find the download link."
**Why that fails:** ArcGIS pages embed megabytes of encoded GeoJSON, SVG paths, and inline base64 images in the DOM. Serialization produces invalid Unicode surrogate pairs that crash the API call. Even when it doesn't crash, each snapshot wastes 100KB-5MB of irreplaceable context. This killed the session on 2026-03-21 and caused ~8 near-crashes in 2 days. Cost: 30+ minutes recovery per crash.

**The right way — by source type:**

**ArcGIS Hub/Portal** — REST API, never browser:
```bash
# Get FeatureServer URL from item ID (returns clean JSON, not 5MB DOM)
curl -s "https://www.arcgis.com/sharing/rest/content/items/{ITEM_ID}?f=json" | jq '.url'

# Download shapefile via Hub API v3 (follows redirect to S3)
curl -L -G "https://{hub-domain}/api/v3/datasets/{DATASET_ID}/downloads/data" \
  -d "format=shp" -d "spatialRefId=4326" -o "export.zip"
# Note: returns 202 if generating cache — poll until 200

# Or query FeatureServer directly for GeoJSON (paginate if >1000 records)
curl -G "{FEATURESERVER_URL}/0/query" \
  --data-urlencode "where=1=1" --data-urlencode "outFields=*" \
  --data-urlencode "f=geojson" > data.geojson
```

**Open Data portals (DCAT discovery)** — find datasets without browsing:
```bash
# Append /data.json to any open data portal → full JSON catalog with IDs + REST URLs
curl -s "https://{portal-domain}/data.json" | jq '.dataset[] | {title, identifier}'
```

**SharePoint** (e.g., connect.ncdot.gov):
```bash
# Public share links: append ?download=1
curl -L -c cookies.txt -b cookies.txt "{SHARE_URL}?download=1" -o file.zip

# Document library direct pattern:
curl -L "https://{tenant}.sharepoint.com/sites/{site}/_layouts/15/download.aspx?SourceUrl={URL-ENCODED-PATH}" -o file.zip
```

**If you absolutely must use browser in-context** — `browser_evaluate` with surgical JS only:
```javascript
// Extract ONLY download links — never the full DOM
Array.from(document.querySelectorAll('a'))
  .filter(a => a.href.match(/\.(zip|xlsx|csv|shp|geojson)/) || a.innerText.toLowerCase().includes('download'))
  .map(a => ({ text: a.innerText.trim(), url: a.href }))
```

Treat `browser_evaluate` like a SQL query for the DOM. You would never `SELECT * FROM massive_table` — don't snapshot an entire page either.

**Banned in `browser_evaluate`:** Never extract `innerHTML`, `outerHTML`, `document.documentElement`, or `document.body`. These dump the full DOM through a side door. Only extract specific attributes, text, and URLs.

Full patterns: `reference/data-source-patterns.md`

---

## Rule 2: Check for Existing Infrastructure Before Building

Before writing ANY new script, browser workflow, or data pipeline, check whether existing homebox infrastructure already handles the task.

**You will be tempted to:** "It's faster to write it myself than to find the existing tool."
**Why that fails:** On 2026-03-12, a throwaway CrUX refresh script hit a generated-column error, then a PostgREST conflict — 3 failures, 30 minutes wasted. The existing Prefect flow completed in 9 seconds. On 2026-03-21, manual Playwright browsing of GIS portals crashed the session when `discover_water_sewer_gis.py` (built the day before) does the same work headlessly on homebox Docker.

**The right way — check these locations in order:**

1. **Municipal Bloodhound** (`<your-project-path>/infrastructure/municipal-bloodhound/src/`):
   - `bloodhound.py` — zoning ordinance discovery
   - `discover_water_sewer_gis.py` — county GIS portal navigation
   - `discover_pdf_urls.py` — PDF source discovery
   - Pattern: Playwright + Gemini Flash in Docker, results to `/output/*.json`

2. **Prefect flows** on homebox Docker:
   ```bash
   # Discover existing flows first:
   ssh user@REDACTED_HOST 'docker exec scripting-host-prefect-worker-1 ls -la /app/flows/'

   # Then trigger the one you need:
   ssh user@REDACTED_HOST 'docker exec scripting-host-prefect-worker-1 python3 -c "from flows.X import Y; Y(args)"'
   ```
   **If a matching flow or script exists, you MUST use it. Do not write a new one.**

3. **Existing scripts** in the project's `scripts/` or `infrastructure/` directories

If nothing exists, build the new tool ON homebox (as a proper script in the infrastructure directory), not as a throwaway in `/tmp`.

---

## Rule 3: Scale = Always Delegate

If performing the same operation for more than 3 entities (counties, domains, URLs, etc.), the work MUST run on external infrastructure, not in-context.

**You will be tempted to:** "I'll just do a few more in-context, it's working fine so far."
**Why that fails:** 23 counties × multiple Playwright snapshots = guaranteed context exhaustion. Each snapshot accumulates. The failures compound silently until the session crashes. The bloodhound container exists precisely for batch GIS portal crawling.

**The right way:**
```bash
# Trigger the bloodhound container for batch work
ssh user@REDACTED_HOST "cd ~/tellus/infrastructure/municipal-bloodhound && docker-compose up -d"

# Or run a specific discovery script
ssh user@REDACTED_HOST "docker exec scripting-host-prefect-browser-worker-1 \
  python3 /app/src/discover_water_sewer_gis.py"

# Pull results back
rsync -av user@REDACTED_HOST:/home/mboscia/zoning_discovery/*.json /tmp/results/
```

The container outputs clean JSON. Your context window only sees the synthesized results.

---

## Rule 4: Prefer API-Direct Over Browser

Most "JS-rendered" pages have underlying API endpoints. Try the API first. Browser is a last resort.

**You will be tempted to:** "The page requires JavaScript rendering — I need Playwright."
**Why that fails:** On 2026-03-21, Playwright was used on the NC DPI page and NCDOT SharePoint when Gemini search had already found the download URLs. The browser was unnecessary overhead.

**The right way — escalation ladder:**

**Start here. Seriously.** A human finds county GIS data in 2 clicks: Google "[County] NC GIS" → download page. You have `mcp__gemini__gemini-search` which does the same thing. For a single county or dataset, this is almost always sufficient. The bloodhound and REST API tricks exist for BATCH work (60 jurisdictions). For one-off lookups, just search.

1. **Gemini search** (`mcp__gemini__gemini-search`) — almost always finds the direct download URL or API endpoint in one call. This is not a fallback — this is the PRIMARY tool.
2. **DCAT catalog** — append `/data.json` to any open data portal for full dataset index with IDs
3. **ArcGIS item metadata** — `sharing/rest/content/items/{ID}?f=json` returns the FeatureServer URL
4. **Hub API v3 export** — `api/v3/datasets/{ID}/downloads/data?format=shp` for direct shapefile download
5. **`curl` / `WebFetch`** — hit the URL directly (always use `-L` for redirects)
6. **`browser_evaluate` with targeted JS** — only if 1-5 fail, extract specific elements only
7. **Municipal Bloodhound on homebox** — for complex multi-page navigation requiring a real browser. This IS browser automation — it runs Playwright + Gemini Flash inside Docker on REDACTED_HOST. The browser runs THERE, not here. Your context window only sees the output JSON.
8. **`browser_snapshot` in-context** — BANNED for data-heavy pages. Only for simple text-only pages as absolute last resort. If you need a browser, use step 7 — that's what it's for.

---

## Before Any NC Data Discovery

The municipal bloodhound codebase at `<your-project-path>/infrastructure/municipal-bloodhound/` contains **9,700+ lines of learned patterns** and **585KB of JSON manifests** for NC government data. Check it BEFORE starting any discovery work.

| What you need | Check this file first |
|---------------|----------------------|
| Zoning ordinance URLs for any NC jurisdiction | `nc_zoning_pdf_sources.json` (~60 jurisdictions with URLs, hashes, scrape methods) |
| Municode client IDs / TOC structure | `nc_municode_clients.json` (97KB), `nc_municode_udo_toc.json` |
| AMLegal jurisdiction mappings | `nc_amlegal_manifest.json` (190KB) |
| Water/sewer GIS portal URLs + alt_urls | `src/discover_water_sewer_gis.py` TARGETS list (23+ counties with start_url + alt_urls) |
| How to navigate Municode programmatically | `src/municode_api.py` (679 lines — full API client) |
| How to extract from AMLegal | `src/adapter_amlegal.py` (1,071 lines — reverse-engineered API) |
| How to scrape non-standard portals | `src/scrape_town_websites_to_pdf.py` + county-specific scrapers |
| County GIS REST endpoints (non-standard paths) | Extraction report: `session-logs/tellus_crashed_session_e628aae4_extraction.md` |

If the data you need is about NC municipal/county government data, the answer is probably already in this codebase. Read before building.

---

## Validation Checklist

Run BEFORE any browser tool call or new script creation:

- [ ] Is this page ArcGIS, OpenData, GIS viewer, or map-heavy? → **Do NOT snapshot. Use REST API.**
- [ ] Did I check `infrastructure/municipal-bloodhound/src/` for an existing script? → **Check before building.**
- [ ] Did I check Prefect flows on homebox for an existing pipeline? → **Check before building.**
- [ ] Am I doing this for >3 entities? → **Delegate to homebox infrastructure.**
- [ ] Did I try Gemini search and direct curl before reaching for the browser? → **API-direct first.**
- [ ] If I must use the browser in-context, am I using `browser_evaluate` with targeted JS (no innerHTML/outerHTML/document.body)? → **Never snapshot, never dump DOM.**
- [ ] For NC data: did I check the bloodhound manifests/targets first? → **Read before building.**

---

## Reference

### Failure Context

| Date | Failure | Cost |
|------|---------|------|
| 2026-03-21 | Playwright `browser_wait_for` on NCDEQ ArcGIS page → Unicode surrogate crash → session death | 30+ min recovery, transcript mining |
| 2026-03-21 | ~8 Playwright snapshots on GIS portals across 2 days → repeated near-crashes | Cumulative context exhaustion |
| 2026-03-21 | Used Playwright on NCDOT/NC DPI when Gemini search already had URLs | Unnecessary context bloat |
| 2026-03-21 | Ignored `discover_water_sewer_gis.py` (built day before) and browsed manually | Session crash could have been avoided entirely |
| 2026-03-12 | Wrote throwaway CrUX script instead of triggering Prefect flow | 3 failures, 30 min wasted. Flow completed in 9 sec. |

### Root Cause (Triumvirate Diagnosis)

**Claude:** "I treat my context window as the execution environment" — worker, not orchestrator.
**Gemini:** "Pulling the data plane into the control plane" — Infrastructure Amnesia + Context Hubris.
**Codex:** "New work is cheaper than understanding the existing system" — optimizing for local control over global throughput.

**Convergent diagnosis:** The agent defaults to direct observation and in-context execution instead of delegating to existing infrastructure. The context window is treated as a data processing pipeline when it should be a control plane.

### Key Infrastructure (Homebox REDACTED_HOST)

| System | What It Does | How to Trigger |
|--------|-------------|----------------|
| Municipal Bloodhound | Playwright + Gemini Flash in Docker — navigates GIS portals, extracts data | `ssh user@REDACTED_HOST "cd ~/tellus/infrastructure/municipal-bloodhound && docker-compose up -d"` |
| Prefect Worker | Battle-tested data pipelines (CrUX, refresh flows, ETL) | `ssh user@REDACTED_HOST 'docker exec scripting-host-prefect-worker-1 python3 -c "..."'` |
| Browser Worker | Headless Playwright for ad-hoc browser tasks | `ssh user@REDACTED_HOST "docker exec scripting-host-prefect-browser-worker-1 python3 /app/src/..."` |
