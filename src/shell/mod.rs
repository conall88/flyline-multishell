//! Host shell abstraction. `ShellBackend` is the seam over the original Bash
//! `bash_funcs` FFI that lets a second host (zsh) plug in: the app talks to
//! `shell::backend()` instead of Bash-specific free functions.

use std::path::PathBuf;
use std::sync::OnceLock;

pub use crate::bash_funcs::{CommandWordInfo, ProgrammableCompleteReturn};

pub mod zsh;

/// One entry from the host shell's in-memory command history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRecord {
    pub timestamp: Option<u64>,
    pub index: usize,
    pub command: String,
}

/// The host shell `flyline` is embedded in.
///
/// Implementors must be `Sync`: a single backend is shared, by `&'static`
/// reference, across flyline's threads.
pub trait ShellBackend: Sync {
    /// True when flyline runs as a Bash loadable builtin; false for standalone zsh.
    fn is_bash(&self) -> bool {
        true
    }

    /// Working directory as the shell sees it (used for prompt + OSC 7 reporting).
    fn cwd(&self) -> String;

    /// Value of a shell/environment variable, if set.
    fn env_var(&self, name: &str) -> Option<String>;

    /// Host name for the prompt / shell-integration OSC sequences.
    fn hostname(&self) -> String;

    /// A variable rendered for display (`name=value` plus any attributes), as
    /// shown in the variable tooltip.
    fn format_var(&self, name: &str) -> String;

    /// Names (each `$`-prefixed) of variables whose name starts with `prefix`
    /// (a leading `$` on `prefix` is ignored). Drives `$VAR` completion.
    fn vars_with_prefix(&self, prefix: &str) -> Vec<String>;

    /// Expand a path through the shell (tilde, env vars, relative → absolute).
    fn expand_path(&self, path: &str) -> String;

    /// Expand a filename through the shell's string expansion rules.
    fn expand_filename(&self, filename: &str) -> String;

    /// Run a prompt string through the host shell's prompt decoder.
    fn decode_prompt(&self, raw: &str, is_prompt: bool) -> Option<String>;

    /// Look up a shell alias definition for `cmd`, if one exists.
    fn find_alias(&self, cmd: &str) -> Option<String>;

    /// Classify a command word (alias, builtin, file on PATH, etc.).
    fn command_info(&self, cmd: &str) -> CommandWordInfo;

    /// Run programmable tab-completion for the given context.
    fn run_programmable_completions(
        &self,
        full_command: &str,
        command_word: &str,
        word_under_cursor: &str,
        cursor_byte_pos: usize,
        word_under_cursor_byte_end: usize,
    ) -> anyhow::Result<ProgrammableCompleteReturn>;

    /// All potential first-word completions (aliases, keywords, functions, etc.).
    fn possible_command_words(&self) -> Vec<CommandWordInfo>;

    /// Evaluate a shell script string in the host shell context.
    fn evaluate_shell_string(&self, script: &str) -> anyhow::Result<()>;

    /// Clear host-shell caches (command info, aliases, etc.).
    fn reset_caches(&self);

    /// Pre-warm completion caches in a background-friendly way.
    fn warm_completion_caches(&self);

    /// Non-zero when the host shell received a terminating signal.
    fn read_terminating_signal(&self) -> libc::c_int;

    /// Resolve where a synthesized completion script should be written.
    fn resolve_completion_script_path(
        &self,
        command_word: &str,
        flycomp_output: Option<&str>,
    ) -> PathBuf;

    /// Write a synthesized completion script to the resolved path.
    fn resolve_and_write_completion_script(
        &self,
        command_word: &str,
        script: &str,
        flycomp_output: Option<&str>,
    ) -> Result<PathBuf, std::io::Error>;

    /// Read command history from the host shell's in-memory list.
    fn parse_history_from_memory(&self) -> Vec<HistoryRecord>;

    /// Exit status of the last command run in the shell.
    fn last_command_exit_status(&self) -> i32;

    /// Number of lines in the current (possibly unfinished) multiline command.
    fn multiline_command_count(&self) -> i32;

    /// Process group ID of the shell (used in shell-integration OSC sequences).
    fn shell_pgrp(&self) -> libc::pid_t;
}

/// The original Bash host; delegates to `bash_funcs`, adding no new behavior.
pub struct BashBackend;

