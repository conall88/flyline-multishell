#!/bin/zsh
# Headless completion daemon for flyline, sourced after the user's rc. Rust drives
# it by writing "buffer<TAB>" to the pty and reads back the compadd-captured hits.

# No `emulate -L zsh`: at top level that would clobber the user's completion setup.
PROMPT=
RPROMPT=

# flyline's own synthesized completions (see ZSH_COMPLETIONS_DIR / the fly-reload
# widget below and resolve_completion_script_path in zsh.rs); keep this path in
# sync. Prepended so autoloadable `_<cmd>` functions written here are discovered.
fpath=( "${HOME}/.local/share/flyline/zsh/completions" $fpath )

# Idempotent compinit. The fast `-C` path skips the security check *and* the
# rescan of $fpath for new functions, reusing the cached dump. That means a
# completion installed after the dump was built (e.g. a freshly `brew install`ed
# `_kubectl`) stays invisible, which makes flyline needlessly offer to synthesize
# one. So rebuild the dump (`-i` skips the interactive insecure-dir prompt that
# would otherwise hang the headless daemon) whenever any $fpath dir is newer than
# the dump; otherwise keep the fast cached path.
autoload -Uz compinit
if (( $+functions[compinit] )); then
  _fly_dump="${ZDOTDIR:-$HOME}/.zcompdump_flyline"
  _fly_stale=""
  if [[ ! -e $_fly_dump ]]; then
    _fly_stale=1
  else
    for _fly_d in $fpath; do
      [[ -d $_fly_d && $_fly_d -nt $_fly_dump ]] && { _fly_stale=1; break; }
    done
  fi
  if [[ -n $_fly_stale ]]; then
    compinit -i -d "$_fly_dump"
  else
    compinit -C -d "$_fly_dump"
  fi
  unset _fly_dump _fly_stale _fly_d
fi

bindkey '^M' undefined
bindkey '^J' undefined
bindkey '^I' complete-word

# Broker cwd change: caller types a path then sends ^F. Only cds — command
# execution stays disabled (^M/^J are `undefined`) — so completions match the dir.
fly-cd () { builtin cd -q -- "$BUFFER" 2>/dev/null; BUFFER=''; print -r -- '<<FLYCD>>' }
zle -N fly-cd
bindkey '^F' fly-cd

# Register a flyline-synthesized completion: caller types the `_<cmd>` file path
# then sends ^G. We add its dir to $fpath (covers a custom flycomp_output dir),
# then autoload + `compdef` it so the very next capture uses it — no rebuild,
# mirroring bash's in-process eval of the synthesized script.
fly-reload () {
  local f=$BUFFER c d
  BUFFER=''
  d=${f:h}; c=${${f:t}#_}
  if [[ -n $c ]]; then
    [[ -n $d && -d $d ]] && fpath=( $d $fpath )
    unfunction "_$c" 2>/dev/null
    autoload -Uz -- "_$c" 2>/dev/null
    compdef "_$c" "$c" 2>/dev/null
  fi
  print -r -- '<<FLYRELOAD>>'
}
zle -N fly-reload
bindkey '^G' fly-reload

fly-begin () { compprefuncs=( fly-begin ); print -r -- '<<FLYBEGIN>>' }
# fly-end reports whether a completion function is registered for the command
# word ($_comps[cmd]) so Rust can tell "a completer ran but had nothing to add"
# (e.g. `kubectl get` with no cluster -> stay silent) apart from "no completer
# exists" (-> offer to synthesize one). Empty payload means no completer.
fly-end () {
  comppostfuncs=( fly-end )
  local -a __fly_words; __fly_words=( ${(z)BUFFER} )
  print -r -- "<<FLYCOMPDEF>>${_comps[${__fly_words[1]}]}"
  print -r -- '<<FLYEND>>'
}
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
