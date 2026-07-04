#!/bin/zsh
# Headless completion daemon for flyline, sourced after the user's rc. Rust drives
# it by writing "buffer<TAB>" to the pty and reads back the compadd-captured hits.

# No `emulate -L zsh`: at top level that would clobber the user's completion setup.
PROMPT=
RPROMPT=
# Idempotent compinit (-C skips the security check); required under NO_RCS.
autoload -Uz compinit
(( $+functions[compinit] )) && compinit -C -d "${ZDOTDIR:-$HOME}/.zcompdump_flyline"

bindkey '^M' undefined
bindkey '^J' undefined
bindkey '^I' complete-word

# Broker cwd change: caller types a path then sends ^F. Only cds — command
# execution stays disabled (^M/^J are `undefined`) — so completions match the dir.
fly-cd () { builtin cd -q -- "$BUFFER" 2>/dev/null; BUFFER=''; print -r -- '<<FLYCD>>' }
zle -N fly-cd
bindkey '^F' fly-cd

fly-begin () { compprefuncs=( fly-begin ); print -r -- '<<FLYBEGIN>>' }
fly-end   () { comppostfuncs=( fly-end ); print -r -- '<<FLYEND>>' }
compprefuncs=( fly-begin )
comppostfuncs=( fly-end )

zstyle ':completion:*' list-grouped false
zstyle ':completion:*' insert-tab false
zstyle ':completion:*' list-separator ''

zmodload zsh/zutil
compadd () {
  if [[ ${@[1,(i)(-|--)]} == *-(O|A|D)\ * ]]; then
    builtin compadd "$@"
    return $?
  fi
  typeset -a __hits __dscr __tmp
  if (( $@[(I)-d] )); then
    __tmp=${@[$[${@[(i)-d]}+1]]}
    if [[ $__tmp == \(* ]]; then
      eval "__dscr=$__tmp"
    else
      __dscr=( "${(@P)__tmp}" )
    fi
  fi
  builtin compadd -A __hits -D __dscr "$@"
  setopt localoptions norcexpandparam extendedglob
  typeset -A apre hpre hsuf asuf
  zparseopts -E P:=apre p:=hpre S:=asuf s:=hsuf
  [[ -n $__hits ]] || return
  local dscr
  for i in {1..$#__hits}; do
    (( $#__dscr >= $i )) && dscr=" -- ${${__dscr[$i]}##$__hits[$i] #}" || dscr=
    echo -E - $IPREFIX$apre$hpre$__hits[$i]$hsuf$asuf$dscr
  done
}

print -r -- READY
