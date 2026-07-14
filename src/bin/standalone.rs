//! Standalone flyline editor for zsh host integration.
//!
//! Launched from the `zle-line-init` widget in `scripts/flyline.zsh`. Draws the
//! TUI on `/dev/tty` and writes the accepted command line to fd 3. Exit codes:
//!   0   — command accepted
//!   130 — cancelled (Ctrl-C / empty abort)
//!   1   — EOF or internal error

use flyline::{
    ExitState, StandaloneTerminalGuard, ZSH_BACKEND, backend, get_command, init_standalone_logging,
    run_comp_broker, run_flyline_command, set_backend, set_cloexec,
};

fn catch_unwind_safe<T>(f: impl FnOnce() -> T) -> Result<T, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| ())
}

fn write_command_fd3(cmd: &str) {
    use std::io::Write;
    use std::os::fd::{FromRawFd, RawFd};

    // ponytail: fd 3 is owned by the zsh parent; never close it on drop.
    let fd = RawFd::from(3);
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let _ = file.write_all(cmd.as_bytes());
    let _ = file.write_all(b"\n");
    std::mem::forget(file);
}

fn run() -> i32 {
    // Broker mode: serve completions over a Unix socket instead of editing a line.
    // Detached from the tty, so it must not touch fd 3 or FLYLINE_HOST/UI state.
    if let Some(sock) = std::env::var_os("FLYLINE_COMP_BROKER") {
        set_backend(&ZSH_BACKEND);
        let _ = init_standalone_logging();
        return run_comp_broker(std::path::Path::new(&sock));
    }

    // SAFETY: standalone is a fresh process; no other threads read these yet.
    unsafe {
        std::env::set_var("FLYLINE_HOST", "zsh");
    }
    set_backend(&ZSH_BACKEND);

    // Subcommand dispatch: `flyline <args>` (invoked via the shell forwarding
    // function) runs the same CLI as the Bash builtin against the persisted
    // settings snapshot, then exits without drawing the editor or touching fd 3.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if !argv.is_empty() {
        let _ = init_standalone_logging();
        let mut settings = backend().load_persisted_settings();
        // Overlay per-terminal session state (e.g. `flyline run-tutorial` sets
        // the tutorial running for the subsequent prompts of this terminal).
        settings.apply_session_state(&backend().load_session_state());
        let arg_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let code = run_flyline_command(&mut settings, &arg_refs);
        backend().persist_settings(&settings);
        backend().persist_session_state(&settings.session_state());
        return code;
    }

    // Mark the fd 3 handoff pipe close-on-exec so helper grandchildren can't hold
    // it open and wedge the parent's `$( ... 3>&1 )` (write_command_fd3 still works).
    set_cloexec(3);

    if let Err(e) = init_standalone_logging() {
        eprintln!("flyline: failed to initialize logging: {e}");
    }

    let mut settings = backend().load_persisted_settings();
    // Overlay per-terminal session state (tutorial progress) so the tutorial
    // advances across this terminal's per-prompt processes, like the long-lived
    // Bash builtin — without leaking into other terminals or the durable config.
    settings.apply_session_state(&backend().load_session_state());
    if let Ok(init) = std::env::var("FLYLINE_INIT") {
        settings.initial_buffer = Some(init);
    }

    // Claim the controlling terminal's foreground process group before drawing.
    // Without this, a wrong foreground group at launch (seen right after a
    // re-install, when `exec zsh` restarts the shell alongside orphaned helper
    // daemons) makes the first tty write raise SIGTTOU and stops us forever,
    // hanging the parent shell. The guard also installs fatal-signal handlers
    // that restore the terminal (raw mode, mouse tracking, ...) if the editor is
    // killed, so it never leaves the tty in a mode that leaks into later shells.
    // The Bash builtin gets both for free via Bash. Held until the editor
    // returns, then dropped.
    let _terminal_guard = StandaloneTerminalGuard::install(settings.enable_extended_key_codes);

    let exit_code = match get_command(&mut settings) {
        ExitState::WithCommand(cmd) => {
            // Progress the tutorial on empty submit, matching the Bash builtin
            // (which does this between prompts in its long-lived process).
            settings.advance_tutorial_on_submit(&cmd);
            write_command_fd3(&cmd);
            0
        }
        ExitState::WithoutCommand => 130,
        ExitState::EOF => 1,
    };

    // Persist durable settings, plus session-scoped tutorial progress for the
    // next prompt in this terminal.
    backend().persist_settings(&settings);
    backend().persist_session_state(&settings.session_state());
    exit_code
}

fn main() {
    let code = match catch_unwind_safe(run) {
        Ok(code) => code,
        Err(()) => {
            eprintln!(
                "flyline: panicked; please report at https://github.com/conall88/flyline-multishell/issues"
            );
            1
        }
    };
    std::process::exit(code);
}
