//! Regression guard: `flyline-standalone` must never hang or spin at startup.
//!
//! A `#[no_mangle] getenv` stub in `bash_stubs.rs` once interposed libc's
//! `getenv` and recursed infinitely (100% CPU) the first time `std::env::var`
//! ran during `Settings::default()`, wedging the host zsh with a blank,
//! unresponsive terminal. This runs the real binary headless (stdin =
//! /dev/null) and asserts it exits promptly instead of hanging.
//!
//! Skips when the binary isn't built (it needs `--features standalone`).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn standalone_exits_promptly_without_a_terminal() {
    let Some(bin) = option_env!("CARGO_BIN_EXE_flyline-standalone") else {
        eprintln!("skipping: flyline-standalone not built (needs --features standalone)");
        return;
    };

    let mut child = Command::new(bin)
        .env("FLYLINE_HOST", "zsh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn flyline-standalone");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return; // exited on its own — no startup hang/spin
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("flyline-standalone did not exit within 10s (startup hang/spin regression)");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