impl ShellBackend for BashBackend {
    fn is_bash(&self) -> bool {
        true
    }

    fn cwd(&self) -> String {
        crate::bash_funcs::get_cwd()
    }

    fn env_var(&self, name: &str) -> Option<String> {
        crate::bash_funcs::get_envvar_value(name)
    }

    fn hostname(&self) -> String {
        crate::bash_funcs::get_hostname()
    }

    fn format_var(&self, name: &str) -> String {
        crate::bash_funcs::format_shell_var(name)
    }

    fn vars_with_prefix(&self, prefix: &str) -> Vec<String> {
        crate::bash_funcs::get_all_variables_with_prefix(prefix)
    }

    fn expand_path(&self, path: &str) -> String {
        crate::bash_funcs::fully_expand_path(path)
    }

    fn expand_filename(&self, filename: &str) -> String {
        crate::bash_funcs::expand_filename(filename)
    }

    fn decode_prompt(&self, raw: &str, is_prompt: bool) -> Option<String> {
        bash_decode_prompt(raw, is_prompt)
    }

    fn find_alias(&self, cmd: &str) -> Option<String> {
        crate::bash_funcs::find_alias(cmd)
    }

    fn command_info(&self, cmd: &str) -> CommandWordInfo {
        crate::bash_funcs::get_command_info(cmd)
    }

    fn run_programmable_completions(
        &self,
        full_command: &str,
        command_word: &str,
        word_under_cursor: &str,
        cursor_byte_pos: usize,
        word_under_cursor_byte_end: usize,
    ) -> anyhow::Result<ProgrammableCompleteReturn> {
        crate::bash_funcs::run_programmable_completions(
            full_command,
            command_word,
            word_under_cursor,
            cursor_byte_pos,
            word_under_cursor_byte_end,
        )
    }

    fn possible_command_words(&self) -> Vec<CommandWordInfo> {
        crate::bash_funcs::get_possible_command_words().collect()
    }

    fn evaluate_shell_string(&self, script: &str) -> anyhow::Result<()> {
        crate::bash_funcs::evaluate_shell_string(script)
    }

    fn reset_caches(&self) {
        crate::bash_funcs::reset_caches();
    }

    fn warm_completion_caches(&self) {
        crate::bash_funcs::warm_completion_caches();
    }

    fn read_terminating_signal(&self) -> libc::c_int {
        crate::bash_funcs::read_terminating_signal()
    }

    fn resolve_completion_script_path(
        &self,
        command_word: &str,
        flycomp_output: Option<&str>,
    ) -> PathBuf {
        crate::bash_funcs::resolve_completion_script_path(command_word, flycomp_output)
    }

    fn resolve_and_write_completion_script(
        &self,
        command_word: &str,
        script: &str,
        flycomp_output: Option<&str>,
    ) -> Result<PathBuf, std::io::Error> {
        crate::bash_funcs::resolve_and_write_completion_script(command_word, script, flycomp_output)
    }

    fn parse_history_from_memory(&self) -> Vec<HistoryRecord> {
        parse_bash_history_from_memory()
    }

    fn last_command_exit_status(&self) -> i32 {
        read_last_command_exit_status()
    }

    fn multiline_command_count(&self) -> i32 {
        read_multiline_command_count()
    }

    fn shell_pgrp(&self) -> libc::pid_t {
        read_shell_pgrp()
    }
}

#[cfg(not(test))]
fn bash_decode_prompt(raw: &str, is_prompt: bool) -> Option<String> {
    if raw.is_empty() {
        return Some(String::new());
    }

    let c_prompt = std::ffi::CString::new(raw).ok()?;
    let _guard = crate::bash_symbols::BASH_LOCK.lock();

    let decoded = unsafe {
        #[cfg(not(feature = "pre_bash_4_4"))]
        let decoded_prompt_cstr =
            crate::bash_symbols::decode_prompt_string(c_prompt.as_ptr(), is_prompt as i32);
        #[cfg(feature = "pre_bash_4_4")]
        let decoded_prompt_cstr = crate::bash_symbols::decode_prompt_string(c_prompt.as_ptr());
        if decoded_prompt_cstr.is_null() {
            log::warn!("decode_prompt_string returned null");
            return None;
        }

        let decoded = std::ffi::CStr::from_ptr(decoded_prompt_cstr)
            .to_str()
            .ok()?
            .to_string();

        crate::bash_symbols::locked_xfree(decoded_prompt_cstr as *mut std::ffi::c_void);
        decoded
    };

    Some(decoded)
}

