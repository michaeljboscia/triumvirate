use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};
use shared_types::AgentStreamEvent;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct CodexExecParser {
    thread_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    stream_tx: Option<mpsc::Sender<AgentStreamEvent>>,
    stream_seq: u64,
}

/// Strip codex's shell wrapper, if present.
///
/// codex does not report `cat /repo/a.rs`. It reports
/// `/bin/zsh -lc 'cat /repo/a.rs'`, verified live on 2026-09-01: the first version of the
/// classifier assumed the bare form, saw the program as `zsh`, and classified every read as
/// Bash. The offline tests all passed because they used the shape I imagined rather than the
/// shape codex emits. Only the live test caught it.
///
/// Returns the inner command when the outer one is a recognised shell invoked with `-c`
/// (including `-lc`, `-lic`), and the input unchanged otherwise. A shell whose script is not
/// a single quoted string is left alone, so anything unusual falls through to the caller's
/// fail-closed checks.
fn unwrap_shell_wrapper(cmd: &str) -> &str {
    const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];
    let mut it = cmd.split_whitespace();
    let Some(program) = it.next() else {
        return cmd;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    if !SHELLS.contains(&base) {
        return cmd;
    }
    // The flag bundle must be a -c form: -c, -lc, -lic, -ic.
    let Some(flag) = it.next() else {
        return cmd;
    };
    if !(flag.starts_with('-') && flag.ends_with('c')) {
        return cmd;
    }
    let rest = cmd[cmd.find(flag).map(|i| i + flag.len()).unwrap_or(cmd.len())..].trim();
    // Only a single fully-quoted script is unwrapped; anything else stays as-is and is then
    // rejected by the caller's compound/redirection checks.
    for q in ['\'', '"'] {
        if rest.starts_with(q) && rest.ends_with(q) && rest.len() >= 2 {
            let inner = &rest[1..rest.len() - 1];
            if !inner.contains(q) {
                return inner;
            }
        }
    }
    cmd
}

/// Does this shell command READ file contents, as opposed to merely naming a path?
///
/// `codex exec` reports every action as `command_execution` with `ToolKind::Bash`, so on that
/// backend "opened the file" and "ran a command mentioning the file" were the same record. That
/// made `required_sources` unenforceable for codex, the peer most likely to be reviewing code,
/// and the sight gate refused named sources there rather than fake them.
///
/// This narrows it honestly. A CONSERVATIVE ALLOWLIST of programs whose job is to emit file
/// contents. Anything unlisted stays `Bash` and still cannot satisfy a source, so an unknown
/// command fails closed.
///
/// Deliberately excluded, and worth stating: `ls`, `find`, `stat`, `file` and `mdfind` name
/// paths without reading them, which is the exact hole that let a search satisfy a source on
/// the agy backend. `grep` and `rg` are excluded TOO, for the pattern-position reason in the
/// body: `rg needle a.rs` reads a.rs while the boundary matcher counted `needle` as opened.
///
/// A compound or piped command is NOT classified, because the reader may not be the part that
/// touched the named path. Any redirection disqualifies, and `sed -i` writes.
/// Programs that put the WHOLE file in front of the model, as opposed to a slice of it.
///
/// FIND-REVIEW-07. Grok found the hole in round 3 and it is the same class it named in
/// FIND-REVIEW-06, moved to the other end of the file. Putting the proof-of-read nonce on the
/// LAST line turned `head -1` into `tail -1`, and `tail` sits in the reader list right next to
/// `head`. One command satisfies the sight gate and returns the nonce, and the work under
/// review never enters the model's context.
///
/// So a source that must be READ is now only satisfied by a program that emits all of it.
///
/// `head`, `tail` and `cut` are readers (they do put file contents in front of the model, which
/// is why they stay in READERS and still count as a read for the no-touch check) but they are
/// PARTIAL: a slice of lines or a slice of columns. `more` and `less` are pagers; non-interactively
/// they dump everything, but that depends on the terminal and on `$PAGER`, so they are treated as
/// partial rather than reasoned about. Fail closed: the cost is a false rejection of an unusual
/// full read, never a false pass on a one-line peek.
const WHOLE_FILE_READERS: &[&str] = &["cat", "nl", "bat", "od", "xxd", "strings", "pr", "zcat"];

/// The ONE file a whole-file shell read opened, or None.
///
/// The sight gate used to match a named source as a substring of the recorded arguments. That
/// passed `cat /dev/null < /repo/a.rs`, `cat /repo/b.rs # /repo/a.rs`, and a grok call whose
/// `description` field named the source while `command` read something else (Antigravity,
/// review of D-010, 2026-09-13). A whole-file read is now bound to its operand: exactly one
/// reader, any flags, exactly one file, and no pipe, redirect, comment, subshell, or chain.
/// Ranged reads are bound separately by `command_read_range`, which already checks the operand.
pub fn whole_file_read_operand(command: &str) -> Option<String> {
    if command_read_range(command).is_some() {
        return None;
    }
    let cmd = unwrap_shell_wrapper(command.trim());
    if cmd.contains(['|', '<', '>', '#', ';', '&', '`', '$', '(', ')']) {
        return None;
    }
    // Only `cat` and `nl`, with the line-preserving flags the ranged-read parser already vets.
    // The wider reader list (`od`, `xxd`, `strings`, ...) is fine for "did it read something"
    // but not for "did it show the whole file": `xxd -l0 SRC` and `od -N0 SRC` are readers that
    // print nothing (Antigravity, second pass on D-010). Any flag outside the vetted list fails.
    whole_file_reader_with_one_operand(cmd).map(|(op, _)| op)
}

