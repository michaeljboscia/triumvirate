---
name: file-taxonomy
description: File organization taxonomy and decision tree for where to put files in GTM Machine projects. Use when creating any new file to determine the correct location.
---

# File Organization Taxonomy — WHERE TO PUT THINGS

**Before creating ANY file, use this decision tree to find the RIGHT location.**

## Folder Taxonomy

| Folder | Purpose | Put Here If... |
|--------|---------|----------------|
| `Adyntel/` | Ad intelligence data & scripts | Related to ad spend monitoring |
| `API Reference Docs/` | Third-party API documentation | External API reference docs |
| `Correlation Engine/` | Pain signal correlation system | Multi-signal analysis |
| `CruxHistoricalPerformance/` | Chrome UX historical data | CrUX API data |
| `DataForSEO/` | DataForSEO integration | Traffic data, keyword data |
| `DocumentFactory/` | Report generation scripts | Scripts that GENERATE reports |
| `Google PSI/` | PageSpeed Insights core | PSI API integration |
| `HubSpot Data Exports/` | CSV exports from/for HubSpot | CSV for HubSpot import/export |
| `Implementation Archives/` | Completed/superseded implementations | Old guides, archived projects |
| `InstantlyHubspot/` | Instantly.ai + HubSpot integration | Email outreach automation |
| `n8n docs/` | n8n workflow documentation | Workflow guides, backups |
| `Outbound content pipeline/` | Active outreach content system | Sequence generation, messaging |
| `Pain Sensor Orchestration/` | Pain sensor system architecture | Multi-sensor coordination |
| `PainSensorReports/` | Pain sensor report outputs | Generated reports for companies |
| `psi_v2/` | PSI v2 system | Enhanced PSI reports |
| `Research & Strategy/` | Market research, frameworks | Pain point inventories, strategy |
| `Scripts/` | Standalone utility scripts | One-off scripts not tied to projects |
| `SecurityPod/` | Security scanning system | ZAP, Nuclei, Amass |
| `supabase-backups/` | Supabase backups | Pre-modification backups |
| `TargetCompanyReports/` | Company-specific outputs | Reports FOR specific companies |
| `wappalyzer-hubspot-automation/` | Technology detection | Wappalyzer API, tech stacks |

## Decision Tree

1. Report OUTPUT for a specific company? → `TargetCompanyReports/<domain>/`
2. Script that GENERATES reports? → `DocumentFactory/<project>/scripts/`
3. n8n workflow backup/guide? → `n8n docs/`
4. CSV data for HubSpot? → `HubSpot Data Exports/`
5. API reference documentation? → `API Reference Docs/`
6. Research or strategy? → `Research & Strategy/`
7. New capability implementation? → Create project folder with standard structure
8. One-off utility script? → `Scripts/`
9. Old/superseded? → `Older Archive/` or `Implementation Archives/`
10. None of the above? → **ASK before creating. Do NOT dump in root.**

## Rules

- NEVER create files in root — find the right folder
- NEVER create a new folder without checking if one exists
- NEVER use generic names like "docs.md", "notes.md"
- NEVER save to /tmp — files will be lost
- ALWAYS use descriptive filenames with domain, date, version

## Filename Conventions

- Company reports: `<domain>_<report_type>_<version>_<YYYYMMDD_HHMMSS>.<ext>`
- Implementation docs: `<system>-<purpose>-<version>.md`
- Scripts: `<action>_<target>_<version>.py`
- Addendums: `ADDENDUM-<YYYY-MM-DD>-<topic>.md`
