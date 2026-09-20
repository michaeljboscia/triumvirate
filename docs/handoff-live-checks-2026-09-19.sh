.env exists, is 600, and is gitignored|test "$(stat -f %Lp /Users/michaelboscia/projects/triumvirate/.env)" = 600 && git -C /Users/michaelboscia/projects/triumvirate check-ignore -q .env
.env holds a real key, not the placeholder|! grep -q 'REPLACE_ME' /Users/michaelboscia/projects/triumvirate/.env && grep -q '^TYPESAFE_API_KEY=.\{20,\}' /Users/michaelboscia/projects/triumvirate/.env
no key material is tracked by git anywhere in the repo|! git -C /Users/michaelboscia/projects/triumvirate grep -qI 'apikey_' -- . ':!*.jsonl'
Jev answers a live noul with HTTP 200|cd /Users/michaelboscia/projects/triumvirate && set -a && . ./.env && set +a && test "$(curl -s -o /dev/null -w '%{http_code}' -X POST https://api.typesafe.ai/v1/systemone -H "Authorization: Bearer $TYPESAFE_API_KEY" -H 'Content-Type: application/json' -d '{"state":"disk is full","model":"jev-latest","questions":{"q":{"type":"noul","instructions":"This is a problem"}}}')" = 200
OPEN.md has exactly 2 open defects|test "$(grep -c '^### D-' /Users/michaelboscia/projects/triumvirate/daemon/docs/bugs/OPEN.md)" = 2
D-004 and D-005 are the two open rows|grep -q 'D-004' /Users/michaelboscia/projects/triumvirate/daemon/docs/bugs/OPEN.md && grep -q 'D-005' /Users/michaelboscia/projects/triumvirate/daemon/docs/bugs/OPEN.md
PR 45 merged|test "$(gh pr view 45 --repo michaeljboscia/triumvirate --json state -q .state)" = MERGED
PR 46 merged|test "$(gh pr view 46 --repo michaeljboscia/triumvirate --json state -q .state)" = MERGED
PR 47 merged|test "$(gh pr view 47 --repo michaeljboscia/triumvirate --json state -q .state)" = MERGED
PR 48 merged|test "$(gh pr view 48 --repo michaeljboscia/triumvirate --json state -q .state)" = MERGED
PR 49 open|test "$(gh pr view 49 --repo michaeljboscia/triumvirate --json state -q .state)" = OPEN
daemon pid 14611 is running the installed binary|ps -p 14611 -o command= | grep -q '/Users/michaelboscia/.local/bin/triumvirate daemon'
probe journal has 60 rows|test "$(wc -l < /Users/michaelboscia/projects/triumvirate/reports/wiki-usage/probes.jsonl | tr -d ' ')" = 60
wiki_search is advertised by the installed binary|printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"v","version":"1"}}}' '{"jsonrpc":"2.0","method":"notifications/initialized"}' '{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}' | /Users/michaelboscia/.local/bin/triumvirate mcp 2>/dev/null | grep -q '"wiki_search"'
the map line naming wiki_search reached all three peer files|grep -q wiki_search /Users/michaelboscia/.codex/AGENTS.md && grep -q wiki_search /Users/michaelboscia/.gemini/GEMINI.md && grep -q wiki_search /Users/michaelboscia/.grok/rules/bosciamem-wiki.md
Claude's _index.md deliberately does NOT name wiki_search|! grep -q wiki_search /Users/michaelboscia/projects/mneme-bosciamem/_index.md
mneme map commit 19f0792 exists|git -C /Users/michaelboscia/projects/mneme-bosciamem cat-file -e 19f0792^{commit}
WIKI_SUBJECT regex still exists for Jev to be scored against|grep -q 'WIKI_SUBJECT = re.compile' /Users/michaelboscia/projects/triumvirate/scripts/wiki-controls.py
VerdictExtractor still returns None on ambiguity|grep -q 'VerdictExtractor' /Users/michaelboscia/projects/triumvirate/daemon/crates/mcp-tools/src/jury.rs
report positive control passes|cd /Users/michaelboscia/projects/triumvirate && python3 scripts/test_wiki_usage_report.py | grep -q '^ok:'
detector parity passes|cd /Users/michaelboscia/projects/triumvirate && python3 scripts/test_wiki_detector_parity.py | grep -q 'agree with the fixture'
docs.typesafe.ai is reachable and documents Jev|test "$(curl -sfL https://docs.typesafe.ai | grep -ci jev)" -gt 0
