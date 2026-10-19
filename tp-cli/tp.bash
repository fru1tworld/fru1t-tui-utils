tp() {
    local output
    output=$(tp-cli "$@") || return

    if [[ "$output" == __TP_CD__:* ]]; then
        builtin cd -- "${output#__TP_CD__:}"
    else
        printf '%s\n' "$output"
    fi
}

# Candidates come from `tp-cli --complete`. Bash 3.2 has no compopt, so the
# completion is registered with nospace and finished words get their space here.
_tp_completions() {
    local IFS=$'\n' candidate
    COMPREPLY=()
    for candidate in $(tp-cli --complete "${COMP_WORDS[@]:1:COMP_CWORD}" 2>/dev/null); do
        [[ "$candidate" == */ ]] || candidate+=" "
        COMPREPLY+=("$candidate")
    done
}

complete -o nospace -F _tp_completions tp
