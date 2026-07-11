//! Zsh host backend for the out-of-process `flyline-standalone` editor.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crate::bash_funcs::{
    CommandWordInfo, CompletionFlags, ProgrammableCompleteReturn, find_quote_type,
};
use crate::kill_on_drop_child::KillOnDropChild;
use crate::shell::{HistoryRecord, ShellBackend};
use anyhow::Context;

const DAEMON_SCRIPT: &str = include_str!("zsh_comp_daemon.zsh");
const READY_MARKER: &str = "READY";
const FLYBEGIN: &str = "<<FLYBEGIN>>";
const FLYEND: &str = "<<FLYEND>>";
const FLYCD: &str = "<<FLYCD>>";
const FLYRELOAD: &str = "<<FLYRELOAD>>";
/// Marker whose payload is `$_comps[<cmd>]`: non-empty means a native completion
/// function is registered for the command word (even if it produced no matches).
const FLYCOMPDEF: &str = "<<FLYCOMPDEF>>";

/// First line of a broker request that asks the daemon to (re)load a completion
/// function instead of capturing completions. A real cwd never starts with this.
const RELOAD_SENTINEL: &str = "\x01RELOAD";

/// Default dir for flyline-synthesized zsh completions. Must stay in sync with
/// the `fpath` prepend in `zsh_comp_daemon.zsh` so the `_<cmd>` files are found.
const ZSH_COMPLETIONS_DIR: &str = "~/.local/share/flyline/zsh/completions/";

/// Read caps so a bad rc/completion can't hang flyline (fail-open on timeout).
const DAEMON_BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
const DUMP_TIMEOUT: Duration = Duration::from_secs(10);

/// Socket-path env set when flyline-standalone runs as the broker (run_comp_broker).
const BROKER_ENV: &str = "FLYLINE_COMP_BROKER";
const BROKER_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const BROKER_SPAWN_WAIT: Duration = Duration::from_secs(4);
/// Request I/O cap; must exceed a cold daemon boot.
const BROKER_REQUEST_TIMEOUT: Duration =
    Duration::from_secs(DAEMON_BOOT_TIMEOUT.as_secs() + CAPTURE_TIMEOUT.as_secs());

/// Cache a dead/absent broker for this process so we don't re-probe every Tab.
static BROKER_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// Dumps the shell's command vocabulary as `FLYT`-prefixed lines (rc loaded).
const DUMP_SCRIPT: &str = concat!(
    "zmodload zsh/parameter 2>/dev/null\n",
    "for k v in \"${(@kv)aliases}\"; do print -r -- \"FLYT\talias\t$k\t$v\"; done\n",
    "for k in \"${(k)functions[@]}\"; do print -r -- \"FLYT\tfunction\t$k\"; done\n",
    "for k in \"${(k)builtins[@]}\"; do print -r -- \"FLYT\tbuiltin\t$k\"; done\n",
    "for k in $reswords; do print -r -- \"FLYT\treserved\t$k\"; done\n",
    "for k v in \"${(@kv)commands}\"; do print -r -- \"FLYT\tcommand\t$k\t$v\"; done\n",
);

/// Opt out of loading the user's rc into flyline's zsh helpers.
fn no_rcs() -> bool {
    std::env::var_os("FLYLINE_ZSH_NO_RCS").is_some()
}

/// Opt out of the shared broker (falls back to a per-process in-process daemon).
fn broker_disabled() -> bool {
    std::env::var_os("FLYLINE_ZSH_NO_BROKER").is_some()
}

/// Mark `fd` close-on-exec. Critical: the widget's `$( "$FLYLINE_BIN" 3>&1 )`
/// only returns once every writer of fd 3 is closed, so a long-lived helper
/// grandchild (completion daemon, gitstatusd) that inherited it would wedge the
/// terminal. Only affects `exec`, not our own `read`/`write`.
pub fn set_cloexec(fd: std::os::fd::RawFd) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags != -1 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

/// The host shell's command vocabulary (built once with rc loaded). Backs
/// `find_alias`/`command_info`/`possible_command_words` via map lookups —
/// injection-safe, since no command word is interpolated into a shell string.
#[derive(Default)]
struct CommandTable {
    aliases: HashMap<String, String>,
    functions: HashSet<String>,
    builtins: HashSet<String>,
    reserved: HashSet<String>,
    commands: HashMap<String, String>,
    ordered: Vec<CommandWordInfo>,
}

fn build_command_table() -> CommandTable {
    if no_rcs() {
        // `zsh -fc` (no rc) is effectively instant — no point caching it.
        return match run_zsh_timeout(&["-fc", DUMP_SCRIPT], DUMP_TIMEOUT) {
            Some(out) => parse_command_table(&out),
            None => CommandTable::default(),
        };
    }
    // The rc-loaded dump takes >1s; a fresh process per prompt would pay it every
    // line. Cache the raw dump on disk, invalidated on ~/.zshrc change or TTL.
    if let Some(path) = cmd_table_cache_path() {
        if let Some(cached) = read_cached_dump(&path) {
            return parse_command_table(&cached);
        }
        if let Some(out) = run_zsh_timeout(&["-ic", DUMP_SCRIPT], DUMP_TIMEOUT) {
            write_cached_dump(&path, &out);
            return parse_command_table(&out);
        }
        return CommandTable::default();
    }
    match run_zsh_timeout(&["-ic", DUMP_SCRIPT], DUMP_TIMEOUT) {
        Some(out) => parse_command_table(&out),
        None => CommandTable::default(),
    }
}

/// Command-table cache TTL: catches PATH changes that don't touch ~/.zshrc.
const CMD_TABLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

fn cmd_table_cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("flyline").join("zsh-cmdtable"))
}

fn zshrc_mtime() -> Option<SystemTime> {
    let home = std::env::var_os("HOME")?;
    std::fs::metadata(PathBuf::from(home).join(".zshrc"))
        .ok()?
        .modified()
        .ok()
}

/// Cached dump if present, within TTL, and newer than ~/.zshrc; else `None`.
fn read_cached_dump(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    if modified
        .elapsed()
        .map(|e| e > CMD_TABLE_TTL)
        .unwrap_or(true)
    {
        return None;
    }
    if let Some(rc) = zshrc_mtime() {
        if rc > modified {
            return None;
        }
    }
    std::fs::read_to_string(path).ok()
}

/// Write the dump atomically (temp + rename) so concurrent shells don't read a
/// half-written cache. Best-effort.
fn write_cached_dump(path: &Path, dump: &str) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("zsh-cmdtable.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, dump).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Parse `FLYT`-prefixed dump lines into a `CommandTable`, ignoring rc noise.
fn parse_command_table(out: &str) -> CommandTable {
    let mut table = CommandTable::default();
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("FLYT\t") else {
            continue;
        };
        let mut parts = rest.splitn(3, '\t');
        let kind = parts.next().unwrap_or("");
        let name = match parts.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let extra = parts.next().unwrap_or("");
        match kind {
            "alias" => {
                table.aliases.insert(name.clone(), extra.to_string());
                table.ordered.push(CommandWordInfo::Alias {
                    command: name,
                    expansion: extra.to_string(),
                });
            }
            "function" => {
                table.functions.insert(name.clone());
                table.ordered.push(CommandWordInfo::Function {
                    command: name,
                    source_file: None,
                    line: None,
                });
            }
            "builtin" => {
                table.builtins.insert(name.clone());
                table.ordered.push(CommandWordInfo::Builtin {
                    command: name,
                    usage: None,
                });
            }
            "reserved" => {
                table.reserved.insert(name.clone());
                table.ordered.push(CommandWordInfo::Keyword {
                    command: name,
                    usage: None,
                });
            }
            "command" => {
                table.commands.insert(name.clone(), extra.to_string());
                table.ordered.push(CommandWordInfo::File {
                    command: name,
                    path: extra.to_string(),
                });
            }
            _ => {}
        }
    }
    table
}

