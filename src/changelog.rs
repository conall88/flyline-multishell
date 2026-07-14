pub(crate) const CHANGELOG: &str = r#"# Changelog

# flyline-multishell fork

## v1.1.0
- **flycomp offers for uncompleted commands**: zsh Tab completion now tells apart "no completer at all" from "a command-specific completer that legitimately found nothing," using a new `<<FLYSPECIFIC>>` provenance marker from the completion daemon. Commands like `claude` that previously fell through to zsh's generic filename fallback (silently listing the current directory) now get the flycomp installation-synthesis offer instead, while commands with a real completer (e.g. `kubectl get` against no cluster) stay silent as before.
- **Show files / Don't ask again**: The flycomp prompt shown for this generic fallback case adds two new options alongside the existing synthesis offer: **Show files** reveals the retained filename candidates without synthesizing anything, and **Don't ask again** blacklists the command and then shows those same files.

## v1.0.2
- **Fixed a shell hang when running `exec zsh` after re-install (only)**: The standalone zsh editor now claims the controlling terminal's foreground process group before drawing, so running `exec zsh` in the same terminal right after a (re)install no longer stops flyline on `SIGTTOU` and wedges the shell until a new tab is opened.
- **improved mouse escape sequence handling**: makes sure mouse capture is enabled only after the viewport is established.
- **Reliable terminal restore on interruption**: The standalone zsh editor now restores the terminal (raw mode, mouse tracking, bracketed paste, cursor) if it is killed by a fatal signal such as `SIGHUP`/`SIGTERM`, so an interrupted edit can no longer leave the terminal emitting stray escape sequences.

## v1.0.1
- **Complete Uninstall**: `install.sh --uninstall` now removes both Bash and zsh startup integration along with installed executables, libraries, scripts, and release metadata.
- **Clear Uninstall Guidance**: The installer reports exactly what it removed and explains how to unload commands already resident in existing Bash and zsh sessions.
- **Installation Documentation**: Restored concise quick-install, Arch Linux, release-download, and source-build options with multishell-specific guidance.

## v1.0.0
- **Zsh Support**: Added first-class Zsh integration via a `zle-line-init` hook that launches flyline as a separate process, with fail-open fallback to native ZLE when the binary is missing or flyline is cancelled/crashes.
- **Standalone Binary**: Introduced the `flyline-standalone` executable so flyline's line editor can run outside the in-process Bash builtin, driving both Bash and Zsh from one codebase.
- **Multishell**: Unified Bash and Zsh under a single multishell architecture, reusing your existing shell setup (e.g. `~/.zshrc`, `compinit` functions, oh-my-zsh plugins) for completions rather than reinventing them.
- **flycomp Option Synthesis**: Extended automatic completion synthesis so flycomp-generated option/flag specs are surfaced as suggestions when a command lacks a native completion script.

# Upstream (HalFrgrd/flyline)

## v1.3.0
- **Leader Keys**: Added support for chorded keybinding sequences (e.g., `Ctrl+x` followed by `Ctrl+f`) via the new `setLeaderKey` and `unsetLeaderKey` actions and the `leaderKeyActive` context variable.
- **Leader Key Visual Feedback**: Introduced the `leader-mode` prompt widget to display visual indicators (like ` X `) in the prompt when the leader key state is active.
- **String Insertion Action**: `insertString(...)` action allows inserting arbitrary strings into the buffer.
- **Strict Modifier Matching**: Switched to strict modifier equality matching to prevent modifier-overlap conflicts when dispatching key actions.
- **Key List Autocomplete & Completion**: Added autocomplete support for listing keybindings for a specific key event (`flyline key list <key>`).

## v1.2.5
- **Global Allocator**: Integrated `mimalloc` to bypass Bash's non-thread-safe allocator and prevent heap corruption on multi-threaded allocations.
- **Nested Arithmetic Lexing**: Stateful lexing updates to correctly parse nested brackets/parentheses inside arithmetic `$(( ... ))` blocks.
- **Word Under Cursor breaks**: Updated word-under-cursor (WUC) detection to respect `:` and `=`, matching bash's standard `COMP_WORD_BREAK` behavior.
- **Kitty Cursor Support**: Added backend selection to keep the terminal emulator cursor visible on Kitty, preventing prompts when closing the window.

## v1.2.4
- **Safety Guards**: Fixed a Use-After-Free (UAF) issue, added safety guards, and enforced usage of the thread manager.
- **Mouse UX Improvements**: Corrected mouse event output formatting and resolved layout bugs, ensuring mouse event rows are always fully printed.
- **Robust WUC Handling**: Patched Word Under Cursor (WUC) edge cases and downgraded internal assertions to errors to prevent shell crashes.
- **AUR Package**: Documented and referenced the official Arch Linux User Repository (AUR) package.
- **Cleanups**: Removed the legacy `get_current_readline_prompt` hook dependency to streamline FFI interactions.

## v1.2.3
- **Thread Safety**: Added `BASH_LOCK` to prevent concurrency crashes when accessing Bash FFI from background threads.
- **Log Forwarding**: Pipes tab-completion child logs back to the parent to prevent double-logging and preserve trails.
- **Fuzzy Mode**: Added `flyline suggestions set-fuzzy-mode` (`all`, `none`, `folder-prefixes`) for folder prefix matching.

## v1.2.2
- **Changelog Command**: Added `flyline changelog` command to display user-facing changelogs directly in the pager.
- **Upgrade Assistant**: Added `flyline upgrade` command which pre-fills the prompt line with the curl installer command.
- **Installer improvements**: Streamlined `install.sh` to run non-interactively, resolving target folders automatically.

## v1.2.1
- **Declarative Mouse Actions**: Re-architected mouse event processing into a declarative, context-aware routing system.
- **Tab Completion Latency**: Reduced visual flashing during tab completion redraws and optimized filtering latency for large lists.
- **Offline Installer**: Updated `install.sh` to bypass GitHub API rate limits by resolving release redirect headers.
- **Wider Platform Support**: Added release builds for FreeBSD, ARMv7, 32-bit x86, RISC-V 64, and PowerPC 64 LE.
- **OSC 52 Paste**: Replaced custom OSC 52 querying with crossterm's native RequestClipboardContents.

## v1.2.0
- **Transient Prompts**: Added support for transient prompts, reducing terminal noise by condensing past prompts upon execution.
- **History Management**: Introduced separate history managers for cancelled commands and agent prompts.
- **Non-blocking Completion**: Improved tab-completion responsiveness by spawning completion generation in a dedicated process.
- **Scroll & Right-Click UX**: Enhanced right-click context menu and continuous proportional scrollbar dragging.

## v1.1.0
- **Fuzzy Sorting**: Introduced suggestion sorting algorithms (mtime, alphabetical) and CLI configuration options.
- **Improved Parsing**: Enhanced flycomp parsing for cargo, git --help, and flag values ending in `=`.
- **Fuzzy Matching**: Tightened fuzzy suggestion matching and fixed scrollbar positions.

## v1.0.0
- **Stable Line Editor**: First major release of the Rust-based GNU readline replacement builtin for Bash.
- **Mouse Selection**: Support for cursor placement and visual drag-selections using mouse.
- **Auto-Closing pairs**: Automatic insertion of closing quotes, brackets, and parentheses.
- **Interactive Tutorial**: Added an in-terminal tutorial to guide users through keyboard and mouse controls.
"#;