#[cfg(test)]
fn bash_decode_prompt(raw: &str, _is_prompt: bool) -> Option<String> {
    if raw.is_empty() {
        Some(String::new())
    } else {
        Some(raw.to_string())
    }
}

#[cfg(not(test))]
fn parse_bash_history_from_memory() -> Vec<HistoryRecord> {
    let mut res = Vec::with_capacity(4096);
    unsafe {
        let hist_array = crate::bash_symbols::history_list();
        if hist_array.is_null() {
            log::warn!("History list is null");
            return res;
        }

        let mut index = 0;
        loop {
            let entry_ptr = *hist_array.offset(index);
            if entry_ptr.is_null() {
                break;
            }

            let hist_entry = &*entry_ptr;

            if !hist_entry.line.is_null() {
                let command_cstr = std::ffi::CStr::from_ptr(hist_entry.line);
                let command_str = command_cstr.to_string_lossy().into_owned();

                let timestamp = if !hist_entry.timestamp.is_null() {
                    let timestamp_cstr = std::ffi::CStr::from_ptr(hist_entry.timestamp);
                    if let Ok(timestamp_str) = timestamp_cstr.to_str() {
                        let ts_str = timestamp_str.trim_start_matches('#').trim();
                        ts_str.parse::<u64>().ok()
                    } else {
                        None
                    }
                } else {
                    None
                };

                res.push(HistoryRecord {
                    timestamp,
                    index: index as usize,
                    command: command_str,
                });
            }

            index += 1;
        }
    }
    res
}

#[cfg(test)]
fn parse_bash_history_from_memory() -> Vec<HistoryRecord> {
    Vec::new()
}

#[cfg(not(test))]
fn read_last_command_exit_status() -> i32 {
    unsafe { crate::bash_symbols::last_command_exit_value }
}

#[cfg(test)]
fn read_last_command_exit_status() -> i32 {
    0
}

#[cfg(not(test))]
fn read_multiline_command_count() -> i32 {
    unsafe { crate::bash_symbols::current_command_line_count }
}

#[cfg(test)]
fn read_multiline_command_count() -> i32 {
    0
}

#[cfg(not(test))]
fn read_shell_pgrp() -> libc::pid_t {
    unsafe { crate::bash_symbols::shell_pgrp }
}

#[cfg(test)]
fn read_shell_pgrp() -> libc::pid_t {
    0
}

static BASH: BashBackend = BashBackend;
static ACTIVE: OnceLock<&'static dyn ShellBackend> = OnceLock::new();

/// True when `FLYLINE_HOST=zsh` selects the standalone zsh backend.
pub fn is_zsh_host_env() -> bool {
    std::env::var("FLYLINE_HOST").as_deref() == Ok("zsh")
}

fn init_backend() -> &'static dyn ShellBackend {
    if is_zsh_host_env() {
        &zsh::ZSH_BACKEND
    } else {
        &BASH
    }
}

/// The active host shell backend. Defaults to Bash unless `FLYLINE_HOST=zsh`.
pub fn backend() -> &'static dyn ShellBackend {
    *ACTIVE.get_or_init(init_backend)
}

