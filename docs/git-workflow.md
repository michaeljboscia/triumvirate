# Git Workflow — How Triumvirate Uses Git

Triumvirate uses git in two distinct ways: **project git** (your normal source code repos) and **memory git** (a private repo that stores AI session logs). This document explains both, and why the separation matters.

---

## Two Repos, Two Purposes

```
~/projects/my-app/              ← Project repo (your code)
  .git/
  .claude/taxonomy.json
  src/
  tests/

~/.ai-memory/                   ← Memory repo (AI working memory)
  .git/
  my-app/
    owner--client_domain_myapp_feature_20260322_v1_claude.md
    owner--client_domain_myapp_feature_20260322_v1_gemini.md
```

**Project repo:** Your source code. You control commits, branches, PRs. Triumvirate interacts with it through auto-staging (hooks) and commit discipline (skills).

**Memory repo:** AI working memory. Session logs, Stenographer notes, compaction summaries. Managed automatically by hooks. You rarely touch it directly.

### Why separate?

Session logs are not project artifacts. They contain:
- Internal reasoning traces ("I tried approach A, it failed because...")
- Debugging notes ("The race condition is in line 142 of...")
- Inter-agent communication records ("Gemini said the architecture should...")
- Personal workflow details

This doesn't belong in your project's git history. It would clutter PRs, confuse collaborators, and expose internal AI workflow details. The memory repo keeps AI context private and separate.

---

## The Memory Repo (`~/.ai-memory/`)

### Setup

The installer creates this automatically:

```bash
mkdir -p ~/.ai-memory && cd ~/.ai-memory && git init
```

**Optional but recommended:** Push to a private remote so your AI memory is backed up and accessible from multiple machines:

```bash
cd ~/.ai-memory
gh repo create ai-memory --private -y
git remote add origin git@github.com:yourname/ai-memory.git
```

### What goes in it

Every session log from every agent across every project:

```
~/.ai-memory/
├── triumvirate/
│   ├── michaeljboscia--core_infra_triumvirate_backport_20260322_v1_claude.md
│   └── michaeljboscia--core_infra_triumvirate_backport_20260322_v1_gemini.md
├── my-app/
│   ├── michaeljboscia--client_frontend_myapp_auth_20260320_v1_claude.md
│   ├── michaeljboscia--client_frontend_myapp_auth_20260320_v2_claude.md
│   └── michaeljboscia--client_frontend_myapp_auth_20260320_v1_codex.md
└── another-project/
    └── ...
```

Each project gets a subdirectory. Session logs accumulate over time. You can `git log` to see when sessions happened, `git diff` to see what changed between saves, and `git blame` to trace when a specific note was written.

### How session logs are named

```
owner--client_domain_repo_feature_YYYYMMDD_vN_agent.md
```

The parts:
- **owner** — from `taxonomy.json` (`"owner"` field, typically your GitHub username)
- **client** — from `taxonomy.json` (`"client"` field)
- **domain** — from `taxonomy.json` (`"domain"` field)
- **repo** — from `taxonomy.json` (`"repo"` field)
- **feature** — from `taxonomy.json` (`"feature"` field)
- **YYYYMMDD** — date of the session
- **vN** — version number within the same day+feature (increments: v1, v2, v3...)
- **agent** — which agent wrote it (`claude`, `gemini`, or `codex`)

The version number is how Triumvirate handles multiple sessions per day on the same feature. The first session is v1, the second is v2, etc. This is computed by `agent-log-path.ts` — it scans the directory for existing logs and increments.

### How session logs get committed

The `pre-compact.sh` hook handles this automatically:

```
Context approaching limit
  → pre-compact.sh fires
  → Gemini summarizes the transcript
  → Summary is written to ~/.ai-memory/<project>/<session-log>.md
  → git add <session-log>.md
  → git commit -m "session: <project> v<N> claude — <timestamp>"
  → Compaction proceeds
```

You don't need to manually commit session logs. The hook does it every time context compacts.

**Important:** The hook only commits to the memory repo, never to your project repo. These are separate git repositories.

### How to push memory to remote

The hooks don't auto-push (pushing requires network access and could slow down compaction). Push manually when you want to back up:

```bash
cd ~/.ai-memory && git push
```

Or set up a cron job / periodic push if you want automation.

---

## Project Repo — Auto-Staging

### What auto-staging does

The `post-tool-use.sh` hook runs after every `Edit`, `Write`, `Bash`, or `Agent` tool call. When a file is modified, it immediately stages the change:

```bash
git add <modified-file>
```

### Why auto-staging matters

Without auto-staging, you accumulate unstaged changes throughout a session. If the session crashes, compacts, or you forget to commit, those changes exist only as working directory modifications — easy to lose, hard to diff against.

With auto-staging, every edit is immediately in git's staging area. This means:

1. **`git diff --staged` shows everything Claude did** — a running audit trail
2. **Nothing is lost on crash** — staged changes survive process death (they're in git's index)
3. **Commits are easy** — when you're ready to commit, everything is already staged
4. **Activity logging** — the hook also writes a line to the session activity table, so you can see what was changed and when