/// Run `zsh <args>`, capturing stdout; `None` on failure/timeout (fail-open).
fn run_zsh_timeout(args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new("zsh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    // Ephemeral I/O reader, not a tracked bash-func thread; lint opt-out is local.
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = rx.recv().unwrap_or_default();
                return status.success().then_some(out);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// The on-disk name for a synthesized zsh completion of `cmd_word`.
///
/// zsh autoloadable completion functions must be named `_<cmd>` and live on
/// `$fpath` (the daemon prepends `ZSH_COMPLETIONS_DIR`); bash's `<cmd>` name in
/// the bash-completion dir would never be picked up here. A path-like command
/// word (alias expansion, absolute path) is reduced to its file name first.
fn zsh_completion_file_name(cmd_word: &str) -> String {
    let base = Path::new(cmd_word)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(cmd_word);
    if base.starts_with('_') {
        base.to_string()
    } else {
        format!("_{base}")
    }
}

/// The zsh host backend. Select with `shell::set_backend(&ZSH_BACKEND)` at
/// standalone startup before any `shell::backend()` call.
pub struct ZshBackend {
    comp_daemon: Mutex<Option<CompDaemon>>,
    daemon_script_path: OnceLock<PathBuf>,
    // command vocabulary built once per process
    command_table: OnceLock<CommandTable>,
}

pub static ZSH_BACKEND: ZshBackend = ZshBackend {
    comp_daemon: Mutex::new(None),
    daemon_script_path: OnceLock::new(),
    command_table: OnceLock::new(),
};

struct CompDaemon {
    _child: KillOnDropChild,
    master: std::fs::File,
}

fn zsh_eval(script: &str) -> Option<String> {
    let output = Command::new("zsh").args(["-fc", script]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.trim_end_matches(&['\n', '\r'][..]).to_string())
}

fn zsh_expand(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    // ponytail: one subprocess per expansion; fine for standalone startup paths.
    unsafe {
        std::env::set_var("_FLYLINE_EXPAND", s);
    }
    let expanded = zsh_eval("emulate -L zsh; print -rn -- ${~_FLYLINE_EXPAND}")
        .unwrap_or_else(|| s.to_string());
    unsafe {
        std::env::remove_var("_FLYLINE_EXPAND");
    }
    expanded
}

fn hostname_from_uname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc == 0 {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        String::new()
    }
}

fn shell_var_name(name: &str) -> Option<&str> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        None
    } else {
        Some(name)
    }
}

impl ZshBackend {
    fn table(&self) -> &CommandTable {
        self.command_table.get_or_init(build_command_table)
    }

    fn daemon_script_path(&self) -> &Path {
        self.daemon_script_path
            .get_or_init(|| {
                let path = std::env::temp_dir().join("flyline_zsh_comp_daemon.zsh");
                let _ = std::fs::write(&path, DAEMON_SCRIPT);
                path
            })
            .as_path()
    }

    fn ensure_comp_daemon(&self) -> anyhow::Result<()> {
        let mut guard = self.comp_daemon.lock().unwrap();
        if guard.is_none() {
            *guard = Some(spawn_comp_daemon(self.daemon_script_path())?);
        }
        Ok(())
    }

    /// Capture completions plus whether a native completer is registered for the
    /// command word (`has_completer`), used to keep flyline from offering to
    /// synthesize a completion when one already exists but simply had nothing to
    /// add (e.g. `kubectl get` with no reachable cluster).
    fn capture_completions(
        &self,
        full_command: &str,
        cursor_byte_pos: usize,
    ) -> anyhow::Result<(Vec<(String, Option<String>)>, bool)> {
        let buffer_at_cursor = full_command
            .get(..cursor_byte_pos.min(full_command.len()))
            .unwrap_or(full_command);
        // Fast path: the shared broker (daemon booted once, reused across prompts).
        if let Some(blob) = broker_capture(&self.cwd(), buffer_at_cursor) {
            return Ok((parse_capture_output(&blob), capture_has_completer(&blob)));
        }
        // Fail-open: per-process in-process daemon (original behaviour).
        self.ensure_comp_daemon()?;
        let mut guard = self.comp_daemon.lock().unwrap();
        let daemon = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("completion daemon not initialized"))?;
        let output = daemon.capture(buffer_at_cursor)?;
        Ok((
            parse_capture_output(&output),
            capture_has_completer(&output),
        ))
    }
}

impl ShellBackend for ZshBackend {
    fn is_bash(&self) -> bool {
        false
    }

    fn cwd(&self) -> String {
        std::env::var("PWD")
            .ok()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .unwrap_or_default()
    }

