#!/bin/zsh
# End-to-end checks for scripts/flyline.zsh inside an interactive zsh pty.
set -euo pipefail

zmodload zsh/zpty
zmodload zsh/datetime

FLYLINE_ZSH="${FLYLINE_ZSH:-/opt/flyline/flyline.zsh}"
FLYLINE_BIN="${FLYLINE_BIN:-/usr/local/bin/flyline-standalone}"

integer pass=0 fail=0
typeset -g _BLOB

_pump() {
  local end=$(( EPOCHREALTIME + $1 )) chunk
  while (( EPOCHREALTIME < end )); do
    if zpty -r -t Z chunk 2>/dev/null; then
      _BLOB+=$chunk
    else
      sleep 0.05
    fi
  done
}

run_shell() {
  local -a lines; lines=("$@")
  _BLOB=""
  zpty Z zsh -f -i
  _pump 0.5
  local l
  for l in $lines; do
    zpty -w -n Z "$l"$'\r'
    # The first flyline-backed line may need to initialize the standalone
    # editor and completion daemon. A fixed sub-second delay was flaky on
    # cold CI builders and arm64 Docker hosts.
    _pump 2.0
  done
  _pump 0.6
  zpty -d Z 2>/dev/null
  print -r -- "${_BLOB//$'\r'/}"
}

check() {
  if [[ $3 == *$2* ]]; then
    print -r -- "  PASS: $1"
    (( ++pass ))
  else
    print -r -- "  FAIL: $1  (expected: $2)"
    print -r -- "  captured output: ${(qqq)3}"
    (( ++fail ))
  fi
}

print "== source flyline.zsh + flyline_enable =="
out=$(run_shell \
  "export FLYLINE_BIN=$FLYLINE_BIN" \
  "source $FLYLINE_ZSH" \
  "flyline_enable" \
  "print ENABLED_42" \
  "exit")
check "flyline_enable keeps native ZLE working" "ENABLED_42" "$out"

print "== fail-open: missing flyline binary =="
out=$(run_shell \
  "export FLYLINE_BIN=/no/such/flyline" \
  "source $FLYLINE_ZSH" \
  "flyline_enable" \
  "print MISSING_42" \
  "exit")
check "native ZLE runs when binary missing" "MISSING_42" "$out"

print "== flyline_disable restores native ZLE =="
out=$(run_shell \
  "export FLYLINE_BIN=$FLYLINE_BIN" \
  "source $FLYLINE_ZSH" \
  "flyline_disable" \
  "print DISABLED_42" \
  "exit")
check "native ZLE runs after flyline_disable" "DISABLED_42" "$out"

print ""
print "RESULT: $pass passed, $fail failed"
(( fail == 0 )) || exit 1
print "SUCCESS: zsh integration test completed"
