# The Plain English Guide to Triumvirate

If you've never used an AI coding tool before, start here. No jargon, no assumptions. Just what this thing does, why it matters, and how it works.

---

## Where This Came From

Everything in this repo exists because someone screwed up first.

The Airlock exists because Claude overwrote a production database config at 2am and there was no backup. The Stenographer exists because the pre-compact summarizer burned 69 million API tokens in four days before anyone noticed. The "persist or fail" skill exists because we lost 5 hours of batch processing results that were sitting in memory and never got saved to disk. The systematic debugging skill exists because Claude kept guessing at fixes, changing 10 files to solve a one-line bug, and breaking 5 other things in the process.

The author and his three AI agents have spent months tripping over things, running face-first into walls, and finding new and creative ways to do things wrong. Every hook, every skill, every safety check in this system was born from a real mistake — usually a painful one.

We released this to try to help other people avoid making the same mistakes we have. The guardrails aren't theoretical. They're scar tissue.

---

## What Is This?

Triumvirate is a system that makes three AI assistants work together on your computer. Instead of you copy-pasting between different AI chat windows, they talk directly to each other, share notes, and remember what you worked on yesterday — or last month.

The three assistants:

- **Claude** is the team lead. This is who you talk to. Claude figures out what needs to happen and either does it or delegates to the right specialist.
- **Gemini** is the researcher. It has an enormous memory — it can hold hundreds of documents at once — and it can search the web. When Claude needs background research, competitive analysis, or needs to understand a massive codebase, it taps Gemini.
- **Codex** is the builder. When something needs to be coded, refactored, or reviewed line-by-line, Codex handles it. It's the one that actually writes and fixes code.

You only talk to Claude. Claude manages the other two behind the scenes.

---

## Why Should I Care?

Most AI tools have three problems:

**1. They forget everything.** Close the tab, start a new conversation, and the AI has no idea what you were working on. You spend the first 10 minutes of every session re-explaining your project.

**2. They work alone.** One AI does everything — research, coding, analysis — and it's mediocre at most of it. There's no way to bring in a specialist.

**3. They can break things.** An AI can overwrite your files, delete your data, or make changes you can't undo. There's no safety net.

Triumvirate fixes all three:

- **It remembers.** A system called the Stenographer quietly takes notes in the background. When you come back tomorrow, Claude reads yesterday's notes and picks up where you left off.
- **It delegates.** Claude brings in Gemini when it needs research or huge context. It brings in Codex when it needs precise code work. You get three specialists instead of one generalist.
- **It protects you.** A system called the Airlock automatically makes a backup of every file before the AI touches it. If something goes wrong, you can always go back.

---

## What Does It Actually Do Day-to-Day?

Here are real scenarios:

**Scenario: Research that carries over**
On Friday, you use Claude to research your competitors. Claude spawns Gemini (the researcher) to dig through websites, PDFs, and market data. Gemini writes its findings into a session log. On Monday, you open Claude and say "write me a strategy brief based on Friday's research." Claude reads Gemini's notes from Friday and produces the brief. You didn't copy-paste anything. You didn't re-explain the project. It just remembered.

**Scenario: Complex project work**
You ask Claude to rebuild your website's checkout page. Claude realizes this is a big coding task, so it dispatches Codex (the builder). Codex looks at the codebase and realizes it needs to understand 50 files to do the job right. Instead of trying to read them all itself, Codex taps Gemini on the shoulder — "hey, read this whole folder and tell me how the payment flow works." Gemini reads everything (it has room for millions of words in its memory), gives Codex a focused briefing, and Codex builds the new checkout page. You just get the finished result.

**Scenario: The 2am save**
It's late and you're letting Claude make changes to your project. Claude accidentally overwrites an important configuration file with something broken. But the Airlock already made a timestamped backup before the edit happened — silently, automatically, without you asking. You restore it in seconds.

**Scenario: Long-running project memory**
You're working on a product launch that spans three weeks. Every day, you work with Claude on different aspects — messaging, technical setup, partner outreach. The Oracle (long-term memory system) holds all the research docs, brand guidelines, and competitive analysis in one place. Three weeks in, you can say "based on everything we've discussed about the competitive landscape, what angles haven't we tried?" and it draws from the entire history.

---

## The Key Concepts (In Plain English)

### The Whiteboard Problem

Every AI has a memory limit. Think of it as a whiteboard. Claude's whiteboard is big. Gemini's is enormous. But they all have edges.

As you work, the whiteboard fills up with conversation history, file contents, and instructions. Eventually it runs out of room. When that happens, the AI has to erase the oldest stuff to make space. This is called **compaction**.

The problem: if the AI erases your morning's work to make room for the afternoon, it forgets what you decided earlier.

**Triumvirate's solution:** Before the whiteboard gets erased, the system takes a photo of it. Actually, several things happen:

