# Batch Claude Deep Research

Automate batch-submission of research topics to claude.ai's Deep Research feature via browser automation.

Write a list of topics in a text or JSON file. Run one command. Walk away — each topic gets its own Deep Research conversation with Research mode enabled automatically.

**Full project:** [github.com/michaeljboscia/claude-deep-research](https://github.com/michaeljboscia/claude-deep-research)

## How it works

- Node.js + Playwright connects to a real Chrome instance via CDP (`--remote-debugging-port=9222`)
- One-time login saves your session to a browser profile
- Batch launch opens each topic as a separate Chrome tab, pastes the prompt, enables Research mode, and submits
- macOS only (Chrome paths and keyboard shortcuts)

## Prerequisites

- Claude Pro or Team account with Deep Research access
- Google Chrome installed
- Node.js 18+

## Quick start

```bash
git clone https://github.com/michaeljboscia/claude-deep-research
cd claude-deep-research
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install

# One-time login
npm run login

# Batch submit topics
node launch-research.js --topics topics/your-topics.txt
```

See the [full README](https://github.com/michaeljboscia/claude-deep-research) for topic file format, options, and troubleshooting.

## Where this fits in Triumvirate

Claude Deep Research (the native feature) and Gemini Deep Research (via `mcp__gemini__gemini-deep-research`) are complementary:

| | Claude DR | Gemini DR |
|--|--|--|
| Access | via this batch launcher (browser) | via MCP tools (API) |
| Best for | Long-form research with citations | Programmatic, pipeable into workflows |
| Output | Saved to claude.ai conversation | Returned directly to Claude |
| Batch | Yes — this tool | Yes — fire multiple via MCP |
