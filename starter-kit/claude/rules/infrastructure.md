---
description: Security and infrastructure rules — applies when working with cloud, firewall, IAM, Docker, or network configuration
globs: ["*.tf", "*.yaml", "*.yml", "docker-compose*", "Dockerfile*", "*.sh"]
---

## Zero Trust by Default

Any firewall rule, security group, IAM binding, or access control with source `0.0.0.0/0` (or `::/0`) is a defect. Fix it immediately. Do not flag it. Do not ask permission.

- `0.0.0.0/0` on any port = delete it on the spot
- Source ranges must be specific CIDRs (home IPs, VPC ranges, known services)
- IAM roles must follow least privilege — no `roles/owner` or `roles/editor` for service accounts
- Service ports must be locked to known IPs, never world-open
- Default-allow rules are deleted on first encounter

When detecting a violation: do not ask "want me to fix this?" — fix it, then note what was fixed.