    fn env_var(&self, name: &str) -> Option<String> {
        if let Ok(v) = std::env::var(name) {
            return Some(v);
        }
        let safe = shell_var_name(name)?;
        let script = format!(r#"(( ${{{safe}+1}} )) && print -r -- "${{(P){safe}}}" "#);
        zsh_eval(&script).filter(|s| !s.is_empty())
    }

    fn hostname(&self) -> String {
        std::env::var("HOST")
            .ok()
            .filter(|h| !h.is_empty())
            .or_else(|| zsh_eval(r#"print -r -- "${HOST:-$(hostname 2>/dev/null)}""#))
            .filter(|h| !h.is_empty())
            .unwrap_or_else(hostname_from_uname)
    }

    fn format_var(&self, name: &str) -> String {
        let Some(safe) = shell_var_name(name) else {
            return String::new();
        };
        zsh_eval(&format!(
            r#"typeset -p {safe} 2>/dev/null || print -r -- "${safe}="" "#
        ))
        .unwrap_or_else(|| match self.env_var(name) {
            Some(value) => format!("${name}={value}"),
            None => format!("${name}="),
        })
    }

    fn vars_with_prefix(&self, prefix: &str) -> Vec<String> {
        let bare = prefix.strip_prefix('$').unwrap_or(prefix);
        if bare.is_empty() {
            return Vec::new();
        }
        let script = format!(
            r#"emulate -L zsh; prefix={bare}; for v in ${{(ok)parameters[(I)${{prefix}}*]}}; do print -r -- "${{v}}"; done"#
        );
        zsh_eval(&script)
            .map(|output| {
                let mut vars: Vec<String> = output
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| format!("${l}"))
                    .collect();
                if vars.is_empty() {
                    vars = std::env::vars()
                        .map(|(name, _)| name)
                        .filter(|name| name.starts_with(bare))
                        .map(|name| format!("${name}"))
                        .collect();
                }
                vars.sort();
                vars.dedup();
                vars
            })
            .unwrap_or_default()
    }

    fn expand_path(&self, path: &str) -> String {
        let expanded = self.expand_filename(path);
        if expanded.is_empty() {
            return std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if Path::new(&expanded).is_absolute() {
            expanded
        } else {
            match std::env::current_dir() {
                Ok(cwd) => format!("{}/{}", cwd.display(), expanded),
                Err(_) => expanded,
            }
        }
    }

    fn expand_filename(&self, filename: &str) -> String {
        zsh_expand(filename)
    }

    fn decode_prompt(&self, raw: &str, _is_prompt: bool) -> Option<String> {
        if raw.is_empty() {
            return Some(String::new());
        }
        unsafe {
            std::env::set_var("_FLYLINE_PROMPT", raw);
        }
        let decoded = zsh_eval("emulate -L zsh; print -Pn -- \"$_FLYLINE_PROMPT\"");
        unsafe {
            std::env::remove_var("_FLYLINE_PROMPT");
        }
        Some(decoded.unwrap_or_else(|| raw.to_string()))
    }

    fn find_alias(&self, cmd: &str) -> Option<String> {
        self.table()
            .aliases
            .get(cmd)
            .filter(|e| !e.is_empty())
            .cloned()
    }

    fn command_info(&self, cmd: &str) -> CommandWordInfo {
        let table = self.table();
        if let Some(expansion) = table.aliases.get(cmd) {
            return CommandWordInfo::Alias {
                command: cmd.to_string(),
                expansion: expansion.clone(),
            };
        }
        if table.functions.contains(cmd) {
            return CommandWordInfo::Function {
                command: cmd.to_string(),
                source_file: None,
                line: None,
            };
        }
        if table.builtins.contains(cmd) {
            return CommandWordInfo::Builtin {
                command: cmd.to_string(),
                usage: None,
            };
        }
        if table.reserved.contains(cmd) {
            return CommandWordInfo::Keyword {
                command: cmd.to_string(),
                usage: None,
            };
        }
        if let Some(path) = table.commands.get(cmd) {
            return CommandWordInfo::File {
                command: cmd.to_string(),
                path: path.clone(),
            };
        }
        CommandWordInfo::Unknown {
            command: cmd.to_string(),
        }
    }

    fn run_programmable_completions(
        &self,
        full_command: &str,
        _command_word: &str,
        word_under_cursor: &str,
        cursor_byte_pos: usize,
        _word_under_cursor_byte_end: usize,
    ) -> anyhow::Result<ProgrammableCompleteReturn> {
        let (pairs, has_completer) = self.capture_completions(full_command, cursor_byte_pos)?;
        let completions = pairs_to_completion_strings(&pairs);
        let mut flags = CompletionFlags::default();
        flags.quote_type = find_quote_type(word_under_cursor);
        flags.some_dont_end_in_equal_sign = completions.iter().any(|s| !s.ends_with('='));
        // A registered completer that ran but produced nothing (e.g. `kubectl get`
        // with no reachable cluster) still counts as "useful": flyline should fall
        // silent like cobra/fig/carapace rather than offering to synthesize a
        // completion (which can't know cluster state anyway). Only a command with
        // no completer at all is treated as useless, which triggers the flycomp prompt.
        let compspec_was_useful = !completions.is_empty() || has_completer;
        Ok(ProgrammableCompleteReturn::new(
            completions,
            flags,
            compspec_was_useful,
        ))
    }

    fn possible_command_words(&self) -> Vec<CommandWordInfo> {
        self.table().ordered.clone()
    }

    fn evaluate_shell_string(&self, script: &str) -> anyhow::Result<()> {
        let status = Command::new("zsh").args(["-fc", script]).status()?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("zsh -fc exited with {status}")
        }
    }

    fn flycomp_output_format(&self) -> flycomp::OutputFormat {
        // zsh consumes native `#compdef` functions, not bash `complete -F` scripts.
        flycomp::OutputFormat::Zsh
    }

    fn activate_completion_script(
        &self,
        command_word: &str,
        _script: &str,
        flycomp_output: Option<&str>,
    ) -> anyhow::Result<()> {
        // The script is already on disk (resolve_and_write_completion_script ran);
        // load that `_<cmd>` file into the completion daemon so the immediate Tab
        // re-run uses it, mirroring bash's in-process eval.
        let path = self.resolve_completion_script_path(command_word, flycomp_output);
        let path_str = path.to_string_lossy().into_owned();
        if path_str.contains('\n') {
            anyhow::bail!("completion script path must not contain newlines");
        }

        let mut activated = false;
        // Shared broker first, so every session picks up the new completion.
        if !broker_disabled() && broker_reload(&path_str) {
            activated = true;
        }
        // Also reload this process's in-process daemon if it exists (broker-disabled
        // path, or broker unreachable), so the local capture below sees it.
        if let Ok(mut guard) = self.comp_daemon.lock() {
            if let Some(daemon) = guard.as_mut() {
                if daemon.reload(&path_str).is_ok() {
                    activated = true;
                }
            }
        }
        if activated {
            return Ok(());
        }
        // Fail-open: boot an in-process daemon and reload there.
        self.ensure_comp_daemon()?;
        let mut guard = self.comp_daemon.lock().unwrap();
        let daemon = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("completion daemon not initialized"))?;
        daemon.reload(&path_str)?;
        Ok(())
    }

    fn reset_caches(&self) {}

    fn warm_completion_caches(&self) {
        // Boot the daemon (broker or in-process) and table off the hot path.
        if broker_disabled() {
            let _ = self.ensure_comp_daemon();
        } else if let Some(path) = broker_socket_path() {
            let _ = connect_or_spawn_broker(&path);
        }
        let _ = self.table();
    }

    fn read_terminating_signal(&self) -> libc::c_int {
        0
    }

    fn resolve_completion_script_path(
        &self,
        command_word: &str,
        flycomp_output: Option<&str>,
    ) -> PathBuf {
        let poss_alias = self.find_alias(command_word);
        let alias_def = poss_alias
            .as_deref()
            .filter(|alias| !alias.is_empty())
            .unwrap_or(command_word);
        let cmd_word = alias_def.split_whitespace().next().unwrap_or(alias_def);
        let file_name = zsh_completion_file_name(cmd_word);
        let output_dir = flycomp_output.unwrap_or(ZSH_COMPLETIONS_DIR);
        Path::new(&self.expand_path(output_dir)).join(file_name)
    }

    fn resolve_and_write_completion_script(
        &self,
        command_word: &str,
        script: &str,
        flycomp_output: Option<&str>,
    ) -> Result<PathBuf, std::io::Error> {
        let write_path = self.resolve_completion_script_path(command_word, flycomp_output);
        if let Some(parent) = write_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&write_path, script)?;
        Ok(write_path)
    }

    fn parse_history_from_memory(&self) -> Vec<HistoryRecord> {
        // ponytail: standalone child cannot read the parent zsh's in-memory history.
        vec![]
    }

