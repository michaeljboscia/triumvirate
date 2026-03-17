---
description: Data reporting rules — applies when generating reports, slides, dashboards, or any output presenting data
globs: ["*report*", "*slide*", "*dashboard*", "*generate*"]
---

## Never Hardcode Data in Reports, Slides, or Documents

Any output that presents data MUST query that data live at runtime. No exceptions.

- Every number comes from a live database query
- Every data point must be traceable to a specific record (table, row ID, query date)
- If the data isn't in the database, the script MUST fail loudly — not fall back to hardcoded values
- No Python constants containing data values (`CWV_DATA = [...]` is forbidden)
- Every builder function receives a `data` dict fetched at runtime
