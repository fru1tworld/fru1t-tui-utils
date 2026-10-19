function tp
    set -l output (tp-cli $argv)
    or return

    if string match -q '__TP_CD__:*' -- $output
        builtin cd -- (string replace '__TP_CD__:' '' -- $output)
    else
        string join \n -- $output
    end
end

# Candidates come from `tp-cli --complete`
complete -c tp -f -a '(tp-cli --complete (commandline -opc)[2..] (commandline -ct) 2>/dev/null)'
