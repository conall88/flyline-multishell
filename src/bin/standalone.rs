//! Standalone flyline editor for zsh host integration.
//!
//! Launched from the `zle-line-init` widget in `scripts/flyline.zsh`. Draws the
//! TUI on `/dev/tty` and writes the accepted command line to fd 3. Exit codes:
//!   0   — command accepted
//!   130 — cancelled (Ctrl-C / empty abort)
//!   1   — EOF or internal error

use flyline::{
    ExitState, Settings, ZSH_BACKEND, get_command, init_standalone_logging, run_comp_broker,
    set_backend, set_cloexec,
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
    // Mark the fd 3 handoff pipe close-on-exec so helper grandchildren can't hold
    // it open and wedge the parent's `$( ... 3>&1 )` (write_command_fd3 still works).
    set_cloexec(3);
    set_backend(&ZSH_BACKEND);

    if let Err(e) = init_standalone_logging() {
        eprintln!("flyline: failed to initialize logging: {e}");
    }

    let mut settings = Settings::default();
    if let Ok(init) = std::env::var("FLYLINE_INIT") {
        settings.initial_buffer = Some(init);
    }

    match get_command(&mut settings) {
        ExitState::WithCommand(cmd) => {
            write_command_fd3(&cmd);
            0
        }
        ExitState::WithoutCommand => 130,
        ExitState::EOF => 1,
    }
}

fn main() {
    let code = match catch_unwind_safe(run) {
        Ok(code) => code,
        Err(()) => {
            eprintln!(
                "flyline: panicked; please report at https://github.com/HalFrgrd/flyline/issues"
            );
            1
        }
    };
    std::process::exit(code);
}
