# Getting API Access — The Free Path

Triumvirate needs three CLI agents. The good news: two of them are completely free to use, and the third has multiple access options. You can be up and running in under 10 minutes.

---

## The Cost Summary

| Agent | CLI | Cost | What You Need |
|-------|-----|------|---------------|
| Claude Code | `claude` | Paid (see options below) | Anthropic API key or Claude subscription |
| Gemini CLI | `gemini` | **Free** | Google AI Studio API key (free tier) |
| Codex | `codex` | **Free** | OpenAI API key (free credits for new accounts) |

**Bottom line:** You're already paying for Claude (you're reading this). Gemini and Codex cost nothing to add.

---

## Gemini CLI — Free

Gemini CLI uses the Google AI Studio API, which has a generous free tier. No credit card required.

### Step 1: Get a Google AI Studio API Key

1. Go to [Google AI Studio](https://aistudio.google.com/apikey)
2. Sign in with your Google account
3. Click "Create API Key"
4. Copy the key

The free tier includes:
- Gemini 2.5 Pro and Gemini 2.5 Flash
- Generous rate limits for personal use
- No credit card required
- No expiration

### Step 2: Install Gemini CLI

```bash
npm install -g @google/gemini-cli
```

Follow the auth prompts — it will ask for your API key or let you sign in with Google.

### Step 3: Verify

```bash
gemini -p "Say hello" --output-format text
```

You should get a response. That's it — Gemini is ready.

### Step 4: Add to Triumvirate

Add your key to `~/.claude/.env`:
```
GEMINI_API_KEY=<your-google-ai-studio-key>
```

This key is used by:
- Pre-compact hooks (Gemini summarizes your session before compaction)
- Gap-fill (Gemini fills gaps in session logs)
- All `spawn_daemon` / `ask_daemon` Gemini operations
- The oracle engine

---

## Codex — Free

Codex CLI uses the OpenAI API. New OpenAI accounts receive free credits, and the CLI itself is open source.

### Step 1: Get an OpenAI API Key

1. Go to [OpenAI Platform](https://platform.openai.com/api-keys)
2. Create an account (or sign in)
3. Navigate to API Keys
4. Click "Create new secret key"
5. Copy the key

New accounts typically receive free credits. Check your [usage dashboard](https://platform.openai.com/usage) for your current balance.

### Step 2: Install Codex CLI

```bash
npm install -g @openai/codex
```

### Step 3: Set your API key

```bash
export OPENAI_API_KEY=<your-openai-key>
```

Or add it to your shell profile (`~/.zshrc` or `~/.bashrc`) so it persists.

### Step 4: Verify

```bash
codex -p "Say hello"
```

### Step 5: Add to Triumvirate

Codex reads `OPENAI_API_KEY` from the environment. As long as it's set in your shell, Triumvirate's MCP server will use it automatically when spawning Codex daemons.

---

## Claude Code — Your Primary Agent

Claude Code is the orchestrator — the agent you interact with directly. The others are workers it delegates to.

### Option 1: Anthropic API Key (pay-per-use)

1. Go to [Anthropic Console](https://console.anthropic.com/)
2. Create an account
3. Add payment method
4. Navigate to API Keys and create one
5. Run `claude` and enter your API key when prompted

Cost depends on usage. Claude Opus (the recommended model) is the most capable but also the most expensive per token.

### Option 2: Claude Pro or Max Subscription

If you have a Claude Pro ($20/month) or Max ($100/month) subscription:

1. Install Claude Code: `npm install -g @anthropic-ai/claude-code`
2. Run `claude`
3. It will open a browser for authentication
4. Sign in with your Claude account

The subscription gives you usage-based access to Claude Code without separate API billing.

### Verify

```bash
claude
```

You should see the Claude Code interface. Type a message and get a response.

---

## Putting It All Together

Once you have all three CLIs working:

```bash
# 1. Clone Triumvirate
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit

# 2. Run the installer
chmod +x install.sh
./install.sh

# 3. Set up credentials
cp ~/.claude/.env.example ~/.claude/.env
# Edit ~/.claude/.env — add your GEMINI_API_KEY
# OPENAI_API_KEY should already be in your shell env

# 4. Start Claude
claude
```

Claude will now have access to Gemini and Codex through the inter-agent MCP server. Try it:

```
You: "Spawn a Gemini daemon and ask it to summarize this project's README"
```

Claude will use `spawn_daemon`, `ask_daemon`, and return Gemini's analysis — all without you leaving the Claude interface.

---

## Troubleshooting

### "Gemini CLI not found"

Make sure `gemini` is in your PATH:
```bash
which gemini
```
If not found, reinstall or add the npm global bin to your PATH.

### "Codex CLI not found"

Same as above:
```bash
which codex
```

### "GEMINI_API_KEY not set"

The hooks source `~/.claude/.env` on session start. Make sure:
1. The file exists: `ls ~/.claude/.env`
2. It contains your `GEMINI_API_KEY`
3. The key is valid (test with `gemini -p "test" --output-format text`)

### "Quota exhausted" errors

**Gemini:** The free tier has rate limits. If you hit them, wait a few minutes. The model fallback chain will automatically try less-capable models first.

**Codex:** If your free credits run out, you'll need to add payment. Check your balance at [platform.openai.com/usage](https://platform.openai.com/usage).

### "MCP server failed to start"

Rebuild the server:
```bash
cd /path/to/triumvirate/mcp-server
npm install
npm run build
```

Check that `dist/gemini/server.js` exists after building.

---

## What Each Agent Costs in Practice

Real-world Triumvirate usage with the free tiers:

| Operation | Agent | Approximate Cost |
|-----------|-------|-----------------|
| Session start (project picker) | Claude only | Included in Claude subscription |
| Stenographer save | Ollama (local) | **$0.00** |
| Pre-compact summarization | Gemini (free tier) | **$0.00** |
| Gap-fill on compaction | Gemini (free tier) | **$0.00** |
| Spawn Gemini daemon for research | Gemini (free tier) | **$0.00** |
| Spawn Codex daemon for code review | Codex (free credits) | **$0.00** (while credits last) |
| Oracle operations | Gemini (free tier) | **$0.00** |
| File snapshots (The Airlock) | Local filesystem | **$0.00** |

The only ongoing cost is Claude itself. Everything else runs on free tiers or local compute.
