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
import select
import struct
import sys
import termios
import time

FLYLINE_FISH = os.environ.get("FLYLINE_FISH", "/opt/flyline/flyline.fish")
FLYLINE_BIN = os.environ.get("FLYLINE_BIN", "/usr/local/bin/flyline-standalone")

passed = 0
failed = 0


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
        os.write(fd, line.encode() + b"\r")
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


print("== source flyline.fish with flyline binary ==")
# $FISH_TEST_VAL only becomes 42 when fish executes the accepted line, and the
# variable reference avoids brackets (flyline auto-closes those while typing).
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN, "FISH_TEST_VAL": "42"},
    ["", "echo ENABLED_$FISH_TEST_VAL"],
    secs_per_line=4.0,  # first prompt boots the flyline TUI
)
check("flyline widget accepts and executes a command", "ENABLED_42", out)

print("== accepted command retains interactive stdin ==")
# Model package-manager confirmation prompts without installing anything. The
# accepted command must run through fish's normal reader so `read` owns the tty
# and receives the second submitted line.
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    ["", "sh -c 'read answer; echo INTERACTIVE_$answer'", "yes"],
    secs_per_line=4.0,
)
check("accepted command can read confirmation input", "INTERACTIVE_yes", out)

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
    ["", "echo SLOW1_$FISH_TEST_A", "", "echo SLOW2_$FISH_TEST_B"],
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

print("== flyline_disable restores prior Enter binding ==")
out = run_shell(
    {"FLYLINE_BIN": FLYLINE_BIN},
    ["flyline_disable", "echo BIND"],
    pre_source="bind --user \\r 'commandline -a _RESTORED; commandline -f execute'",
)
check("custom Enter binding restored after disable", "BIND_RESTORED", out)

print()
print(f"RESULT: {passed} passed, {failed} failed")
if failed:
    sys.exit(1)
print("SUCCESS: fish integration test completed")
