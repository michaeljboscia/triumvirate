---
name: documentation-standards
description: DNA-level documentation requirements for new capabilities — required file structure, enforcement rules, and project directory template. Use when creating any new capability, integration, or automation.
---

# Documentation Standards — DNA Level Requirement

**NEVER develop a new capability without complete documentation.**

When developing ANY new capability, integration, data collection, or automation, you MUST create the following documentation structure IN THE PROJECT DIRECTORY before considering the work complete.

## Required Documentation Files (ALL MANDATORY)

1. **README.md** — Project overview, quick start, API keys location, data coverage, known limitations, last updated date
2. **IMPLEMENTATION_GUIDE.md** (or `docs/implementation.md`) — Step-by-step setup, prerequisites, configuration, how to run, troubleshooting, how to reproduce
3. **Database Schema Documentation** — CREATE TABLE statements, relationships, indexes, sample queries, migration history
4. **ALL Scripts** — Save to project directory (NOT /tmp). Header comments with purpose, author, date. Document all API endpoints and parameters.
5. **Automation Scripts** — Prefect flows in project directory. Document triggers, schedule, HTTP endpoints, credentials. **NO n8n/Make/Zapier.**
6. **Reporting Templates WITH EXAMPLES** — At least ONE complete example report. Template structure docs. How to generate.
7. **Stored Procedures/Edge Functions** — Definitions, deployment instructions, test cases.
8. **Addendums** — Dated addendum files as capabilities evolve. Document what/why/when.
9. **HubSpot Custom Properties** (if applicable) — All properties with internal names, types, validation, purpose, population method, integration points. Save as `docs/hubspot_custom_properties.md`.

## Enforcement

**BEFORE marking any work "complete":**
1. All scripts saved to project directory (not /tmp)
2. README.md exists and documents the capability
3. At least ONE example output/report exists
4. Schema/database documentation is complete
5. Reproduction steps are documented

**If ANY are missing, the work is NOT complete.**

## When Scripts Are Lost

1. Acknowledge as CRITICAL FAILURE
2. Document exactly what is missing
3. DO NOT make excuses
4. Create recovery plan with proper documentation

## Project Directory Structure

```
ProjectName/
├── README.md
├── IMPLEMENTATION_GUIDE.md
├── scripts/
├── schema/
├── automation/
├── reports/
├── docs/
└── functions/
```
