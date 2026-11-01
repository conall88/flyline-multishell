#!/usr/bin/env python3
"""End-to-end checks for scripts/flyline.fish inside an interactive fish pty.

Mirrors docker/zsh_integration_test.sh (fish has no zpty, so python's pty
module drives the shell). Typed keys go through the real flyline TUI when it
is active; the `(math ...)` substitutions only evaluate once fish executes the
accepted line, so matching the result proves execution, not just echo.
"""

import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

FLYLINE_FISH = os.environ.get("FLYLINE_FISH", "/opt/flyline/flyline.fish")
FLYLINE_BIN = os.environ.get("FLYLINE_BIN", "/usr/local/bin/flyline-standalone")

passed = 0
failed = 0


CTRL_C = "\x03"


def run_shell(env_overrides, lines, secs_per_line=1.5, reply_delay=0.0, pre_source=""):
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["HOME"] = env.get("HOME", "/root")
    env.update(env_overrides)

    pid, fd = pty.fork()
    if pid == 0:
        init = f"{pre_source}; source {FLYLINE_FISH}" if pre_source else f"source {FLYLINE_FISH}"
        os.execvpe("fish", ["fish", "-i", "-C", init], env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))

    blob = b""
    pending = []

    def pump(seconds):
        nonlocal blob
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.1)
            now = time.time()
            for item in list(pending):
                if now >= item[0]:
                    os.write(fd, item[1])
                    pending.remove(item)
            if not r:
                continue
            try:
                chunk = os.read(fd, 8192)
            except OSError:
                return
            if not chunk:
                return
            blob += chunk
            # Answer fish 4's terminal queries so startup doesn't stall.
            # reply_delay simulates a slow terminal: replies land while the
            # NEXT flyline holds the tty (the fish 4.x query-assert vector).
            if b"\x1b[c" in chunk or b"\x1b[0c" in chunk:
                pending.append((now + reply_delay, b"\x1b[?62c"))
            if b"\x1b[6n" in chunk:
                pending.append((now + reply_delay, b"\x1b[1;1R"))
            if b"\x1b]11;?" in chunk:
                pending.append((now + reply_delay, b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\"))

    pump(3.0)
    for line in lines:
        # Control keys are sent bare; everything else is a line ended by Enter.
        os.write(fd, line.encode() if line == CTRL_C else line.encode() + b"\r")
        pump(secs_per_line)
    pump(2.0)

    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    os.close(fd)
    return blob.decode(errors="replace")


def check(name, expected, blob):
    global passed, failed
    if expected in blob:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}  (expected: {expected})")
        failed += 1


def check_absent(name, pattern, blob):
    global passed, failed
    match = re.search(pattern, blob)
    if match is None:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}  (found: {match.group(0)!r})")
        failed += 1


print("== source flyline.fish with flyline binary ==")
# $FISH_TEST_VAL only becomes 42 when fish executes the accepted line, and the
# variable reference avoids brackets (flyline auto-closes those while typing).
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN, "FISH_TEST_VAL": "42"},
    ["echo ENABLED_$FISH_TEST_VAL"],
    secs_per_line=4.0,  # first prompt boots the flyline TUI
)
check("flyline widget accepts and executes a command", "ENABLED_42", out)
# fish 3.x's `set_color normal` emits terminfo sgr0 (`ESC ( B ESC [ m`); fish
# itself writes it raw, but flyline must not draw the `(B` as text.
check_absent("default prompt has no stray charset escape text", r"(?<!\x1b)\(B", out)

# Non-SGR escapes from a prompt: charset designations (incl. DEC line drawing)
# and cursor save/restore. The rendered text only appears once flyline draws
# it; fish's own copy of the prompt keeps the raw escapes between letters.
print("== prompt escape sequences render cleanly ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    [],
    secs_per_line=4.0,
    pre_source="function fish_prompt; printf 'PR\\e(B\\e[mOMPT\\e(0q\\e(B\\e7\\e8> '; end",
)
check("prompt drawn with escapes applied", "PROMPT\u2500>", out)

# Accepted lines run while fish's reader has the tty in raw settings. Commands
# doing a plain line read (apt/pacman confirmations, `read` in scripts) must
# still see Enter end the line, from stdin and from /dev/tty.
print("== accepted command can read confirmation input ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    ["sh -c 'read answer; echo INTERACTIVE_$answer'", "yes"],
    secs_per_line=4.0,
)
check("accepted command reads a line from stdin", "INTERACTIVE_yes", out)

print("== accepted command can read /dev/tty ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    ["sh -c 'read answer </dev/tty; echo TTY_$answer'", "yes"],
    secs_per_line=4.0,
)
check("accepted command reads a line from /dev/tty", "TTY_yes", out)

print("== Ctrl-C interrupts an accepted command ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN, "FISH_TEST_VAL": "42"},
    ["sleep 30", CTRL_C, "echo AFTERINT_$FISH_TEST_VAL"],
    secs_per_line=4.0,
)
check("prompt works after Ctrl-C", "AFTERINT_42", out)

# fish's reader expects its own tty settings back after an accepted command:
# with normal settings left on, Enter arrives as \n and a user's `\r` binding
# stops firing. `flyline_disable` itself is accepted through flyline here.
print("== reader tty settings restored after an accepted command ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    ["flyline_disable", "echo BIND"],
    secs_per_line=4.0,
    pre_source="bind \\r 'commandline -a _RESTORED; commandline -f execute'",
)
check("user Enter binding still fires", "BIND_RESTORED", out)

print("== fail-open: missing flyline binary ==")
out = run_shell(
    {"FLYLINE_BIN": "/no/such/flyline"},
    ["echo MISSING_(math 40 + 2)"],
)
check("native fish runs when binary missing", "MISSING_42", out)

print("== slow-terminal query replies (fish 4.x assert regression) ==")
# Regression guard for `assertion failed: query.is_none()` (fish reader.rs):
# delayed replies to fish's prompt-time terminal queries used to be eaten by
# the next flyline instance, wedging fish's query state and crashing fish.
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN, "FISH_TEST_A": "41", "FISH_TEST_B": "42"},
    ["echo SLOW1_$FISH_TEST_A", "echo SLOW2_$FISH_TEST_B"],
    secs_per_line=4.0,
    reply_delay=0.3,
)
check("first command executes under reply lag", "SLOW1_41", out)
check("second command executes under reply lag", "SLOW2_42", out)
if "panicked" in out:
    print("  FAIL: fish crashed under reply lag (query.is_none assert)")
    failed += 1
else:
    print("  PASS: fish does not crash under reply lag")
    passed += 1

print("== flyline_disable restores native fish ==")
out = run_shell(
    {"FLYLINE_BIN": "/no/such/flyline"},
    ["flyline_disable", "echo DISABLED_(math 43 - 1)"],
)
check("native fish runs after flyline_disable", "DISABLED_42", out)

print()
print(f"RESULT: {passed} passed, {failed} failed")
if failed:
    sys.exit(1)
print("SUCCESS: fish integration test completed")