/// The segments of an `&&` chain, quotes honored. A command with no `&&` is one segment.
///
/// D-017: codex read a 97-line brief with `wc -l F && sed -n '1,240p' F`, which reads every
/// line of it, and the gate rejected the turn as "never opened". Both read parsers refuse any
/// command containing `&&`, because that refusal is what closed the D-010 decoy attacks. The
/// refusal is right about the ATTACKS and wrong about the CHAIN: each link is its own command.
///
/// ONLY `&&`, deliberately. The gate counts a call only when the tool reported success, and an
/// `&&` chain exits zero only if every link ran and succeeded, so each link's read really
/// happened. `;` and `||` both mask a failure (`cat missing ; true`, `cat missing || true`
/// exit zero having read nothing), so a command containing either is still refused whole.
///
/// Splitting changes only which text each parser sees. Every segment goes through the same
/// unchanged parser, which still refuses a redirect, a comment, a subshell, a backtick, a
/// single `&`, and a second operand, so no D-010 shape survives the split.
pub fn and_chain_segments(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let (mut out, mut start, mut i) = (Vec::new(), 0usize, 0usize);
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => {
                if b == b'\'' || b == b'"' {
                    quote = Some(b);
                } else if b == b'&' && bytes.get(i + 1) == Some(&b'&') {
                    out.push(command[start..i].trim());
                    i += 2;
                    start = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    // An unterminated quote is a command we do not understand; refuse to split it.
    if quote.is_some() {
        return vec![command.trim()];
    }
    out.push(command[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

/// Every whole-file read in a command, one per `&&` segment.
pub fn whole_file_read_operands(command: &str) -> Vec<String> {
    and_chain_segments(unwrap_shell_wrapper(command.trim()))
        .into_iter()
        .filter_map(whole_file_read_operand)
        .collect()
}

/// Every ranged read in a command, one per `&&` segment. A chain that walks a file in windows
/// (`sed -n '1,200p' F && sed -n '201,400p' F`) yields both, so the gate can union them.
pub fn command_read_ranges(command: &str) -> Vec<RangedRead> {
    and_chain_segments(unwrap_shell_wrapper(command.trim()))
        .into_iter()
        .filter_map(command_read_range)
        .collect()
}

/// A shell tool call that READS a file is a `ReadFile` for the sight gate.
///
/// Every adapter mapped its shell tool to `Bash` by name alone, so a `cat` of a named source
/// was invisible to the gate while the gate's own rejection text told the reviewer to `cat`
/// the file (D-010). This is applied by every adapter after its name-based mapping. The
/// command is read from `command` (grok, claude, gemini, codex) or `CommandLine` (agy).
pub fn shell_read_kind(kind: ToolKind, args: Option<&serde_json::Value>) -> ToolKind {
    if kind != ToolKind::Bash {
        return kind;
    }
    match shell_command_from_value(args) {
        Some(cmd) if command_reads_file_contents(&cmd) => ToolKind::ReadFile,
        _ => kind,
    }
}

/// The command string inside a shell tool's recorded arguments, whichever key the adapter uses.
pub fn shell_command_from_value(args: Option<&serde_json::Value>) -> Option<String> {
    let v = args?;
    v.get("command")
        .or_else(|| v.get("CommandLine"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Same, from the JSON string a `ToolCallRecord` carries.
pub fn shell_command_from_args_json(args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    shell_command_from_value(Some(&v))
}

pub fn command_reads_whole_file(command: &str) -> bool {
    // Per `&&` segment (D-017): `wc -l F && cat F` took `wc` as the program and reported a
    // partial read of a command that shows the whole file.
    and_chain_segments(unwrap_shell_wrapper(command.trim()))
        .into_iter()
        .any(segment_reads_whole_file)
}

fn segment_reads_whole_file(command: &str) -> bool {
    if !command_reads_file_contents(command) {
        return false;
    }
    // A ranged read is partial by definition, whatever program feeds it. Grok found that
    // without this line `nl FILE | sed -n '161,360p'` took the first token, saw `nl`, and was
    // a WHOLE read, so the coverage union never ran and 200 lines satisfied a source.
    if command_read_range(command).is_some() {
        return false;
    }
    let cmd = unwrap_shell_wrapper(command.trim());
    let Some(program) = cmd.split_whitespace().next() else {
        return false;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    WHOLE_FILE_READERS.contains(&base)
}

/// The line window a RANGED read put in front of the model, when `command` is one of the two
/// shapes codex actually uses to read a file in pieces. `end == None` means "to end of file".
///
/// Captured live on 2026-09-03 from `codex exec --json` (codex-cli 0.145.0) asked to review a
/// 556-line file:
///
///   /bin/zsh -lc "sed -n '1,260p' /repo/a.rs"
///   /bin/zsh -lc "sed -n '261,556p' /repo/a.rs"
///   /bin/zsh -lc "nl -ba /repo/a.rs | sed -n '161,360p'"
///
/// Eight reads on that turn, and not one was `cat`. Every source-gated codex review that day
/// was rejected "never opened" because `sed` was not a reader and a pipe fails closed.
///
/// What is accepted, and nothing wider:
///
///   sed -n 'N,Mp' FILE            one flag, exactly `-n`; one script, exactly a line range
///   sed -n 'N,$p' FILE            ending in `p`; exactly one operand after it.
///   sed -n 'Np'   FILE
///   READER [flags] FILE | sed -n 'N,Mp'
///                                 stage 1 is a whole-file reader with exactly one operand;
///                                 stage 2 is the same sed form with NO operand.
///
/// `sed` cannot write under that shape: `-i` is not `-n`, `w` is not `p`, and a script that is
/// only digits, a comma, `$` and `p` has no room for a command. Any other flag, a second
/// operand, `-e`, `--expression`, `2>`, `&&`, a third pipeline stage: all `None`.
///
/// A window is a PARTIAL read on its own. `agent_exec::codex_ranged_reads_cover_source` unions
/// the windows a turn read against the file's real line count, so `1,260` plus `261,556` on a
/// 556-line file is a whole read and `1,260` alone is the "only read PART" rejection it should
/// be. That is what keeps Grok's `tail -1` attack closed: a window that stops short of the last
/// line does not cover the file, whatever it contains.
pub fn command_read_range(command: &str) -> Option<RangedRead> {
    let cmd = unwrap_shell_wrapper(command.trim());
    if cmd.is_empty()
        || cmd.contains("&&")
        || cmd.contains("||")
        || cmd.contains(';')
        || cmd.contains('&')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('`')
        || cmd.contains("$(")
        || cmd.contains("$'")
    {
        return None;
    }
    let stages: Vec<&str> = cmd.split('|').map(str::trim).collect();
    match stages.as_slice() {
        [single] => {
            let (range, operand) = sed_range_stage(single, true)?;
            Some(RangedRead { range, operand: operand?, via_nl: false })
        }
        [reader, filter] => {
            let (operand, via_nl) = whole_file_reader_with_one_operand(reader)?;
            let (range, _) = sed_range_stage(filter, false)?;
            Some(RangedRead { range, operand, via_nl })
        }
        _ => None,
    }
}

/// A ranged read, with the ONE operand it read, so the gate can bind the window to the file
/// rather than to a path string that happens to appear somewhere in the command. Codex found
/// on review that matching the whole command let a decoy operand embedding the source path
/// collect the source's coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangedRead {
    pub range: LineRange,
    /// The file operand, surrounding quotes stripped, otherwise as written.
    pub operand: String,
    /// True when the window was taken over `nl` output. `nl` omits logical-page delimiter
    /// lines (`\:`, `\:\:`, `\:\:\:` alone on a line), so on a file containing one the window
    /// is over fewer lines than the source has. The gate checks the file for that.
    pub via_nl: bool,
}

fn strip_quotes(tok: &str) -> &str {
    for q in ['\'', '"'] {
        if let Some(inner) = tok.strip_prefix(q).and_then(|t| t.strip_suffix(q)) {
            return inner;
        }
    }
    tok
}

/// A line window, 1-based and inclusive, as `sed` numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: u64,
    /// `None` is `$`: to the end of the file.
    pub end: Option<u64>,
}

/// `sed -n 'SCRIPT' [FILE]`, and nothing else. `with_operand` says whether exactly one FILE must
/// follow the script (the bare form) or none may (the pipeline filter form). Returns the window
/// and, in the bare form, the operand.
fn sed_range_stage(stage: &str, with_operand: bool) -> Option<(LineRange, Option<String>)> {
    let toks: Vec<&str> = stage.split_whitespace().collect();
    let expected = if with_operand { 4 } else { 3 };
    if toks.len() != expected {
        return None;
    }
    let base = toks[0].rsplit('/').next().unwrap_or(toks[0]);
    if base != "sed" || toks[1] != "-n" {
        return None;
    }
    let operand = if with_operand {
        let op = strip_quotes(toks[3]);
        if op.starts_with('-') || op.is_empty() {
            return None;
        }
        Some(op.to_string())
    } else {
        None
    };
    Some((parse_sed_print_range(toks[2])?, operand))
}

/// `'N,Mp'`, `"N,$p"`, `Np`, with or without the quotes, and NOTHING else.
fn parse_sed_print_range(script: &str) -> Option<LineRange> {
    let inner = script
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| script.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(script);
    let body = inner.strip_suffix('p')?;
    let (start, end) = match body.split_once(',') {
        Some((a, "$")) => (a, None),
        Some((a, b)) => (a, Some(b)),
        None => (body, Some(body)),
    };
    let start: u64 = start.parse().ok().filter(|n| *n > 0)?;
    let end = match end {
        Some(e) => Some(e.parse::<u64>().ok().filter(|n| *n >= start)?),
        None => None,
    };
    Some(LineRange { start, end })
}

/// `cat FILE`, `nl -ba FILE`: a LINE-PRESERVING whole-file reader, exactly one operand. The
/// operand is what the gate matches the named source against, so there must be only one.
///
/// Line-preserving is the whole requirement, because the `sed -n 'N,Mp'` after the pipe
/// numbers OUTPUT lines and the coverage check numbers SOURCE lines. Codex found on review
/// that `pr` (paginates, inserts headers) and `bat` (decorations, wrapping, depending on
/// config) were on the first version of this list, so `pr FILE | sed -n '1,9999p'` clamped to
/// the source line count and passed while source lines never reached the model. Now only
/// `nl` and `cat`, each with an allowlist of flags that cannot change the line count AND
/// exist on both macOS and GNU. Returns the one operand (quotes stripped) and whether the
/// reader is `nl`.
///
/// `cat -s` squeezes blank lines. `nl -s SEP` puts SEP after every number, and Grok showed SEP
/// can be a newline (`-s$'\n'`), which doubles the output lines; `-d`, `-h`, `-f`, `-l`, `-p`,
/// `-v` and `-i` change numbering or sectioning and are simply not needed to read a file.
///
/// Portability is a gate property, not a nicety. Grok's round 3: `nl -w0` and `cat -A` both
/// ERROR on this host, and the live wrapper is `/bin/zsh -lc` without `pipefail`, so the
/// pipeline's exit code is sed's 0 on empty stdin. The record says success, the model saw
/// nothing, and a `1,$p` window would have covered the file. So `-w` must be a positive number
/// and only the flags BSD cat and GNU cat share are accepted.
fn whole_file_reader_with_one_operand(stage: &str) -> Option<(String, bool)> {
    const CAT_PORTABLE_LINE_PRESERVING_FLAGS: &[&str] = &["-n", "-b", "-v", "-e", "-t"];
    // `-ba` / `-bt` / `-bn` (which lines get numbers) and `-w<digits>`, width, at least 1.
    fn nl_flag_is_line_preserving(tok: &str) -> bool {
        matches!(tok, "-ba" | "-bt" | "-bn")
            || tok
                .strip_prefix("-w")
                .is_some_and(|d| d.parse::<u32>().is_ok_and(|n| n > 0))
    }
    let mut toks = stage.split_whitespace();
    let program = toks.next()?;
    let base = program.rsplit('/').next().unwrap_or(program);
    if base != "cat" && base != "nl" {
        return None;
    }
    let mut operand: Option<String> = None;
    for tok in toks {
        // A bare `-` is stdin, which is a second input the window would then be over. Codex
        // found this on the live review of this very function: `cat - FILE | sed -n` counted
        // as one operand because `-` starts with a dash.
        if tok == "-" {
            return None;
        }
        if tok.starts_with('-') {
            let ok = if base == "cat" {
                CAT_PORTABLE_LINE_PRESERVING_FLAGS.contains(&tok)
            } else {
                nl_flag_is_line_preserving(tok)
            };
            if !ok {
                return None;
            }
        } else {
            if operand.is_some() {
                return None;
            }
            let op = strip_quotes(tok);
            if op.is_empty() || op.starts_with('-') {
                return None;
            }
            operand = Some(op.to_string());
        }
    }
    operand.map(|op| (op, base == "nl"))
}

/// Public face of `command_reads_file_contents` for the gate's peek binding.
pub fn command_reads_file_contents_pub(command: &str) -> bool {
    command_reads_file_contents(command)
}

pub(crate) fn command_reads_file_contents(command: &str) -> bool {
    // Per `&&` segment (D-017). This is the THIRD parser with the same blanket refusal, and
    // the one that actually decided the live rejections: it runs in `shell_read_kind`, so a
    // chained read stayed `ToolKind::Bash`, and the gate's coverage check filters on
    // `ToolKind::ReadFile` before any of the other parsers are consulted. Fixing the two
    // downstream parsers changed nothing while this one classified the call out of the set.
    and_chain_segments(unwrap_shell_wrapper(command.trim()))
        .into_iter()
        .any(segment_reads_file_contents)
}

fn segment_reads_file_contents(command: &str) -> bool {
    // A ranged read is a read. Checked first because its pipeline form would otherwise be
    // refused by the compound-command rule below, which exists for commands where the reader
    // may not be the part that touched the named path. Here both stages are constrained.
    if command_read_range(command).is_some() {
        return true;
    }
    // ONLY programs that emit FILE CONTENTS to the model.
    //
    // The first version included `grep`, `rg`, `wc`, `shasum`, `diff`, `cmp`, `awk`, `sed`,
    // `jq` and `yq`, and every one of them was a hole:
    //
    //   `rg /repo/required.rs /repo/other.rs`  reads other.rs; required.rs is the PATTERN, and
    //                                          the boundary matcher counted it as opened.
    //   `wc /repo/a.rs`                        shows the model a COUNT, not the file.
    //   `shasum`, `cmp`                        same: a summary, not contents.
    //   `yq -i`, `awk -i inplace`, `perl -i`   MUTATE while classified as reads. Only `sed -i`
    //                                          was checked, and `sed -ix` slipped past that.
    //
    // Codex found the pattern-position hole, Antigravity found the in-place-mutation bypass,
    // and Grok named the principle that fixes all of them: a read is a program that puts the
    // file's CONTENTS in front of the model. Anything else is a search or a summary, and
    // `sight_21` already forbids a search from satisfying a source on the other backends.
    //
    // The doc comment on this function used to say "`grep` and `rg` ARE included: they read the
    // file to match against it." That was FALSE: the array below does not contain them, and the
    // list above records why they were taken out. Grok caught the stale sentence in round 3.
    // In this repo a wrong comment is a defect, because it is what the next reader trusts.
    //
    // Unknown programs stay Bash and fail closed, so the cost of being strict here is a false
    // REJECTION of an unusual reader, never a false pass.
    const READERS: &[&str] = &[
        "cat", "head", "tail", "nl", "bat", "od", "xxd", "strings", "cut", "pr", "zcat", "more",
        "less",
    ];
    let cmd = unwrap_shell_wrapper(command.trim());
    if cmd.is_empty() {
        return false;
    }
    if cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') || cmd.contains('|')
        || cmd.contains('&')
    {
        return false;
    }
    if cmd.contains('>') {
        return false;
    }
    let Some(program) = cmd.split_whitespace().next() else {
        return false;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    // Belt and braces: no in-place flag may ever ride a reader, whatever gets added later.
    // `sed -ix` and `sed --in-place` both slipped past the old `" -i"` substring check.
    for tok in cmd.split_whitespace().skip(1) {
        if tok == "-i" || tok.starts_with("-i") && tok.len() <= 4 || tok.starts_with("--in-place")
        {
            return false;
        }
    }
    READERS.contains(&base)
}

impl CodexExecParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self {
        Self {
            stream_tx: Some(tx),
            ..Self::default()
        }
    }

    fn emit_stream_event(&mut self, event: AgentStreamEvent) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.try_send(event);
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.stream_seq += 1;
        self.stream_seq
    }

    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;
        let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match event_type {
            "thread.started" => {
                self.thread_id = json.get("thread_id").and_then(|v| v.as_str()).map(ToString::to_string);
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "thread started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "turn.started" => {
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "turn started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "item.started" => self.parse_item_event(&json, true),
            "item.completed" => self.parse_item_event(&json, false),
            "turn.completed" => {
                let usage = json.get("usage").cloned().unwrap_or_default();
                let token_usage = TokenUsage {
                    input: usage.get("input_tokens").and_then(|v| v.as_u64()),
                    output: usage.get("output_tokens").and_then(|v| v.as_u64()),
                    cached: usage.get("cached_input_tokens").and_then(|v| v.as_u64()),
                    // 0.145 reports reasoning tokens separately as `reasoning_output_tokens`;
                    // map to thinking_tokens (already emitted as tv_thinking_tokens). Previously
                    // dropped, so codex reasoning volume went uncounted.
                    thinking_tokens: usage.get("reasoning_output_tokens").and_then(|v| v.as_u64()),
                    latency_ms: None,
                    tool_calls: None,
                    total: None,
                };
                self.token_usage = Some(token_usage.clone());
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnCompleted,
                    detail: "turn completed".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: Some(token_usage.clone()),
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnCompleted {
                    agent: "codex".into(),
                    tokens_in: token_usage.input.unwrap_or(0) as i64,
                    tokens_out: token_usage.output.unwrap_or(0) as i64,
                    cached_tokens: token_usage.cached.map(|c| c as i64),
                    tool_count: self.tool_calls.len() as i64,
                    duration_ms: token_usage.latency_ms.unwrap_or(0),
                    seq,
                });
                Some(event)
            }
            "error" => {
                let detail = json
                    .get("message")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("codex error")
                    .to_string();
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::Error,
                    detail: detail.clone(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::Error {
                    agent: "codex".into(),
                    message: detail,
                    seq,
                });
                Some(event)
            }
            _ => {
                tracing::debug!("unknown codex event type: {event_type}");
                None
            }
        }
    }

    fn parse_item_event(&mut self, json: &serde_json::Value, started: bool) -> Option<WorkingStateEvent> {
        let item = json.get("item")?;
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match item_type {
            "agent_message" => {
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                if !text.is_empty() {
                    self.response_chunks.push(text.to_string());
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::MessageDelta,
                    detail: "assistant response chunk".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
            "command_execution" => {
                let command = item.get("command").and_then(|v| v.as_str()).unwrap_or_default();
                if started {
                    self.tool_calls.push(ToolCallRecord {
                        id: item.get("id").and_then(|v| v.as_str()).map(ToString::to_string),
                        tool: "command_execution".to_string(),
                        // A pure content reader is classified as a READ so codex can satisfy
                        // `required_sources`. Everything else stays Bash and cannot.
                        kind: if command_reads_file_contents(command) {
                            ToolKind::ReadFile
                        } else {
                            ToolKind::Bash
                        },
                        success: None,
                        duration_ms: None,
                        args_json: Some(serde_json::json!({"command": command}).to_string()),
                    });
                } else {
                    let id = item.get("id").and_then(|v| v.as_str());
                    let exit_code = item.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if let Some(id) = id
                        && let Some(existing) = self.tool_calls.iter_mut().find(|r| r.id.as_deref() == Some(id))
                    {
                        existing.success = Some(exit_code == 0);
                    }
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: if started {
                        WorkingState::CommandStarted
                    } else {
                        WorkingState::CommandCompleted
                    },
                    detail: if started {
                        "running command".to_string()
                    } else {
                        "command completed".to_string()
                    },
                    tool_name: Some("command_execution".to_string()),
                    tool_args_json: Some(serde_json::json!({"command": command}).to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                if started {
                    let seq = self.next_seq();
                    self.emit_stream_event(AgentStreamEvent::ToolCall {
                        agent: "codex".into(),
                        tool_name: "bash".into(),
                        args_summary: command.to_string(),
                        seq,
                    });
                }
                Some(event)
            }
            _ => {
                let state = if started {
                    WorkingState::ToolCallStarted
                } else {
                    WorkingState::ToolCallCompleted
                };
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state,
                    detail: format!("{} {}", if started { "started" } else { "completed" }, item_type),
                    tool_name: Some(item_type.to_string()),
                    tool_args_json: Some(item.to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.join("\n"),
            session_id: self.thread_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            self_reported_cost_usd: None,
            cli_version: None,
            parser_mode: "codex-exec-json".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_golden_trace() {
        let mut parser = CodexExecParser::new();
        let raw = include_str!("../../../tests/fixtures/codex-exec-trace.jsonl");
        let mut parsed = 0;
        for line in raw.lines() {
            if parser.parse_line(line).is_some() {
                parsed += 1;
            }
        }
        let result = parser.finish();
        assert!(parsed >= 5);
        assert_eq!(
            result.session_id.as_deref(),
            Some("019d626f-c562-7a43-b388-f48c1d9b8dc8")
        );
        assert!(result.response_text.contains("8 crates"));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.output), Some(157));
    }
}

#[cfg(test)]
mod command_classification_tests {
    use super::*;

    /// Programs that emit file contents are reads, so codex can satisfy `required_sources`.
    /// RED IF: the reader allowlist is emptied, which returns codex to being unable to
    /// source-gate at all.
    #[test]
    fn codex_01_content_readers_are_reads() {
        for c in [
            "cat crates/foo/src/lib.rs",
            "head -n 50 /repo/a.rs",
            "/usr/bin/cat /repo/a.rs",
            "bat /repo/a.rs",
            "cut -d, -f1 /repo/a.rs",
        ] {
            assert!(command_reads_file_contents(c), "should be a read: {c}");
        }
    }

    /// THE SHAPE CODEX ACTUALLY EMITS, captured live: a shell wrapper around the real command.
    ///
    /// The first classifier assumed a bare `cat /repo/a.rs`. codex emits
    /// `/bin/zsh -lc 'cat /repo/a.rs'`, so the program looked like `zsh` and every read was
    /// classified Bash. Every offline test passed because they all used the shape I imagined.
    /// Only the live test caught it.
    ///
    /// RED IF: the shell unwrapper is removed, which silently returns codex to being
    /// unable to satisfy a named source.
    #[test]
    fn codex_05_the_shell_wrapper_codex_actually_emits_is_unwrapped() {
        assert!(
            command_reads_file_contents("/bin/zsh -lc 'cat /repo/a.rs'"),
            "this is the literal shape from a live codex turn"
        );
        assert!(command_reads_file_contents("bash -c \"head -n 5 /repo/a.rs\""));
        // A search inside the wrapper is still not a read.
        assert!(!command_reads_file_contents("/bin/zsh -lc 'rg needle /repo/a.rs'"));
        assert!(command_reads_file_contents("/bin/sh -lic 'tail -n 5 /repo/a.rs'"));
        // The wrapper must not launder a non-reader.
        assert!(!command_reads_file_contents("/bin/zsh -lc 'ls /repo'"));
        // A compound script IS unwrapped and split, and its reader classifies it (D-017). The
        // operand binding, not the refusal, is what keeps the decoy out; see codex_03b.
        assert!(command_reads_file_contents("/bin/zsh -lc 'ls /repo && cat /repo/a.rs'"));
        assert!(!command_reads_file_contents("/bin/zsh -lc 'ls /repo ; cat /repo/a.rs'"));
        // A shell without a -c form is left alone and fails closed.
        assert!(!command_reads_file_contents("/bin/zsh script.sh"));
    }

    /// Naming a path is not reading it. This is the hole that let a search satisfy a source on
    /// the agy backend, and it must not reopen on codex.
    /// RED IF: ls, find, stat or mdfind are added to the reader allowlist.
    #[test]
    fn codex_02_naming_a_path_is_not_reading_it() {
        for c in [
            "ls -la /repo/a.rs",
            "find . -name a.rs",
            "stat /repo/a.rs",
            "file /repo/a.rs",
            "mdfind a.rs",
            "test -f /repo/a.rs",
            // SEARCHES and SUMMARIES. None of these put the file's contents in front of the
            // model, and the first two let the PATH be the search pattern rather than the file
            // operand: `rg /repo/required.rs /repo/other.rs` reads other.rs.
            "rg needle /repo/a.rs",
            "grep -n fn /repo/a.rs",
            "rg /repo/required.rs /repo/other.rs",
            "wc /repo/a.rs",
            "shasum /repo/a.rs",
            "diff /repo/a.rs /repo/b.rs",
            // PROGRAMMABLE, and all have in-place flags. `sed -n 'N,Mp' FILE` is the one
            // exception, as a ranged read under a shape that cannot write; see codex_06.
            "sed -i '' 's/a/b/' /repo/a.rs",
            "sed -n '1,80w /tmp/out' /repo/a.rs",
            "sed -e '1,80p' /repo/a.rs",
            "awk '{print}' /repo/a.rs",
            "jq . /repo/a.json",
        ] {
            assert!(!command_reads_file_contents(c), "must NOT be a read: {c}");
        }
    }

    /// THE SHAPES CODEX READS WITH, captured live on 2026-09-03 from codex-cli 0.145.0.
    ///
    /// Every source-gated codex review that day was rejected "never opened". The reader list
    /// had `cat`; codex used `sed -n` windows and `nl | sed -n`, eight times in one turn and
    /// never once `cat`. Same lesson as codex_05: the classifier was built from the command a
    /// human types, not the one the agent emits.
    ///
    /// RED IF: `sed -n` ranged reads or the `nl | sed -n` pipeline stop classifying as reads,
    /// which returns codex to failing every `required_sources` dispatch.
    #[test]
    fn codex_06_the_ranged_read_shapes_codex_actually_emits_are_reads() {
        let r = |s, e| Some(LineRange { start: s, end: e });
        let command_read_range = |c: &str| command_read_range(c).map(|x| x.range);
        // Verbatim from the live trace.
        assert_eq!(command_read_range("/bin/zsh -lc \"sed -n '1,260p' /repo/a.rs\""), r(1, Some(260)));
        assert_eq!(command_read_range("/bin/zsh -lc \"sed -n '261,556p' /repo/a.rs\""), r(261, Some(556)));
        assert_eq!(command_read_range("/bin/zsh -lc \"nl -ba /repo/a.rs | sed -n '161,360p'\""), r(161, Some(360)));
        assert!(command_reads_file_contents("/bin/zsh -lc \"sed -n '1,260p' /repo/a.rs\""));
        assert!(command_reads_file_contents("/bin/zsh -lc \"nl -ba /repo/a.rs | sed -n '161,360p'\""));
        // The other spellings of the same window.
        assert_eq!(command_read_range("sed -n 1,80p /repo/a.rs"), r(1, Some(80)));
        assert_eq!(command_read_range("sed -n \"1,80p\" /repo/a.rs"), r(1, Some(80)));
        assert_eq!(command_read_range("sed -n '200,$p' /repo/a.rs"), r(200, None));
        assert_eq!(command_read_range("sed -n '7p' /repo/a.rs"), r(7, Some(7)));
        assert_eq!(command_read_range("cat /repo/a.rs | sed -n '1,50p'"), r(1, Some(50)));
        assert_eq!(command_read_range("cat -n /repo/a.rs | sed -n '1,50p'"), r(1, Some(50)));
        assert_eq!(command_read_range("nl -w3 -bt /repo/a.rs | sed -n '1,50p'"), r(1, Some(50)));
        // A window is PARTIAL on its own; the union check in agent_exec decides coverage.
        // The pipeline forms too: Grok found the first version took `nl` as the program and
        // called the window a whole read, so coverage never ran.
        assert!(!command_reads_whole_file("sed -n '1,260p' /repo/a.rs"));
        assert!(!command_reads_whole_file("sed -n '1,$p' /repo/a.rs"));
        assert!(!command_reads_whole_file("/bin/zsh -lc \"nl -ba /repo/a.rs | sed -n '161,360p'\""));
        assert!(!command_reads_whole_file("cat /repo/a.rs | sed -n '1,50p'"));
        assert!(!command_reads_whole_file("nl -ba /repo/a.rs | sed -n '1,$p'"));
        // And the plain whole readers still are.
        assert!(command_reads_whole_file("/bin/zsh -lc 'cat /repo/a.rs'"));
        assert!(command_reads_whole_file("nl -ba /repo/a.rs"));
    }

    /// The window is bound to the OPERAND, quotes stripped, and says whether `nl` fed it.
    /// Codex's round-3 finding: matching the whole command string let a decoy operand that
    /// embeds the source path collect the source's coverage.
    /// RED IF: `operand` stops being the file the reader actually opened.
    #[test]
    fn codex_08_a_ranged_read_names_its_one_operand() {
        let op = |c: &str| super::command_read_range(c).map(|x| (x.operand, x.via_nl));
        assert_eq!(op("sed -n '1,80p' /repo/a.rs"), Some(("/repo/a.rs".into(), false)));
        assert_eq!(op("sed -n '1,80p' '/repo/a.rs'"), Some(("/repo/a.rs".into(), false)));
        assert_eq!(op("sed -n '1,80p' \"/repo/a.rs\""), Some(("/repo/a.rs".into(), false)));
        assert_eq!(op("cat -n /repo/a.rs | sed -n '1,80p'"), Some(("/repo/a.rs".into(), false)));
        assert_eq!(op("nl -ba /repo/a.rs | sed -n '1,80p'"), Some(("/repo/a.rs".into(), true)));
        assert_eq!(op("nl -ba '/repo/a.rs' | sed -n '1,80p'"), Some(("/repo/a.rs".into(), true)));
        // A decoy that merely CONTAINS the source path is its own operand, nothing else.
        assert_eq!(
            op("sed -n '1,80p' /tmp/decoy=/repo/a.rs"),
            Some(("/tmp/decoy=/repo/a.rs".into(), false))
        );
        // Grok's round-3 finding: flags that error on this host, hidden by the pipe.
        for c in [
            "nl -w0 /repo/a.rs | sed -n '1,$p'",
            "nl -w /repo/a.rs | sed -n '1,$p'",
            "cat -A /repo/a.rs | sed -n '1,$p'",
            "cat -E /repo/a.rs | sed -n '1,$p'",
            "cat -T /repo/a.rs | sed -n '1,$p'",
        ] {
            assert_eq!(super::command_read_range(c), None, "must NOT be a ranged read: {c}");
        }
        assert!(super::command_read_range("nl -w3 /repo/a.rs | sed -n '1,$p'").is_some());
    }

    /// Everything one character wider than the accepted shape is not a read.
    /// RED IF: the sed parser learns `-e`, a second operand, a write script, a third pipeline
    /// stage, or lets a non-reader feed the pipe.
    #[test]
    fn codex_07_anything_wider_than_the_ranged_shape_fails_closed() {
        for c in [
            "sed -i '1,80p' /repo/a.rs",
            "sed -ni '1,80p' /repo/a.rs",
            "sed -n -i '1,80p' /repo/a.rs",
            "sed -n -e '1,80p' /repo/a.rs",
            "sed -n '1,80w /tmp/x' /repo/a.rs",
            "sed -n '1,80d' /repo/a.rs",
            "sed -n '/fn/p' /repo/a.rs",
            "sed -n '$p' /repo/a.rs",
            "sed -n '0,5p' /repo/a.rs",
            "sed -n '80,1p' /repo/a.rs",
            "sed -n '1,80p' /repo/a.rs /repo/b.rs",
            "sed -n '1,80p'",
            "sed -n '1,80p' /repo/a.rs > /tmp/out",
            "sed -n '1,80p' /repo/a.rs 2>&1",
            "sed -n '1,80p' /repo/a.rs; rm -rf /",
            "rg needle /repo/a.rs | sed -n '1,80p'",
            "ls /repo | sed -n '1,80p'",
            "cat /repo/a.rs /repo/b.rs | sed -n '1,80p'",
            "cat - /repo/a.rs | sed -n '1,80p'",
            "cat /repo/a.rs - | sed -n '1,80p'",
            // Codex, on review: a reader that does not preserve lines makes the window a
            // window over something other than the source.
            "pr /repo/a.rs | sed -n '1,9999p'",
            "bat /repo/a.rs | sed -n '1,763p'",
            "zcat /repo/a.rs.gz | sed -n '1,80p'",
            "cat -s /repo/a.rs | sed -n '1,80p'",
            "cat -ns /repo/a.rs | sed -n '1,80p'",
            "cat --squeeze-blank /repo/a.rs | sed -n '1,80p'",
            // Grok: `nl -s` with a newline separator doubles the output lines.
            "nl -s$'\\n' /repo/a.rs | sed -n '1,80p'",
            "nl -ba -s' ' /repo/a.rs | sed -n '1,80p'",
            "nl -d'' /repo/a.rs | sed -n '1,80p'",
            "nl -v0 /repo/a.rs | sed -n '1,80p'",
            "nl -p /repo/a.rs | sed -n '1,80p'",
            "nl -ba /repo/a.rs | sed -n '1,80p' | head -1",
            "nl -ba /repo/a.rs | sed -n '1,80p' /repo/b.rs",
            "cat /repo/a.rs | tail -1",
            "/bin/zsh -lc 'ls /repo && sed -n \"1,80p\" /repo/a.rs'",
        ] {
            assert_eq!(command_read_range(c), None, "must NOT be a ranged read: {c}");
            assert!(!command_reads_whole_file(c), "must NOT be a whole read: {c}");
        }
        // And none of them launder through the general classifier either, except the ones
        // that were already plain readers (there are none in this list).
        for c in ["sed -n -e '1,80p' /repo/a.rs", "cat /repo/a.rs | tail -1"] {
            assert!(!command_reads_file_contents(c), "must NOT be a read: {c}");
        }
    }

    /// Fail closed on anything the classifier cannot reason about.
    /// RED IF: pipes or redirections start being classified, where the reader may not be the
    /// part that touched the named path, or the command writes.
    ///
    /// `&&` was on this list until D-017 and is no longer, because the concern it was carrying
    /// is now carried structurally. "Does this command read a file" and "WHICH file did it
    /// read" are different questions, and the second is answered by the operand binding
    /// (`codex_03b` below), not by refusing to look at the command. Refusing the whole chain
    /// threw away real reviews: codex reads with `wc -l F && sed -n '1,240p' F`.
    /// `;` and `||` stay refused, because both exit zero on a read that failed.
    #[test]
    fn codex_03_compound_and_writing_commands_fail_closed() {
        for c in [
            "ls /repo ; cat /repo/a.rs",
            "ls /repo || cat /repo/a.rs",
            "cat /repo/a.rs | grep x",
            "cat /repo/a.rs > /tmp/copy",
            "sed -i '' 's/a/b/' /repo/a.rs",
            "cat -i /repo/a.rs",
            "cat --in-place /repo/a.rs",
            "cat /repo/a.rs & rm /repo/b.rs",
            "python3 -c 'open(\"/repo/a.rs\")'",
            "",
        ] {
            assert!(!command_reads_file_contents(c), "must fail closed: {c}");
        }
    }

    /// D-017. An `&&` chain containing a read IS a read, and it is a read of the file the
    /// READER opened, never of a path a sibling link happened to mention.
    /// RED IF: a chain stops being classified, or a decoy in one link collects another's credit.
    #[test]
    fn codex_03b_a_chain_is_classified_by_its_reader_and_bound_to_that_readers_file() {
        assert!(command_reads_file_contents("ls /repo && cat /repo/a.rs"), "the cat is a read");
        assert!(command_reads_file_contents("wc -l /repo/a.rs && sed -n '1,240p' /repo/a.rs"));

        // The old rationale for refusing chains, now enforced where it belongs. `a.rs` is
        // named by the chain and read by nothing in it.
        assert_eq!(whole_file_read_operands("ls /repo/a.rs && cat /repo/b.rs"), vec!["/repo/b.rs"]);
        assert!(command_read_ranges("wc -l /repo/a.rs && sed -n '1,9p' /repo/b.rs")
            .iter()
            .all(|r| r.operand == "/repo/b.rs"));
        // And a chain whose every link is a non-reader stays closed.
        assert!(!command_reads_file_contents("ls /repo && wc -l /repo/a.rs"));
    }

    /// The parser must actually apply the classification, not just define it.
    /// RED IF: command_execution goes back to hardcoding ToolKind::Bash.
    #[test]
    fn codex_04_the_parser_applies_the_classification() {
        let mut p = CodexExecParser::new();
        let read = r#"{"type":"item.started","item":{"type":"command_execution","id":"c1","command":"cat /repo/a.rs"}}"#;
        let list = r#"{"type":"item.started","item":{"type":"command_execution","id":"c2","command":"ls /repo"}}"#;
        let _ = p.parse_line(read);
        let _ = p.parse_line(list);
        let r = p.finish();
        let by_id = |id: &str| {
            r.tool_calls
                .iter()
                .find(|c| c.id.as_deref() == Some(id))
                .unwrap_or_else(|| panic!("missing {id}"))
                .kind
                .clone()
        };
        assert_eq!(by_id("c1"), ToolKind::ReadFile, "`cat` reads the file");
        assert_eq!(by_id("c2"), ToolKind::Bash, "`ls` does not");
    }
}

#[cfg(test)]
mod whole_file_read_operand_tests {
    use super::{shell_read_kind, whole_file_read_operand};
    use crate::ToolKind;
    use serde_json::json;

    #[test]
    fn a_plain_cat_binds_to_its_operand() {
        assert_eq!(whole_file_read_operand("cat /repo/a.rs").as_deref(), Some("/repo/a.rs"));
        assert_eq!(whole_file_read_operand("cat -n '/repo/a.rs'").as_deref(), Some("/repo/a.rs"));
        assert_eq!(whole_file_read_operand("/bin/zsh -lc \"cat /repo/a.rs\"").as_deref(), Some("/repo/a.rs"));
    }

    /// The three shapes Antigravity named. RED IF: any of them yields an operand, because the
    /// gate would then count a read that never put the source in front of the model.
    #[test]
    fn redirects_comments_and_second_operands_do_not_bind() {
        for cmd in [
            "cat /dev/null < /repo/a.rs",
            "cat /repo/b.rs # /repo/a.rs",
            "cat /repo/b.rs /repo/a.rs",
            "cat /repo/a.rs > /dev/null",
            "cat /repo/a.rs | wc -l",
            "cat $(echo /repo/a.rs)",
            "cat -",
            "xxd -l0 /repo/a.rs",
            "od -N0 /repo/a.rs",
            "cat -s /repo/a.rs",
            "strings -n 9999 /repo/a.rs",
        ] {
            assert_eq!(whole_file_read_operand(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn a_ranged_read_is_not_a_whole_read() {
        assert_eq!(whole_file_read_operand("sed -n '1,5p' /repo/a.rs"), None);
    }

    #[test]
    fn shell_read_kind_lifts_a_reading_command_and_leaves_the_rest() {
        assert_eq!(shell_read_kind(ToolKind::Bash, Some(&json!({"command": "cat /repo/a.rs"}))), ToolKind::ReadFile);
        assert_eq!(shell_read_kind(ToolKind::Bash, Some(&json!({"CommandLine": "sed -n '1,9p' a.rs"}))), ToolKind::ReadFile);
        assert_eq!(shell_read_kind(ToolKind::Bash, Some(&json!({"command": "ls -la"}))), ToolKind::Bash);
        assert_eq!(shell_read_kind(ToolKind::Grep, Some(&json!({"command": "cat a"}))), ToolKind::Grep);
        assert_eq!(shell_read_kind(ToolKind::Bash, None), ToolKind::Bash);
    }
}