    fn last_command_exit_status(&self) -> i32 {
        std::env::var("FLYLINE_LAST_EXIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| std::env::var("_").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    }

    fn multiline_command_count(&self) -> i32 {
        0
    }

    fn shell_pgrp(&self) -> libc::pid_t {
        unsafe { libc::getpgrp() }
    }

    /// This host spawns a fresh process per prompt, so settings changed via
    /// `flyline <subcommand>` are persisted to a config file rather than kept in
    /// a live builtin. Uses `$XDG_CONFIG_HOME/flyline/settings.json`, falling back
    /// to `~/.config/flyline/settings.json`.
    fn persisted_settings_path(&self) -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })?;
        Some(base.join("flyline").join("settings.json"))
    }

    /// Transient tutorial state is scoped to the terminal session so it never
    /// leaks into other terminals or outlives this one, matching how the Bash
    /// builtin keeps it live in its per-terminal process. The session is keyed
    /// by the POSIX session id (stable across this terminal's per-prompt
    /// processes, distinct per terminal window). Stored under
    /// `$XDG_RUNTIME_DIR/flyline/` (falling back to the system temp dir).
    fn session_state_path(&self) -> Option<std::path::PathBuf> {
        // SAFETY: getsid is a simple, thread-safe syscall with no preconditions.
        let sid = unsafe { libc::getsid(0) };
        if sid <= 0 {
            return None;
        }
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Some(base.join("flyline").join(format!("session-{sid}.json")))
    }
}

impl CompDaemon {
    /// `cd` the long-lived daemon to `cwd` (so path/git completions match the
    /// caller) then capture. For the broker; one daemon serves many directories.
    fn cd_and_capture(&mut self, cwd: &str, buffer_at_cursor: &str) -> anyhow::Result<String> {
        if !cwd.is_empty() && !cwd.contains('\n') {
            self.master.write_all(cwd.as_bytes()).context("write cwd")?;
            // ^F -> fly-cd widget (see zsh_comp_daemon.zsh): cd + clear buffer.
            self.master.write_all(b"\x06").context("write ^F")?;
            self.master.flush().context("flush cwd")?;
            read_until_marker_timeout(&self.master, FLYCD, CAPTURE_TIMEOUT)
                .context("wait for daemon cd")?;
        }
        self.capture(buffer_at_cursor)
    }

    fn capture(&mut self, buffer_at_cursor: &str) -> anyhow::Result<String> {
        if buffer_at_cursor.contains('\n') {
            anyhow::bail!("completion buffer must not contain newlines");
        }
        self.master
            .write_all(buffer_at_cursor.as_bytes())
            .context("write buffer to zsh daemon")?;
        self.master
            .write_all(b"\t")
            .context("write tab to trigger completion")?;
        self.master.flush().context("flush daemon stdin")?;
        let output = read_until_marker_timeout(&self.master, FLYEND, CAPTURE_TIMEOUT)?;
        // Reset line for next round (Ctrl-U).
        self.master
            .write_all(b"\x15")
            .context("reset daemon line")?;
        self.master.flush().context("flush after reset")?;
        Ok(output)
    }

    /// Load the `_<cmd>` completion function at `path` into the running daemon so
    /// subsequent captures use it. Types the path then fires the `fly-reload`
    /// widget (^G) which autoloads + `compdef`s it (see zsh_comp_daemon.zsh).
    fn reload(&mut self, path: &str) -> anyhow::Result<String> {
        if path.contains('\n') {
            anyhow::bail!("reload path must not contain newlines");
        }
        self.master
            .write_all(path.as_bytes())
            .context("write reload path")?;
        // ^G -> fly-reload widget: register the completion + clear buffer.
        self.master.write_all(b"\x07").context("write ^G")?;
        self.master.flush().context("flush reload path")?;
        let output = read_until_marker_timeout(&self.master, FLYRELOAD, CAPTURE_TIMEOUT)?;
        // Clear the line in case the widget left anything behind (Ctrl-U).
        self.master
            .write_all(b"\x15")
            .context("reset daemon line")?;
        self.master.flush().context("flush after reload")?;
        Ok(output)
    }
}

/// Parse daemon output between the markers into `(value, description)` pairs.
///
/// zsh calls our `compadd` override once per completer/tag, so the same value can
/// be emitted several times (e.g. `cd` adds each dir under multiple tags). zsh
/// dedups matches before display; we replicate that here, keeping first-seen order
/// and the first non-empty description.
pub fn parse_capture_output(blob: &str) -> Vec<(String, Option<String>)> {
    let mut inside = false;
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in blob.lines() {
        let line = line.trim_end_matches('\r');
        if line.contains(FLYEND) {
            break;
        }
        // The completer-registration marker is metadata, not a completion value.
        if line.contains(FLYCOMPDEF) {
            continue;
        }
        if inside && !line.is_empty() {
            let (value, desc) = parse_capture_line(line);
            match seen.get(&value) {
                Some(&idx) => {
                    if out[idx].1.is_none() && desc.is_some() {
                        out[idx].1 = desc;
                    }
                }
                None => {
                    seen.insert(value.clone(), out.len());
                    out.push((value, desc));
                }
            }
        }
        if line.contains(FLYBEGIN) {
            inside = true;
        }
    }
    out
}

/// Whether the daemon reported a native completion function for the command word
/// (the `<<FLYCOMPDEF>>` marker carries `$_comps[cmd]`; a non-empty payload means
/// a completer is registered). Lets the backend distinguish "a completer ran but
/// had nothing to add" (stay silent) from "no completer exists" (offer flycomp).
pub fn capture_has_completer(blob: &str) -> bool {
    blob.lines().any(|line| {
        line.trim_end_matches('\r')
            .strip_prefix(FLYCOMPDEF)
            .is_some_and(|rest| !rest.trim().is_empty())
    })
}

/// Parse one compadd-capture line (`value` or `value -- description`).
pub fn parse_capture_line(line: &str) -> (String, Option<String>) {
    if let Some(pos) = line.find(" -- ") {
        let value = line[..pos].to_string();
        let desc = line[pos + 4..].trim();
        (
            value,
            if desc.is_empty() {
                None
            } else {
                Some(desc.to_string())
            },
        )
    } else {
        (line.to_string(), None)
    }
}

/// Convert parsed pairs to flyline completion strings (`value\\tdescription`).
pub fn pairs_to_completion_strings(pairs: &[(String, Option<String>)]) -> Vec<String> {
    pairs
        .iter()
        .map(|(value, desc)| match desc {
            Some(d) => format!("{value}\t{d}"),
            None => value.clone(),
        })
        .collect()
}

fn read_until_marker(reader: &mut std::fs::File, marker: &str) -> anyhow::Result<String> {
    let mut buf = String::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).context("read from zsh daemon")?;
        if n == 0 {
            break;
        }
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
        if buf.contains(marker) {
            break;
        }
    }
    Ok(buf)
}

/// `read_until_marker` bounded by `timeout`, on a worker thread so a stuck shell
/// can't hang flyline; on timeout the caller drops the daemon, unblocking it.
fn read_until_marker_timeout(
    master: &std::fs::File,
    marker: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    let mut reader = master.try_clone().context("clone pty master for read")?;
    let marker_owned = marker.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    // Ephemeral I/O reader, not a tracked bash-func thread; lint opt-out is local.
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        let _ = tx.send(read_until_marker(&mut reader, &marker_owned));
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => anyhow::bail!("timed out waiting for {marker} from zsh daemon"),
    }
}

// ponytail: libc openpty — plain pipes cannot drive ZLE `complete-word`; no extra crate.
fn spawn_comp_daemon(script_path: &Path) -> anyhow::Result<CompDaemon> {
    spawn_comp_daemon_with_env(script_path, &[])
}

