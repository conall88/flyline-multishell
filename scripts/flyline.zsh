# >>> flyline start >>>
# flyline runs as a separate process from a `zle-line-init` hook: it draws its TUI
# on the tty and returns the chosen line on fd 3. Fail-open — if flyline is
# missing, cancelled, or crashes, native ZLE handles the line unharmed.

[[ -n ${_FLYLINE_LOADED:-} ]] && return
typeset -g _FLYLINE_LOADED=1

# Default to flyline-standalone next to the install dir (parent of scripts/).
: ${FLYLINE_BIN:=${${(%):-%x}:A:h:h}/flyline-standalone}

_flyline_edit() {
  local last_exit=$?   # capture before emulate/anything clobbers $?
  emulate -L zsh
  [[ -x $FLYLINE_BIN ]] || return 0   # fail open

  # flyline reads history from $HISTFILE; flush this session's first.
  fc -AI 2>/dev/null || true

  # Hand flyline a prompt-format string it can `print -Pn` (don't pre-expand `%`;
  # that mangles starship's %{...%}). starship renders via its CLI; templates we
  # can't resolve here ($(...)/${...}, e.g. p10k) fall back to a minimal prompt.
  local fly_ps1=$PROMPT fly_rps1=$RPROMPT
  if (( $+commands[starship] )) && [[ $PROMPT == *starship* ]]; then
    fly_ps1="$(starship prompt --terminal-width=$COLUMNS --status=$last_exit 2>/dev/null)"
    fly_rps1="$(starship prompt --right --terminal-width=$COLUMNS --status=$last_exit 2>/dev/null)"
  fi
  [[ $fly_ps1 == *'${'* || $fly_ps1 == *'$('* || -z $fly_ps1 ]] && fly_ps1='%n@%m %1~ %# '
  [[ $fly_rps1 == *'${'* || $fly_rps1 == *'$('* ]] && fly_rps1=''

  local cmd rc
  # In a ZLE widget, command-substitution stdin is /dev/null, so read keys from
  # the tty (0</dev/tty). UI -> /dev/tty, chosen line -> fd 3 -> captured here.
  cmd="$( \
    FLYLINE_HOST=zsh \
    FLYLINE_INIT=$BUFFER \
    FLYLINE_LAST_EXIT=$last_exit \
    PS1=$fly_ps1 \
    RPS1=$fly_rps1 \
    "$FLYLINE_BIN" 0</dev/tty 3>&1 1>/dev/tty 2>/dev/tty \
  )"
  rc=$?

  if (( rc == 0 )); then
    # Accept even when empty, so line-init re-fires and relaunches flyline
    # (guarding on a non-empty buffer left blank Enter stuck in native ZLE).
    BUFFER=$cmd
    zle .redisplay
    zle .accept-line
    return 0
  elif (( rc == 130 )); then
    # Ctrl-C: abort to a fresh prompt (relaunches flyline) instead of raw ZLE.
    BUFFER=
    zle .send-break
    return 0
  else
    zle .redisplay
    return 0
  fi
}

flyline_enable() {
  autoload -Uz add-zle-hook-widget
  zle -N _flyline_edit
  add-zle-hook-widget line-init _flyline_edit
}

flyline_disable() {
  add-zle-hook-widget -d line-init _flyline_edit 2>/dev/null
  zle -D _flyline_edit 2>/dev/null
  unset _FLYLINE_LOADED
}

flyline_uninstall() {
  flyline_disable
  unset FLYLINE_BIN
}

flyline_enable
# <<< flyline end <<<
