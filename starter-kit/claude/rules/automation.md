---
description: Automation and orchestration rules — applies when building workflows, pipelines, or integrations
globs: ["*.py", "*.ts", "*.js"]
---

## No Visual Workflow Automation — EVER

NEVER use n8n, Make (formerly Integromat), or Zapier. Not for anything. Not even temporarily.

| Need | Use Instead |
|------|-------------|
| Orchestration / scheduling | Prefect |
| HTTP / API calls | Python (`requests`, `httpx`) |
| Data pipelines | Python scripts + Prefect flows |
| Anything n8n / Make / Zapier does | Python + the right library |