/// Boot the headless completion daemon. `extra_env` is layered onto the child
/// `zsh` (used by tests to point at a scratch rc); production callers pass `&[]`.
///
/// `FLYLINE_ZSH_DAEMON=1` is always set here — the single choke point every
/// daemon (in-process and broker) flows through. `scripts/flyline.zsh` reads it
/// to skip installing its interactive `line-init` editor hook, which the
/// completion-only daemon never needs and which could otherwise re-enter the
/// editor against the user's controlling tty.
fn spawn_comp_daemon_with_env(
    script_path: &Path,
    extra_env: &[(&str, &str)],
) -> anyhow::Result<CompDaemon> {
    use std::os::unix::io::FromRawFd;

    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    unsafe {
        if libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) != 0
        {
            anyhow::bail!("openpty failed: {}", std::io::Error::last_os_error());
        }
    }
    // Pty ends close-on-exec so no helper inherits the master (which would keep
    // the pty from hanging up on exit, orphaning the daemon). Stdio dup2 below is
    // unaffected by FD_CLOEXEC.
    set_cloexec(master);
    set_cloexec(slave);

    let slave_file = unsafe { std::fs::File::from_raw_fd(slave) };
    let slave_in = slave_file.try_clone().context("clone pty slave")?;

    // Load the user's rc by default (for their fpath completions); NO_RCS uses -f.
    let args: &[&str] = if no_rcs() { &["-f", "-i"] } else { &["-i"] };
    let mut cmd = Command::new("zsh");
    cmd.args(args)
        .env("FLYLINE_ZSH_DAEMON", "1")
        .stdin(Stdio::from(slave_in))
        .stdout(Stdio::from(slave_file))
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let child = cmd.spawn().context("spawn zsh completion daemon")?;

    // Guard the child now so any early return (e.g. boot timeout) kills it.
    let daemon = CompDaemon {
        _child: KillOnDropChild::new(child),
        master: unsafe { std::fs::File::from_raw_fd(master) },
    };

    // Startup-file arg makes `zsh -i` exit after the script; source over pty instead.
    let source_cmd = format!("source {}\n", script_path.display());
    (&daemon.master)
        .write_all(source_cmd.as_bytes())
        .context("source daemon script")?;
    (&daemon.master).flush().context("flush source command")?;

    let boot = read_until_marker_timeout(&daemon.master, READY_MARKER, DAEMON_BOOT_TIMEOUT)
        .context("wait for zsh daemon READY")?;
    if !boot.contains(READY_MARKER) {
        anyhow::bail!("zsh daemon did not emit READY");
    }

    Ok(daemon)
}

// Persistent completion broker: boots one daemon and serves completions over a
// Unix socket, shared across prompts/terminals, instead of paying the ~1.3s
// rc-loaded boot per prompt. Strictly fail-open — any failure falls back to the
// in-process daemon, so the broker can never wedge the shell.

/// Binary identity for the broker key: the running executable's mtime + size.
/// Any rebuild, upgrade, or reinstall rewrites the file, so this changes and
/// clients route to a fresh broker (the previous one idles out) — no manual
/// restart needed. `current_exe` resolves symlinks, so this works for the dev
/// `~/.local/lib -> target/...` symlink, `cargo install`, and package-manager
/// installs alike (not just local/standalone layouts). Size is folded in so
/// version bumps are still distinguished under reproducible builds that pin
/// mtime. Falls back to "0" (version-agnostic) if the path is unreadable.
fn binary_version_key() -> String {
    std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{mtime}.{}", m.len())
        })
        .unwrap_or_else(|_| "0".to_string())
}

/// Broker socket filename, keyed by ~/.zshrc mtime, binary identity, and no-rcs
/// so an rc edit *or* a new binary routes to a fresh daemon.
fn broker_socket_name(rc_key: u64, bin_key: &str, norc: u8) -> String {
    format!("zcompd-{rc_key}-{bin_key}-{norc}.sock")
}

/// Per-rc, per-binary broker socket. `$XDG_RUNTIME_DIR` preferred, else temp dir.
fn broker_socket_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("flyline");
    let _ = std::fs::create_dir_all(&dir);
    let rc_key = zshrc_mtime()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let norc = u8::from(no_rcs());
    Some(dir.join(broker_socket_name(rc_key, &binary_version_key(), norc)))
}

/// Capture `buffer_at_cursor` in `cwd` via the broker, returning the raw daemon
/// blob (parsed by the caller so it can also read the `<<FLYCOMPDEF>>` marker).
/// `None` (→ in-process fallback) when disabled, unreachable, or the reply is
/// malformed; a valid empty capture still returns `Some(blob)` so we don't
/// redundantly re-capture in-process.
fn broker_capture(cwd: &str, buffer_at_cursor: &str) -> Option<String> {
    // Under `cargo test` current_exe() is the harness, so a spawn would re-exec
    // the whole suite. Tests drive `run_comp_broker` directly.
    if cfg!(test) {
        return None;
    }
    if broker_disabled()
        || BROKER_UNAVAILABLE.load(Ordering::Relaxed)
        || buffer_at_cursor.contains('\n')
    {
        return None;
    }
    match broker_request(cwd, buffer_at_cursor) {
        Some(blob) if blob.contains(FLYEND) => Some(blob),
        _ => {
            BROKER_UNAVAILABLE.store(true, Ordering::Relaxed);
            None
        }
    }
}

/// Send one request to the broker (spawning it if needed) and read the reply.
fn broker_request(cwd: &str, buffer_at_cursor: &str) -> Option<String> {
    let path = broker_socket_path()?;
    let mut stream = connect_or_spawn_broker(&path)?;
    stream.set_read_timeout(Some(BROKER_REQUEST_TIMEOUT)).ok()?;
    stream
        .set_write_timeout(Some(BROKER_REQUEST_TIMEOUT))
        .ok()?;
    let req = format!("{cwd}\n{buffer_at_cursor}\n");
    stream.write_all(req.as_bytes()).ok()?;
    stream.flush().ok()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;
    Some(resp)
}

/// Ask the broker (spawning it if needed) to (re)load the completion function at
/// `script_path`. Returns true when the daemon acknowledged with the reload
/// marker. Fail-open: any error returns false so activation falls back locally.
fn broker_reload(script_path: &str) -> bool {
    // Under `cargo test` current_exe() is the harness; never spawn a broker.
    if cfg!(test) {
        return false;
    }
    let Some(path) = broker_socket_path() else {
        return false;
    };
    let Some(mut stream) = connect_or_spawn_broker(&path) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(BROKER_REQUEST_TIMEOUT))
        .is_err()
        || stream
            .set_write_timeout(Some(BROKER_REQUEST_TIMEOUT))
            .is_err()
    {
        return false;
    }
    let req = format!("{RELOAD_SENTINEL}\n{script_path}\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    if stream.flush().is_err() {
        return false;
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut resp = String::new();
    if stream.read_to_string(&mut resp).is_err() {
        return false;
    }
    resp.contains(FLYRELOAD)
}