1. The **Stenographer** has been quietly writing summaries into a notebook every few minutes (like a court reporter)
2. The **pre-compact hook** asks Gemini to write a final, detailed summary of the whole whiteboard
3. That summary gets saved to a **session log** (the notebook)
4. After the whiteboard is erased, the **recovery hook** reads the notebook back onto the fresh whiteboard

Result: the AI "remembers" across erasures. The whiteboard is temporary, but the notebook is permanent.

> **About the Stenographer:** The Stenographer writes incremental notes every few minutes using a free AI running on your own computer (Ollama). It requires Ollama installed with a model pulled (e.g., `ollama pull qwen2.5:7b` — a 4.4GB download). Without Ollama, session persistence still works through the pre-compact saves (Gemini summarizes at compaction time), but you miss the incremental mid-session notes.

### Hooks: Automatic Habits

Hooks are things that happen automatically when certain events occur. Think of them as habits you've trained into the system.

Some examples of what hooks do:

| When this happens... | This hook automatically... |
|---------------------|--------------------------|
| You start a new session | Finds yesterday's notes and loads them |
| Claude edits any file | Makes a backup copy first (the Airlock) |
| The conversation hits ~50,000 words | Triggers the Stenographer to write a summary |
| The whiteboard is about to be erased | Saves a detailed summary to the notebook |
| After the whiteboard is erased | Reads the summary back in so Claude remembers |
| Claude runs a database command | Checks that you have a recent backup first |

You don't manage hooks. They just run. They're the system's automatic safety habits.

### The Airlock: Your Safety Net

Every time the AI is about to change a file, the Airlock silently makes a copy of the original. No prompts, no questions — it just happens.

Why this matters: AI assistants are confident and fast. Sometimes too fast. They'll rewrite a file without hesitation, and if they get it wrong, you need the original back. The Airlock ensures every change is reversible.

It has three levels of protection:

- **Strict** (for databases): Won't let the AI touch your database unless you've backed it up recently. Actually blocks the action.
- **Best-effort** (for important files): Makes the backup, lets the AI proceed. You have a copy if you need it.
- **Copy** (for everything else): Same — backup made, AI proceeds freely.

### The Oracle: Long-Term Memory

If the whiteboard is short-term memory and session logs are notebooks, the Oracle is a filing cabinet with a dedicated librarian.

You can fill the filing cabinet with documents — research reports, brand guidelines, technical specs, competitive analysis. The librarian (a Gemini assistant) reads everything and answers questions about it. Days, weeks, or months later.

