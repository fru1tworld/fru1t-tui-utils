# tp - Bash shell wrapper for tp directory bookmarks
# Add to your ~/.bashrc:
#   eval "$(tp-cli init bash)"

tp() {
    local output
    output=$(tp-cli "$@")
    local command_status=$?

    if (( command_status != 0 )); then
        printf '%s\n' "$output" >&2
        return "$command_status"
    fi

    if [[ "$output" == __TP_CD__:* ]]; then
        local target="${output#__TP_CD__:}"
        builtin cd -- "$target"
    else
        printf '%s\n' "$output"
    fi
}

_tp_completions() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    local commands="add set del ch gc list help"

    if [[ "${COMP_WORDS[1]}" == "set" ]]; then
        if (( COMP_CWORD % 2 == 0 )); then
            local aliases=$(tp-cli --completions 2>/dev/null)
            COMPREPLY=($(compgen -W "$aliases" -- "$cur"))
        else
            COMPREPLY=($(compgen -d -- "$cur"))
        fi
        return
    fi

    case "$prev" in
        tp)
            local aliases=$(tp-cli --completions 2>/dev/null)
            COMPREPLY=($(compgen -W "$commands $aliases" -- "$cur"))
            ;;
        add|del|ch)
            local aliases=$(tp-cli --completions 2>/dev/null)
            COMPREPLY=($(compgen -W "$aliases" -- "$cur"))
            ;;
        list)
            COMPREPLY=($(compgen -W "-u --utf8 -r --recent" -- "$cur"))
            ;;
        *)
            COMPREPLY=()
            ;;
    esac
}

complete -F _tp_completions tp