/// Connect to the broker, spawning a detached one and polling for its socket if
/// none is listening yet.
fn connect_or_spawn_broker(path: &Path) -> Option<UnixStream> {
    if let Ok(s) = UnixStream::connect(path) {
        return Some(s);
    }
    spawn_broker(path)?;
    let deadline = Instant::now() + BROKER_SPAWN_WAIT;
    loop {
        if let Ok(s) = UnixStream::connect(path) {
            return Some(s);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Spawn `flyline-standalone` in broker mode, detached (setsid) so a closing
/// terminal's SIGHUP can't kill the shared daemon.
fn spawn_broker(path: &Path) -> Option<()> {
    // Safety boundary: never re-exec the test harness (current_exe under tests).
    if cfg!(test) {
        return None;
    }
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().ok()?;
    let mut cmd = Command::new(exe);
    cmd.env(BROKER_ENV, path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // fd 3 is already close-on-exec, so the detached broker can't hold the
    // parent's `$( ... 3>&1 )` open.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().ok()?;
    Some(())
}

/// Broker entry point: boot one daemon and serve completions until idle. Returns
/// only on setup failure; the idle watchdog exits the process otherwise.
pub fn run_comp_broker(socket_path: &Path) -> i32 {
    // Singleton: if a broker is already listening here, let it own the socket.
    if UnixStream::connect(socket_path).is_ok() {
        return 0;
    }
    if let Some(dir) = socket_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Clear a stale socket left by a crashed broker, then claim the path.
    let _ = std::fs::remove_file(socket_path);
    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(_) => return 0, // lost the bind race; the winner serves
    };
    // Fail-open: on boot failure, tear down so clients revert to in-process.
    let mut daemon = match spawn_comp_daemon(ZSH_BACKEND.daemon_script_path()) {
        Ok(d) => d,
        Err(_) => {
            let _ = std::fs::remove_file(socket_path);
            return 1;
        }
    };

    let last_activity = Arc::new(Mutex::new(Instant::now()));
    spawn_idle_watchdog(last_activity.clone(), socket_path.to_path_buf());

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if let Ok(mut act) = last_activity.lock() {
            *act = Instant::now();
        }
        match read_broker_request(&stream) {
            Some(BrokerRequest::Capture { cwd, buffer }) => {
                let blob = daemon.cd_and_capture(&cwd, &buffer).unwrap_or_default();
                let _ = stream.write_all(blob.as_bytes());
            }
            Some(BrokerRequest::Reload { path }) => {
                let blob = daemon.reload(&path).unwrap_or_default();
                let _ = stream.write_all(blob.as_bytes());
            }
            None => {}
        }
        // Dropping the stream closes it: EOF frames the response for the client.
    }
    0
}

/// A request served by the broker: capture completions, or (re)load a
/// synthesized completion function into the daemon.
enum BrokerRequest {
    Capture { cwd: String, buffer: String },
    Reload { path: String },
}

/// Read a broker request. A capture request is `<cwd>\n<buffer>\n`; a reload
/// request is `<RELOAD_SENTINEL>\n<path>\n`. A missing second line yields an
/// empty tail (harmless empty completion / no-op reload).
fn read_broker_request(stream: &UnixStream) -> Option<BrokerRequest> {
    let clone = stream.try_clone().ok()?;
    let _ = clone.set_read_timeout(Some(BROKER_REQUEST_TIMEOUT));
    let mut reader = BufReader::new(clone);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let first = first.trim_end_matches('\n').to_string();
    let mut second = String::new();
    reader.read_line(&mut second).ok()?;
    let second = second.trim_end_matches('\n').to_string();
    if first == RELOAD_SENTINEL {
        Some(BrokerRequest::Reload { path: second })
    } else {
        Some(BrokerRequest::Capture {
            cwd: first,
            buffer: second,
        })
    }
}

/// Exit the broker process once it has been idle past `BROKER_IDLE_TIMEOUT`.
fn spawn_idle_watchdog(last_activity: Arc<Mutex<Instant>>, socket_path: PathBuf) {
    // Ephemeral watchdog, not a tracked bash-func thread; lint opt-out is local.
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let idle = last_activity
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if idle >= BROKER_IDLE_TIMEOUT {
                let _ = std::fs::remove_file(&socket_path);
                std::process::exit(0);
            }
        }
    });
}