When the filing cabinet gets too full (even Gemini's huge memory has limits), the librarian writes a "story so far" summary. You give the librarian a fresh start but hand them the summary. They're fast again, but still know the plot.

The Oracle is optional. Basic Triumvirate works fine without it. But for long-running projects where you're accumulating lots of reference material, it's powerful.

### Skills: Guardrails and Discipline

Skills are rules that the AI follows — like SOPs (standard operating procedures) for an employee.

Without skills, AI assistants make the same mistakes repeatedly:
- They guess at problems instead of diagnosing them
- They claim code exists without checking
- They forget to save important results
- They rebuild things that already exist
- They skip documentation

Skills prevent this. Each skill is a set of rules loaded into the AI's instructions when the situation calls for it. For example:

- **"Systematic debugging"** forces the AI to prove the root cause before attempting a fix, instead of randomly changing things and hoping
- **"Persist or fail"** makes the AI save results immediately instead of holding them in memory where they can be lost
- **"Context before action"** forces the AI to actually check if something exists before declaring it does or doesn't

You don't invoke skills manually. The AI recognizes when a skill applies and loads it.

### Session Logs: The Notebooks

Session logs are the notebooks where everything gets written down. They're plain text files (Markdown) stored in a private folder on your computer.

Every AI writes to the same folder, in a compatible format. This means:

- Claude can read what Gemini researched yesterday
- Codex can read what Claude decided last week
- You can search them with simple text search

They're also stored in git (a version-tracking system), so you can see the complete history of every session you've ever had — when it happened, what was discussed, what decisions were made.

When you start a new session, the system automatically finds the most recent notes and loads them. The AI reads them and picks up where you left off.

### Git: The Time Machine

Git is a system that tracks every change to your files over time. Think of it as a time machine — you can go back to any previous version of any file.

Triumvirate uses git in two places:

**Your project folder:** Every time the AI edits a file, the system automatically stages it (puts it in the "ready to save" box). When you're happy with the changes, you tell Claude to commit (take a permanent snapshot). This means every change is tracked and reversible.

**The memory folder:** Session logs are automatically committed before the whiteboard gets erased. This creates a permanent record of every AI session.

Important: the AI never pushes your code to the internet without asking. Staging and committing happen locally on your computer. Pushing (sharing with the world) is always a manual, explicit action.

---

## What Do I Need to Get Started?

### The Three AIs

| AI | Cost | How to Get It |
|----|------|---------------|
| Claude | Paid (Pro subscription $20/mo, or API) | [claude.ai](https://claude.ai) |
| Gemini | **Free** | Google AI Studio key — no credit card needed |
| Codex | **Free** | OpenAI account — free credits for new signups |

See `docs/getting-api-access.md` for step-by-step instructions on getting each one set up.

### The Tools

You'll need these installed on your computer:
- **Node.js** (version 20 or newer) — the runtime that powers the system
- **git** — the time machine for your files
- **jq** — a small utility for reading config files
- **Ollama** (optional) — the free, local AI that powers the Stenographer

### Installation

```bash
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit
chmod +x install.sh
./install.sh
```

The installer takes about 2 minutes. It:
1. Installs all the automatic habits (hooks)
2. Installs the operating rules (skills)
3. Builds the communication system between the three AIs
4. Creates the notebook folder for session logs
5. Gives you templates for your API keys

After installation, set up your credentials:
```bash
cp ~/.claude/.env.example ~/.claude/.env
# Open ~/.claude/.env in a text editor and add your API keys
```

Then just start Claude:
```bash
claude
```

That's it. The hooks, skills, Stenographer, and Airlock all run automatically from the first session.

---

## Do I Need to Understand All This?

No. You can use Triumvirate by just opening Claude and telling it what you want. Everything described in this guide happens automatically in the background.

But knowing what's happening helps in a few situations:

- **When Claude mentions "session logs"** — it's talking about the notebooks. You can read them yourself in `~/.ai-memory/`.
- **When Claude says "spawning a Gemini daemon"** — it's calling in the researcher. Let it do its thing.
- **When you see "Airlock blocked"** — the safety net caught something. The AI tried to change a file without a recent backup. Take the backup and try again.
- **When Claude mentions "compaction"** — the whiteboard is being erased. The system already saved everything important. Claude will recover automatically.

If you want to go deeper into any component, the technical docs are in the `docs/` folder:

| Doc | What It Covers |
|-----|---------------|
| `how-it-all-fits-together.md` | Technical deep-dive into all four layers |
| `oracle-engine.md` | Everything about the long-term memory system |
| `configuration-reference.md` | Every setting, file, and variable you can configure |
| `git-workflow.md` | How the time machine works under the hood |
| `getting-api-access.md` | Step-by-step API key setup for all three AIs |

---

## Glossary

Quick reference for terms you might see in Claude's output or in the docs:

| Term | What It Means |
|------|--------------|
| **Daemon** | A background assistant that stays running. You can ask it questions, and it remembers the conversation. |
| **Spawn** | Start up an assistant. "Spawn a Gemini daemon" = "bring in the researcher." |
| **Dismiss** | Send the assistant home. "Soft dismiss" = they keep their desk for next time. "Hard dismiss" = clean slate. |
| **Context window** | The whiteboard — how much the AI can hold in short-term memory at once. |
| **Tokens** | Words (roughly). 1,000 tokens is about 750 words. |
| **Compaction** | Erasing the whiteboard to make room. The system saves a summary first. |
| **Hook** | An automatic habit that fires when something happens (file edited, session started, etc.) |
| **The Airlock** | Automatic file backup before every edit. Your undo button. |
| **Stenographer** | The background note-taker that summarizes your session every few minutes. Runs on local Ollama, costs nothing. Requires `ollama pull qwen2.5:7b` to set up. |
| **Oracle** | Long-term memory. A filing cabinet with a librarian. Remembers across weeks and months. |
| **Corpus** | The set of documents loaded into an Oracle's filing cabinet. |
| **Checkpoint** | A "story so far" summary that the Oracle writes so it can get a fresh start without losing knowledge. |
| **Session log** | The notebook. A text file recording what happened in a session. All three AIs share them. |
| **Skill** | A set of operating rules loaded into the AI for specific situations. Like an SOP for an employee. |
| **MCP** | The communication system that lets the three AIs talk to each other. You never interact with it directly. |
| **Git** | The time machine for your files. Tracks every change so you can go back. |
| **Repo** | Short for "repository" — just your project folder, tracked by git. |
| **Commit** | Taking a permanent snapshot of your files at a point in time. |
| **Staging** | Putting files in the "ready to snapshot" box. Happens automatically. |
| **Push** | Uploading your snapshots to the internet (GitHub). Never happens without your permission. |
| **Taxonomy** | An identity card for your project — who owns it, what it's for, what feature you're working on. Used for naming session logs. |
| **Model fallback** | When one AI model runs out of free quota, the system automatically switches to another available model so your work doesn't stop. |

---

## Questions?

If anything in this guide doesn't make sense, or you hit a wall during setup, please reach out:

- **GitHub Issues** (preferred): [github.com/michaeljboscia/triumvirate/issues](https://github.com/michaeljboscia/triumvirate/issues)
- **LinkedIn:** [linkedin.com/in/michaeljboscia](https://linkedin.com/in/michaeljboscia)

There are no dumb questions, only undocumented features.
