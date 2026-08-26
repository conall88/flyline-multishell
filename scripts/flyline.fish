# >>> flyline start >>>
# On an empty native prompt, Enter launches flyline as a separate process. It
# draws its TUI on the tty and returns the chosen line on fd 3. Running from a
# fish key binding keeps accepted commands in the reader's normal execution
# path, including interactive job control and terminal input.

if status is-interactive; and not set -q _flyline_loaded
    set -g _flyline_loaded 1
    set -g _flyline_script (status filename)

    # Default to flyline-standalone next to the install dir (parent of scripts/).
    if not set -q FLYLINE_BIN
        set -g FLYLINE_BIN (path resolve (status dirname)/../flyline-standalone)
    end

    # Forward `flyline <subcommand>` typed at the prompt to the standalone binary
    # so the full CLI surface (run-tutorial, set-style, changelog, ...) matches
    # the Bash builtin / zsh widget. Fail-open if the binary is missing.
    function flyline
        test -x "$FLYLINE_BIN"; or return 127
        "$FLYLINE_BIN" $argv
    end

    # fish 4.x readline sends blocking terminal queries (cursor position,
    # background color, DA1) each prompt. Their replies land while flyline
    # owns the tty and get consumed by its TUI, leaving the query pending
    # forever — fish then dies on `assertion failed: query.is_none()`
    # (reader.rs) at the next prompt, window resize, or theme change.
    # This variable is checked before every query, so setting it here and
    # clearing it in flyline_disable is safely session-scoped. While flyline
    # renders the command line, fish loses nothing it uses.
    set -g FISH_TEST_NO_RECURRENT_QUERIES 1

    function _flyline_edit
        set -l last_exit $argv[1]

        # flyline reads history from the fish history file; flush this session's
        # first, and tell flyline which session file to read.
        builtin history save 2>/dev/null
        set -l hist_session $fish_history
        test -n "$hist_session"; or set hist_session fish
        set -lx FLYLINE_FISH_HISTORY $__fish_user_data_dir/{$hist_session}_history

        # fish prompts are functions: hand flyline the rendered output (ANSI),
        # covering starship/tide/etc. for free.
        set -lx PS1 (fish_prompt 2>/dev/null | string collect)
        test -n "$PS1"; or set PS1 (printf '%s@%s %s> ' $USER (prompt_hostname) (prompt_pwd))
        set -lx RPS1 ''
        functions -q fish_right_prompt
        and set RPS1 (fish_right_prompt 2>/dev/null | string collect)

        set -lx FLYLINE_HOST fish
        set -lx FLYLINE_INIT (commandline | string collect)
        set -lx FLYLINE_LAST_EXIT $last_exit

        # UI -> /dev/tty, chosen line -> fd 3 -> captured here; keys from the tty.
        set -l cmd ("$FLYLINE_BIN" </dev/tty 3>&1 1>/dev/tty 2>/dev/tty | string collect)
        set -l rc $pipestatus[1]

        if test $rc -eq 0
            commandline -r -- $cmd
            commandline -f execute
        else if test $rc -eq 130
            # Ctrl-C: clear to a fresh native line for this prompt.
            commandline -r ''
            commandline -f repaint
        else
            commandline -f repaint
        end
        return 0
    end

    function _flyline_enter
        set -l last_exit $status
        if test -n (commandline | string collect); or not test -x "$FLYLINE_BIN"
            commandline -f execute
            return 0
        end
        _flyline_edit $last_exit
    end

    # Preserve user Enter bindings so flyline_disable is reversible. Fish's
    # preset binding remains underneath when there is no user override.
    set -g _flyline_prev_default_enter (bind --user -M default \r 2>/dev/null | string collect)
    set -g _flyline_prev_insert_enter (bind --user -M insert \r 2>/dev/null | string collect)
    bind --user -M default \r _flyline_enter
    bind --user -M insert \r _flyline_enter

    function flyline_enable
        set -l script $_flyline_script
        flyline_disable
        source $script
    end

    function flyline_disable
        functions -e _flyline_edit
        functions -e _flyline_enter
        functions -e flyline
        bind --erase --user -M default \r 2>/dev/null
        bind --erase --user -M insert \r 2>/dev/null
        test -n "$_flyline_prev_default_enter"; and eval $_flyline_prev_default_enter
        test -n "$_flyline_prev_insert_enter"; and eval $_flyline_prev_insert_enter
        set -e _flyline_loaded
        set -e _flyline_prev_default_enter
        set -e _flyline_prev_insert_enter
        set -e FISH_TEST_NO_RECURRENT_QUERIES
    end

    function flyline_uninstall
        flyline_disable
        set -e FLYLINE_BIN
    end
end
# <<< flyline end <<<