### What auto-staging does NOT do

- It does NOT commit. You (or Claude, when you ask it to) decide when to commit.
- It does NOT push. Commits stay local until you push.
- It does NOT stage files outside the project repo. Only files in the current git repository.
- It does NOT stage `.env` files, credentials, or other sensitive files. The hook respects `.gitignore`.

### The staging flow

```
You ask Claude to edit src/auth.ts
  → Claude calls the Edit tool
  → The edit is applied to the file
  → post-tool-use.sh fires
  → Hook detects src/auth.ts was modified
  → Hook runs: git add src/auth.ts
  → Hook logs: "Staged: src/auth.ts (+15/-3)"
  → You see the log entry in the session activity table
```

---

## Commit Discipline

Triumvirate's `CLAUDE.md` template includes the rule: **"Commit early, commit often."** But it also says: **"Don't push without asking."**

### The workflow

1. Claude makes changes (auto-staged by hooks)
2. You review changes: `git diff --staged`
3. You tell Claude to commit (or commit yourself)
4. Claude creates a commit with a descriptive message
5. You tell Claude to push (or push yourself)

### Why Claude doesn't auto-commit

Committing is a judgment call. Some changes are works-in-progress that shouldn't be committed yet. Some changes span multiple files and should be committed together. Some changes need a specific commit message. Auto-committing would remove your control over the project's git history.

Auto-staging is safe because it's reversible (`git reset` unstages everything). Auto-committing is not — it creates permanent history entries.

### Why Claude doesn't auto-push

Pushing affects shared state (remote repos, CI/CD, collaborators). It's irreversible in practice. Claude should never push without explicit permission because:

- You might be on a branch that isn't ready
- You might have commits you want to squash first
- The push might trigger CI that you're not ready for
- The push might notify collaborators prematurely

---

## Session Log Versioning

### Within a single session

Stenographer writes incremental saves to the SAME session log file, appending paragraphs as the session progresses:

```
session-log_v1_claude.md
  ## SESSION UPDATE: 14:30
  [first stenographer save — ~50K tokens]

  ## SESSION UPDATE: 14:45
  [second stenographer save — ~100K tokens]

  ## SESSION UPDATE: 15:00
  [pre-compact save — Gemini summary of full session]
```

### Across sessions

Each new Claude session on the same day+feature gets a new version number:

```
session-log_v1_claude.md   ← morning session
session-log_v2_claude.md   ← afternoon session (started fresh)
session-log_v3_claude.md   ← evening session
```

### Across days

The date changes:

```
..._20260321_v1_claude.md  ← yesterday
..._20260322_v1_claude.md  ← today
```

### How the next session finds the latest log

When Claude starts a new session, `session-start.sh` calls `session_log_path.py --recover` which:

1. Reads `.claude/taxonomy.json` to know the project identity
2. Looks in `$AI_MEMORY_DIR/<repo>/` for files matching the taxonomy
3. Sorts by date (descending) and version (descending)
4. Returns the most recent one

Claude reads that file and has full context from the previous session.

---

## The AI Memory Repo in Practice

### Typical size

After 3 months of daily use across 10 projects, the memory repo might contain:
- 200-500 session log files
- 5-20 MB total (session logs are plain markdown, very compact)
- 500-2000 commits (one per compaction)

### Maintenance

The memory repo grows but doesn't need pruning. Session logs are small and git handles thousands of small files well. If you want to archive old projects:

```bash
cd ~/.ai-memory
mkdir -p archive
mv old-project/ archive/old-project/
git add -A && git commit -m "archive: move old-project to archive"
```

### Multi-machine sync

If you work on multiple machines, push the memory repo to a private remote:

```bash
# On machine A
cd ~/.ai-memory && git push

# On machine B
cd ~/.ai-memory && git pull
```

Now Claude on machine B can resume context from sessions that happened on machine A.

### Searching your memory

```bash
# Find all sessions for a specific project
ls ~/.ai-memory/my-project/

# Find what you worked on last Tuesday
ls ~/.ai-memory/*/\*20260318\*

# Search across all sessions for a keyword
grep -r "authentication" ~/.ai-memory/

# See git history of session saves
cd ~/.ai-memory && git log --oneline | head -20
```

---

## Git and the Hooks — Complete Picture

Here's every git operation that Triumvirate's hooks perform:

| Hook | Git Operation | Which Repo | When |
|------|--------------|------------|------|
| `post-tool-use.sh` | `git add <file>` | Project repo | After every edit/write/bash |
| `pre-compact.sh` | `git add <session-log>` | Memory repo | Before compaction |
| `pre-compact.sh` | `git commit -m "session: ..."` | Memory repo | Before compaction |
| `session-start.sh` | `git log` (read-only) | Memory repo | On session start (to find latest log) |

**No hook ever runs `git push`.** Pushing is always manual and explicit.

**No hook ever commits to the project repo.** Only the memory repo gets automatic commits.

**No hook runs destructive git commands** (`reset`, `rebase`, `force-push`, `clean`). The hooks are append-only — they add files and create commits, nothing else.