/// Select the host backend once, at load, before the first `backend()` call.
/// ponytail: single process-global backend, matching flyline's global shell state.
#[allow(dead_code)]
pub fn set_backend(b: &'static dyn ShellBackend) {
    let _ = ACTIVE.set(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The seam must be behavior-neutral: `backend()` routes through the same
    // `bash_funcs` calls (test fixtures under cfg(test)) the old call sites used.

    #[test]
    fn default_backend_is_bash() {
        assert!(backend().is_bash());
    }

    #[test]
    fn default_backend_delegates_hostname() {
        assert_eq!(backend().hostname(), crate::bash_funcs::get_hostname());
        assert_eq!(backend().hostname(), "test-host");
    }

    #[test]
    fn default_backend_delegates_cwd() {
        assert_eq!(backend().cwd(), crate::bash_funcs::get_cwd());
    }

    #[test]
    fn default_backend_delegates_known_env_var() {
        assert_eq!(backend().env_var("USER"), Some("john".to_string()));
        assert_eq!(
            backend().env_var("PATH"),
            crate::bash_funcs::get_envvar_value("PATH"),
        );
    }

    #[test]
    fn default_backend_returns_none_for_unset_env_var() {
        assert_eq!(backend().env_var("FLYLINE_DEFINITELY_UNSET_VAR"), None);
    }

    #[test]
    fn default_backend_delegates_format_var() {
        assert_eq!(
            backend().format_var("FOO"),
            crate::bash_funcs::format_shell_var("FOO"),
        );
    }

    #[test]
    fn default_backend_delegates_vars_with_prefix() {
        assert_eq!(
            backend().vars_with_prefix("P"),
            crate::bash_funcs::get_all_variables_with_prefix("P"),
        );
        assert_eq!(backend().vars_with_prefix("$P"), vec!["$PATH", "$PWD"]);
    }

    #[test]
    fn default_backend_delegates_expand_path() {
        assert_eq!(
            backend().expand_path("."),
            crate::bash_funcs::fully_expand_path("."),
        );
    }

    #[test]
    fn default_backend_delegates_expand_filename() {
        assert_eq!(
            backend().expand_filename("$HOME"),
            crate::bash_funcs::expand_filename("$HOME"),
        );
    }

    #[test]
    fn default_backend_delegates_decode_prompt() {
        assert_eq!(
            backend().decode_prompt("\\u@\\h", true),
            bash_decode_prompt("\\u@\\h", true),
        );
    }

    #[test]
    fn default_backend_delegates_find_alias() {
        assert_eq!(
            backend().find_alias("gd"),
            crate::bash_funcs::find_alias("gd"),
        );
        assert_eq!(backend().find_alias("gd"), Some("git diff".to_string()));
    }

    #[test]
    fn default_backend_delegates_command_info() {
        assert_eq!(
            backend().command_info("git"),
            crate::bash_funcs::get_command_info("git"),
        );
    }

    #[test]
    fn default_backend_delegates_run_programmable_completions() {
        let via_backend = backend()
            .run_programmable_completions("git comm", "git", "comm", 4, 8)
            .unwrap();
        let via_bash =
            crate::bash_funcs::run_programmable_completions("git comm", "git", "comm", 4, 8)
                .unwrap();
        assert_eq!(via_backend.completions, via_bash.completions);
    }

    #[test]
    fn default_backend_delegates_possible_command_words() {
        let via_backend = backend().possible_command_words();
        let via_bash: Vec<_> = crate::bash_funcs::get_possible_command_words().collect();
        assert_eq!(via_backend, via_bash);
    }

    #[test]
    fn default_backend_delegates_evaluate_shell_string() {
        assert!(backend().evaluate_shell_string("true").is_ok());
        assert!(crate::bash_funcs::evaluate_shell_string("true").is_ok());
    }

    #[test]
    fn default_backend_delegates_reset_and_warm_caches() {
        backend().reset_caches();
        crate::bash_funcs::reset_caches();
        backend().warm_completion_caches();
        crate::bash_funcs::warm_completion_caches();
    }

    #[test]
    fn default_backend_delegates_read_terminating_signal() {
        assert_eq!(
            backend().read_terminating_signal(),
            crate::bash_funcs::read_terminating_signal(),
        );
    }

    #[test]
    fn default_backend_delegates_resolve_completion_script_path() {
        assert_eq!(
            backend().resolve_completion_script_path("git", None),
            crate::bash_funcs::resolve_completion_script_path("git", None),
        );
    }

    #[test]
    fn default_backend_delegates_parse_history_from_memory() {
        assert_eq!(
            backend().parse_history_from_memory(),
            parse_bash_history_from_memory(),
        );
    }

    #[test]
    fn default_backend_delegates_last_command_exit_status() {
        assert_eq!(
            backend().last_command_exit_status(),
            read_last_command_exit_status(),
        );
    }

    #[test]
    fn default_backend_delegates_multiline_command_count() {
        assert_eq!(
            backend().multiline_command_count(),
            read_multiline_command_count(),
        );
    }

    #[test]
    fn default_backend_delegates_shell_pgrp() {
        assert_eq!(backend().shell_pgrp(), read_shell_pgrp());
    }
}
