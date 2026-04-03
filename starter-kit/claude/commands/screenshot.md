# Screenshot Analyzer

**Slash Command:** `/screenshot`

**Purpose:** Analyze the most recent Snagit screenshot using Gemini vision (token-efficient — image never enters Claude context)
**Duration:** ~5-10 seconds
**Output:** Screenshot analysis with metadata context

---

## Arguments

- `/screenshot` — Analyze the most recent screenshot
- `/screenshot 2` or `/screenshot 3` — Analyze the 2nd or 3rd most recent screenshot
- `/screenshot visual` — Emphasize visual/layout analysis over text content
- `/screenshot text` — Focus on extracting and presenting text content only (no Gemini call, uses Snagit's built-in OCR)

---

## Execution Steps (Follow these EXACTLY)

### Step 1: Find the target screenshot

Search BOTH screenshot locations and pick the most recent file across all sources:

```bash
# List all screenshots from both locations, sorted by modification time
(ls -t ~/Desktop/screenshots/*.png 2>/dev/null; ls -t ~/Pictures/Snagit/*.snagx 2>/dev/null) | head -5
```

- Default: pick the most recent file (line 1)
- If user passed a number N, pick line N
- Show the filename and timestamp so user knows which capture we're analyzing

### Step 2: Prepare the image for analysis

**If the file is a `.snagx` (Snagit capture):**

The `.snagx` format is a ZIP archive containing PNG images and JSON metadata.

```bash
SCRATCH="/private/tmp/claude-screenshot-work"
mkdir -p "$SCRATCH"
unzip -o "<path_to_snagx>" -d "$SCRATCH/" 2>/dev/null
```

Then:
- Read `$SCRATCH/metadata.json` for rich context (OcrText, WebURL, AppName, WindowName, CaptureDate)
- The PNG is the UUID-named file: `ls "$SCRATCH"/*.png | grep -v thumbnail`
- Set `IMAGE_PATH` to that PNG

**If the file is a `.png` (macOS native screenshot):**

No extraction needed. Set `IMAGE_PATH` to the file directly. No metadata is available.

### Step 3: Analyze based on mode

**If mode is `text` (user passed "text" argument) AND source is .snagx:**
- Skip Gemini. Present the OCR text from metadata.json directly.
- Format nicely and done.

**If mode is `text` AND source is .png:**
- Tell user: "No OCR text available for macOS screenshots. Use default mode for Gemini analysis."

**For all other modes (default or `visual`):**

1. Load the Gemini image analysis tool:
```
ToolSearch: "select:mcp__gemini__gemini-analyze-image"
```

2. Send the PNG to Gemini with an appropriate prompt:
   - **Default mode prompt:** "You are extracting information from a screenshot so a text-only AI assistant can understand what the user is showing it. Be concise and precise. Extract: (1) What app/website/page is shown — name it specifically. (2) ALL text content visible — transcribe it accurately, preserving structure. (3) Any data, numbers, errors, or status indicators. (4) UI state that matters (selected tabs, highlighted items, toggle states, form values). Skip generic UI chrome description. Do NOT narrate or editorialize. Just extract the facts."
   - **Visual mode prompt:** "You are extracting visual/design information from a screenshot for a text-only AI assistant. Focus on: layout structure, visual hierarchy, color usage, spacing, component patterns, responsive behavior, and design decisions. Skip text content unless it's part of the design pattern."

### Step 4: Present results

**IMPORTANT: The purpose of /screenshot is to give Claude "eyes" — the user already sees the screenshot. Do NOT describe what's on screen back to them. Instead, internalize Gemini's extraction and respond as if you can see it yourself.**

- Do NOT use a formatted "SCREENSHOT ANALYSIS" block with headers
- Do NOT narrate what's on screen ("I can see a spreadsheet with...")
- DO respond naturally as if you were shown the image in person
- DO act on the content — ask what they need help with, offer relevant observations, or continue the conversation based on what you now understand
- If the screenshot clearly relates to ongoing work, connect it to that context
- If the screenshot shows an error, immediately help debug it
- If it shows content/data, engage with the substance, not the container

**The user has eyes. You don't. That's the only reason this skill exists.**

### Step 5: Cleanup

```bash
rm -rf /private/tmp/claude-screenshot-work
```

---

## Key Technical Details

- **Primary screenshot folder:** `~/Desktop/screenshots/` (PNG files from macOS screenshots or Snagit exports)
- **Secondary screenshot folder:** `~/Pictures/Snagit/` (Snagit `.snagx` captures)
- **File formats:** `.png` (direct image), `.snagx` (ZIP archive containing `{UUID}.png`, `metadata.json`, `index.json`, `thumbnail.png`)
- **Token efficiency:** Image is sent to Gemini externally. Only text results enter Claude context. ~200-500 tokens vs 2,000-6,000 for native vision.
- **OCR quality:** Snagit's built-in OCR is surprisingly good for text-heavy screenshots (tweets, articles, error messages)
- **Gemini tool:** `mcp__gemini__gemini-analyze-image`

---

## Error Handling

- **No .snagx files found:** Tell the user "No Snagit screenshots found in ~/Pictures/Snagit/"
- **Empty OCR + Gemini fails:** Fall back to native Read tool on the PNG (Tier 3)
- **Corrupt archive:** Tell the user and suggest taking a new screenshot

---

**Last Updated:** 2026-02-04
**Version:** 1.0
**Status:** Production Ready