#[cfg(test)]
fn zsh_available() -> bool {
    Command::new("which")
        .arg("zsh")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_capture_line_value_only() {
        assert_eq!(parse_capture_line("main"), ("main".to_string(), None));
    }

    #[test]
    fn parse_capture_line_with_description() {
        assert_eq!(
            parse_capture_line("get -- Display a resource"),
            ("get".to_string(), Some("Display a resource".to_string()),),
        );
    }

    #[test]
    fn parse_capture_output_between_markers() {
        let blob = "noise\n<<FLYBEGIN>>\nmain\nget -- Display a resource\n<<FLYEND>>\ntrailer\n";
        assert_eq!(
            parse_capture_output(blob),
            vec![
                ("main".to_string(), None),
                ("get".to_string(), Some("Display a resource".to_string())),
            ],
        );
    }

    #[test]
    fn parse_capture_output_skips_empty_lines() {
        let blob = "<<FLYBEGIN>>\n\nfoo\n<<FLYEND>>\n";
        assert_eq!(parse_capture_output(blob), vec![("foo".to_string(), None)],);
    }

    #[test]
    fn parse_capture_output_dedups_repeated_values() {
        // `cd` completion emits each dir under several tags; keep first-seen order
        // and adopt the first non-empty description that appears for a value.
        let blob = "<<FLYBEGIN>>\nbin\nsrc\nbin\nsrc -- source\nbin\n<<FLYEND>>\n";
        assert_eq!(
            parse_capture_output(blob),
            vec![
                ("bin".to_string(), None),
                ("src".to_string(), Some("source".to_string())),
            ],
        );
    }

    #[test]
    fn set_cloexec_marks_and_blocks_inheritance() {
        // pipe(2) fds start without FD_CLOEXEC.
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);
        assert_eq!(
            unsafe { libc::fcntl(w, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );

        set_cloexec(w);
        assert_ne!(
            unsafe { libc::fcntl(w, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0,
            "set_cloexec did not set FD_CLOEXEC"
        );

        // A child that execs must NOT inherit `w`; after we close our own copy
        // the reader hangs up. If `w` leaked into the child, POLLHUP never fires
        // and the poll times out — exactly the bug that wedged the zsh widget.
        let mut child = Command::new("sleep").arg("2").spawn().unwrap();
        unsafe { libc::close(w) };
        let mut pfd = libc::pollfd {
            fd: r,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&raw mut pfd, 1, 1000) };
        let _ = child.kill();
        let _ = child.wait();
        unsafe { libc::close(r) };
        assert!(ready > 0, "reader never hung up: child inherited the fd");
    }

    #[test]
    fn parse_command_table_classifies_kinds_and_ignores_noise() {
        // Includes rc stdout noise (lines without the FLYT sentinel) that must
        // be skipped, plus one entry of each kind.
        let dump = "p10k instant prompt noise\n\
            FLYT\talias\tgst\tgit status\n\
            FLYT\tfunction\tmy_fn\n\
            FLYT\tbuiltin\tprint\n\
            FLYT\treserved\tif\n\
            FLYT\tcommand\tgit\t/usr/bin/git\n\
            more noise\n";
        let table = parse_command_table(dump);

        assert_eq!(
            table.aliases.get("gst").map(String::as_str),
            Some("git status")
        );
        assert!(table.functions.contains("my_fn"));
        assert!(table.builtins.contains("print"));
        assert!(table.reserved.contains("if"));
        assert_eq!(
            table.commands.get("git").map(String::as_str),
            Some("/usr/bin/git")
        );
        // One CommandWordInfo per real entry; noise lines dropped.
        assert_eq!(table.ordered.len(), 5);
    }

    #[test]
    fn parse_command_table_skips_entries_without_a_name() {
        let table = parse_command_table("FLYT\talias\t\nFLYT\tbuiltin\tvalid\n");
        assert!(table.aliases.is_empty());
        assert!(table.builtins.contains("valid"));
        assert_eq!(table.ordered.len(), 1);
    }

    #[test]
    fn command_table_cache_round_trips_and_expires() {
        let dir = std::env::temp_dir().join(format!(
            "flyline_cmdtable_test_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("zsh-cmdtable");
        let dump = "FLYT\talias\tgd\tgit diff\nFLYT\tcommand\tls\t/bin/ls\n";

        // Missing cache -> None (caller must rebuild).
        assert!(read_cached_dump(&path).is_none());

        // Fresh write -> read returns identical content that parses correctly.
        write_cached_dump(&path, dump);
        let cached = read_cached_dump(&path).expect("fresh cache should be readable");
        assert_eq!(cached, dump);
        assert_eq!(
            parse_command_table(&cached)
                .aliases
                .get("gd")
                .map(String::as_str),
            Some("git diff")
        );

        // Backdate past the TTL -> None (stale cache is rejected).
        let old = SystemTime::now() - CMD_TABLE_TTL - Duration::from_secs(60);
        let secs = old
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as libc::time_t;
        let tv = libc::timeval {
            tv_sec: secs,
            tv_usec: 0,
        };
        let times = [tv, tv];
        let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::utimes(c_path.as_ptr(), times.as_ptr()) }, 0);
        assert!(
            read_cached_dump(&path).is_none(),
            "expired cache should be rejected"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pairs_to_completion_strings_formats_tab_descriptions() {
        let pairs = vec![
            ("a".to_string(), None),
            ("b".to_string(), Some("desc".to_string())),
        ];
        assert_eq!(
            pairs_to_completion_strings(&pairs),
            vec!["a".to_string(), "b\tdesc".to_string()],
        );
    }

    #[test]
    fn zsh_env_var_reads_home() {
        if !zsh_available() {
            return;
        }
        let backend = &ZSH_BACKEND;
        assert_eq!(backend.env_var("HOME"), std::env::var("HOME").ok());
    }

    #[test]
    fn zsh_run_programmable_completions_git() {
        if !zsh_available() {
            return;
        }
        // ponytail: pty daemon can hang in CI/WSL; cap wait so `cargo test --lib` stays bounded.
        let backend = &ZSH_BACKEND;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = backend.run_programmable_completions("git ", "git", "", 4, 4);
            let _ = tx.send(result);
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(15))
            .expect("git completion capture timed out after 15s")
            .expect("git completion capture");
        assert!(
            !result.completions.is_empty(),
            "expected git completions, got empty",
        );
        assert!(
            result
                .completions
                .iter()
                .any(|s| s.starts_with("add") || s.starts_with("checkout")),
            "expected git subcommand in {:?}",
            result.completions.iter().take(5).collect::<Vec<_>>(),
        );
    }

    /// When the daemon loads a user rc that sources
    /// `scripts/flyline.zsh`, the `FLYLINE_ZSH_DAEMON` guard must stop the rc
    /// from installing flyline's interactive editor hook inside the daemon. We
    /// assert the effect directly — the `_flyline_edit` widget is absent — which
    /// is deterministic and doesn't depend on the recursive editor's tty
    /// behaviour. The daemon must still reach READY with the rc loaded.
    ///
    /// The rc writes two markers so a broken test can't pass silently: one
    /// proves the rc was actually sourced (else ZDOTDIR wasn't honored and the
    /// test is a no-op), the other is written only if `_flyline_edit` got
    /// registered — i.e. the guard failed.
    #[test]
    fn daemon_rc_does_not_install_editor_hook() {
        if !zsh_available() {
            eprintln!("skipping flyline-rc daemon boot: zsh not available");
            return;
        }
        // `-f` (no_rcs) skips the rc entirely, so the guard would never be
        // exercised; this test only means something when the rc is loaded.
        if no_rcs() {
            eprintln!("skipping flyline-rc daemon boot: FLYLINE_ZSH_NO_RCS set");
            return;
        }

        let zdotdir = std::env::temp_dir().join(format!(
            "flyline-test-zdotdir-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&zdotdir).expect("create scratch ZDOTDIR");

        let flyline_script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/flyline.zsh");
        let rc_marker = zdotdir.join("rc_sourced");
        let hook_marker = zdotdir.join("editor_hook_installed");
        let zshrc = zdotdir.join(".zshrc");
        std::fs::write(
            &zshrc,
            format!(
                "print -r -- sourced > {rc_marker:?}\n\
                 source {flyline_script}\n\
                 (( ${{+widgets[_flyline_edit]}} )) && print -r -- hooked > {hook_marker:?}\n",
            ),
        )
        .expect("write scratch .zshrc");

        let script_path = ZSH_BACKEND.daemon_script_path().to_path_buf();
        let zdotdir_str = zdotdir.to_str().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result =
                spawn_comp_daemon_with_env(&script_path, &[("ZDOTDIR", zdotdir_str.as_str())])
                    .map(|_daemon| ());
            let _ = tx.send(result);
        });

        let booted = rx.recv_timeout(DAEMON_BOOT_TIMEOUT + Duration::from_secs(5));
        let rc_was_sourced = rc_marker.exists();
        let editor_hook_installed = hook_marker.exists();
        let _ = std::fs::remove_dir_all(&zdotdir);

        assert!(
            rc_was_sourced,
            "scratch rc was not sourced — ZDOTDIR not honored, test is a no-op",
        );
        booted
            .expect("daemon boot did not resolve in time")
            .expect("daemon should reach READY with a flyline-enabled rc");
        assert!(
            !editor_hook_installed,
            "FLYLINE_ZSH_DAEMON guard failed: the daemon's rc installed the \
             _flyline_edit line-init hook",
        );
    }

    /// Serve two requests over the socket and confirm the second reuses the warm
    /// daemon (no per-line reboot). Drives `run_comp_broker` directly.
    #[test]
    fn broker_serves_and_reuses_daemon() {
        if !zsh_available() {
            eprintln!("skipping broker round-trip: zsh not available");
            return;
        }
        let sock = std::env::temp_dir().join(format!(
            "flyline-test-broker-{}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let broker_sock = sock.clone();
        // Broker never returns during the test; it dies with the test process.
        #[allow(clippy::disallowed_methods)]
        std::thread::spawn(move || {
            run_comp_broker(&broker_sock);
        });

        let cwd = std::env::temp_dir();
        let cwd = cwd.to_str().unwrap();
        let request = |buffer: &str| -> Option<Vec<(String, Option<String>)>> {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut stream = loop {
                if let Ok(s) = UnixStream::connect(&sock) {
                    break s;
                }
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            stream.set_read_timeout(Some(BROKER_REQUEST_TIMEOUT)).ok()?;
            stream
                .write_all(format!("{cwd}\n{buffer}\n").as_bytes())
                .ok()?;
            stream.flush().ok()?;
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut resp = String::new();
            stream.read_to_string(&mut resp).ok()?;
            resp.contains(FLYEND).then(|| parse_capture_output(&resp))
        };

        // First request boots the daemon (rc load) and returns git subcommands.
        let first = request("git ").expect("broker should serve git completions");
        assert!(
            first.len() > 10,
            "expected >10 git completions, got {}",
            first.len()
        );

        // Second request must reuse the warm daemon: still correct, and fast.
        let started = Instant::now();
        let second = request("git ").expect("broker should serve a warm request");
        assert!(second.len() > 10);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "warm request took {:?} — daemon not reused?",
            started.elapsed()
        );

        let _ = std::fs::remove_file(&sock);
    }

    #[test]
    fn zsh_completion_file_name_prefixes_underscore() {
        assert_eq!(zsh_completion_file_name("kubectl"), "_kubectl");
        // Path-like command words are reduced to their file name.
        assert_eq!(
            zsh_completion_file_name("/opt/homebrew/bin/kubectl"),
            "_kubectl"
        );
        assert_eq!(zsh_completion_file_name("~/bin/foo"), "_foo");
        // An already-underscored name is not doubled.
        assert_eq!(zsh_completion_file_name("_kubectl"), "_kubectl");
    }

    #[test]
    fn broker_socket_name_varies_with_rc_and_binary() {
        assert_eq!(
            broker_socket_name(100, "200.4096", 0),
            "zcompd-100-200.4096-0.sock"
        );
        // A rebuild/upgrade (new binary mtime) routes clients to a fresh broker.
        assert_ne!(
            broker_socket_name(100, "200.4096", 0),
            broker_socket_name(100, "201.4096", 0)
        );
        // A size change alone (e.g. reproducible build, same pinned mtime) too.
        assert_ne!(
            broker_socket_name(100, "200.4096", 0),
            broker_socket_name(100, "200.8192", 0)
        );
        // An rc edit does too, as before.
        assert_ne!(
            broker_socket_name(100, "200.4096", 0),
            broker_socket_name(101, "200.4096", 0)
        );
        // no-rcs remains part of the key.
        assert_ne!(
            broker_socket_name(100, "200.4096", 0),
            broker_socket_name(100, "200.4096", 1)
        );
    }

    #[test]
    fn binary_version_key_is_stable() {
        // Must not flap within a process, or clients and broker would disagree.
        assert_eq!(binary_version_key(), binary_version_key());
        // mtime.size shape (or the "0" fallback for an unreadable path).
        let key = binary_version_key();
        assert!(
            key == "0" || key.split('.').count() == 2,
            "unexpected binary key shape: {key}"
        );
    }

    #[test]
    fn capture_has_completer_detects_registration() {
        let registered = "<<FLYBEGIN>>\nget -- Display\n<<FLYCOMPDEF>>_kubectl\n<<FLYEND>>\n";
        assert!(capture_has_completer(registered));
        let empty_payload = "<<FLYBEGIN>>\n<<FLYCOMPDEF>>\n<<FLYEND>>\n";
        assert!(!capture_has_completer(empty_payload));
        let no_marker = "<<FLYBEGIN>>\nfoo\n<<FLYEND>>\n";
        assert!(!capture_has_completer(no_marker));
    }

    #[test]
    fn parse_capture_output_ignores_compdef_marker() {
        let blob = "<<FLYBEGIN>>\nget -- Display a resource\n<<FLYCOMPDEF>>_kubectl\n<<FLYEND>>\n";
        assert_eq!(
            parse_capture_output(blob),
            vec![("get".to_string(), Some("Display a resource".to_string()))],
        );
    }

    #[test]
    fn zsh_backend_synthesizes_zsh_completions() {
        assert!(matches!(
            ZSH_BACKEND.flycomp_output_format(),
            flycomp::OutputFormat::Zsh
        ));
    }

    #[test]
    fn bash_backend_synthesizes_bash_completions() {
        assert!(matches!(
            crate::shell::BashBackend.flycomp_output_format(),
            flycomp::OutputFormat::Bash
        ));
    }

    /// End-to-end: a `_<cmd>` file written outside the daemon's default fpath is
    /// registered via the broker reload verb (the `fly-reload` widget adds its dir
    /// to fpath + `compdef`s it), then the next capture returns its completions.
    #[test]
    fn broker_reload_registers_synthesized_completion() {
        if !zsh_available() {
            eprintln!("skipping broker reload: zsh not available");
            return;
        }
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let comp_dir =
            std::env::temp_dir().join(format!("flyline-comp-{}-{}", std::process::id(), stamp));
        std::fs::create_dir_all(&comp_dir).unwrap();
        let comp_file = comp_dir.join("_flytest");
        // Autoload-compatible compdef offering static subcommands (no binary call).
        std::fs::write(
            &comp_file,
            "#compdef flytest\n\
             _flytest() {\n\
               local -a subcmds\n\
               subcmds=(alpha beta gamma)\n\
               _describe 'command' subcmds\n\
             }\n\
             if [ \"$funcstack[1]\" = \"_flytest\" ]; then\n\
               _flytest \"$@\"\n\
             else\n\
               compdef _flytest flytest\n\
             fi\n",
        )
        .unwrap();

        let sock = std::env::temp_dir().join(format!(
            "flyline-test-reload-{}-{}.sock",
            std::process::id(),
            stamp
        ));
        let broker_sock = sock.clone();
        #[allow(clippy::disallowed_methods)]
        std::thread::spawn(move || {
            run_comp_broker(&broker_sock);
        });

        let connect = || -> Option<UnixStream> {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if let Ok(s) = UnixStream::connect(&sock) {
                    return Some(s);
                }
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        };

        // Reload request.
        let mut stream = connect().expect("connect for reload");
        stream
            .set_read_timeout(Some(BROKER_REQUEST_TIMEOUT))
            .unwrap();
        stream
            .write_all(format!("{RELOAD_SENTINEL}\n{}\n", comp_file.display()).as_bytes())
            .unwrap();
        stream.flush().unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(
            resp.contains(FLYRELOAD),
            "reload not acknowledged, got: {resp:?}"
        );

        // Capture request now sees the freshly registered completion.
        let mut stream = connect().expect("connect for capture");
        stream
            .set_read_timeout(Some(BROKER_REQUEST_TIMEOUT))
            .unwrap();
        let cwd = std::env::temp_dir();
        stream
            .write_all(format!("{}\nflytest \n", cwd.display()).as_bytes())
            .unwrap();
        stream.flush().unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        let hits = parse_capture_output(&resp);
        let values: Vec<&str> = hits.iter().map(|(v, _)| v.as_str()).collect();
        assert!(
            values.contains(&"alpha") && values.contains(&"beta"),
            "expected synthesized subcommands, got {values:?}"
        );
        assert!(
            capture_has_completer(&resp),
            "registered _flytest should be reported as a completer, got: {resp:?}"
        );

        // A command with no registered completer must not report one — that's the
        // signal that keeps flyline from offering flycomp only when nothing exists.
        let mut stream = connect().expect("connect for unknown-cmd capture");
        stream
            .set_read_timeout(Some(BROKER_REQUEST_TIMEOUT))
            .unwrap();
        stream
            .write_all(format!("{}\nzzznosuchcmd_flytest \n", cwd.display()).as_bytes())
            .unwrap();
        stream.flush().unwrap();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp2 = String::new();
        stream.read_to_string(&mut resp2).unwrap();
        assert!(
            !capture_has_completer(&resp2),
            "unregistered command should not report a completer, got: {resp2:?}"
        );

        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_dir_all(&comp_dir);
    }
}
