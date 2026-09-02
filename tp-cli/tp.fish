# tp - Fish shell wrapper for tp directory bookmarks
# Add to your ~/.config/fish/config.fish:
#   tp-cli init fish | source

function tp
    set -l output (tp-cli $argv)
    set -l command_status $status

    if test $command_status -ne 0
        string join \n -- $output >&2
        return $command_status
    end

    if string match -q '__TP_CD__:*' -- $output
        set -l target (string replace '__TP_CD__:' '' -- $output)
        builtin cd -- $target
    else
        string join \n -- $output
    end
end

function __tp_set_wants_alias
    __fish_seen_subcommand_from set; or return 1
    set -l tokens (commandline -opc)
    test (math (count $tokens) % 2) -eq 0
end

function __tp_set_wants_path
    __fish_seen_subcommand_from set; or return 1
    set -l tokens (commandline -opc)
    test (math (count $tokens) % 2) -eq 1
end

complete -c tp -f

complete -c tp -n '__fish_use_subcommand' -a 'add' -d 'Add or update current directory bookmark (upsert)'
complete -c tp -n '__fish_use_subcommand' -a 'set' -d 'Set one or more bookmark paths (upsert)'
complete -c tp -n '__fish_use_subcommand' -a 'del' -d 'Delete bookmark'
complete -c tp -n '__fish_use_subcommand' -a 'ch' -d 'Rename alias'
complete -c tp -n '__fish_use_subcommand' -a 'gc' -d 'Clean invalid bookmarks'
complete -c tp -n '__fish_use_subcommand' -a 'list' -d 'Show all bookmarks'
complete -c tp -n '__fish_use_subcommand' -a 'help' -d 'Show help'

complete -c tp -n '__fish_use_subcommand' -a '(tp-cli --completions 2>/dev/null)'

complete -c tp -n '__fish_seen_subcommand_from add del ch' -a '(tp-cli --completions 2>/dev/null)'
complete -c tp -n '__tp_set_wants_alias' -a '(tp-cli --completions 2>/dev/null)'
complete -c tp -n '__tp_set_wants_path' -a '(__fish_complete_directories (commandline -ct))'

complete -c tp -n '__fish_seen_subcommand_from list' -s u -l utf8 -d 'Sort by alias (UTF-8 order)'
complete -c tp -n '__fish_seen_subcommand_from list' -s r -l recent -d 'Sort by newest first'
