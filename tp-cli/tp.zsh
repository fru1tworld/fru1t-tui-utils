# tp - Zsh shell wrapper for tp directory bookmarks
# Add to your ~/.zshrc, after compinit:
#   eval "$(tp-cli init zsh)"

tp() {
    local output
    output=$(tp-cli "$@")
    local command_status=$?

    if (( command_status != 0 )); then
        print -r -- "$output" >&2
        return $command_status
    fi

    if [[ "$output" == __TP_CD__:* ]]; then
        local target="${output#__TP_CD__:}"
        builtin cd -- "$target"
    else
        print -r -- "$output"
    fi
}

_tp_completions_zsh() {
    local commands="add set del ch gc list help"
    local aliases=$(tp-cli --completions 2>/dev/null)

    case "$words[2]" in
        add|del|ch)
            _values 'alias' ${(f)aliases}
            ;;
        set)
            if (( CURRENT % 2 == 1 )); then
                _values 'alias' ${(f)aliases}
            else
                _directories
            fi
            ;;
        list)
            _values 'order' -u --utf8 -r --recent
            ;;
        gc|help)
            ;;
        *)
            _values 'command' $commands ${(f)aliases}
            ;;
    esac
}

# compdef does not exist until compinit has run
(( $+functions[compdef] )) && compdef _tp_completions_zsh tp
