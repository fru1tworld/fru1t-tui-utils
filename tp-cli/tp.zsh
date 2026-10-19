tp() {
    local output
    output=$(tp-cli "$@") || return

    if [[ "$output" == __TP_CD__:* ]]; then
        builtin cd -- "${output#__TP_CD__:}"
    else
        print -r -- "$output"
    fi
}

# Candidates come from `tp-cli --complete`; ones ending in "/" take no suffix.
_tp_completions_zsh() {
    local -a candidates=(${(f)"$(tp-cli --complete "${(@)words[2,CURRENT]}" 2>/dev/null)"})
    compadd -S '' -- ${(M)candidates:#*/}
    compadd -- ${candidates:#*/}
}

(( $+functions[compdef] )) && compdef _tp_completions_zsh tp
