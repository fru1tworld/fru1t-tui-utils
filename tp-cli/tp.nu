# Candidates come from `tp-cli --complete`
def "nu-complete tp" [context: string] {
    let words = ($context | split row " " | skip 1)
    try { tp-cli --complete ...$words | lines } catch { [] }
}

# --env is required so cd persists in the caller's environment
def --env tp [...args: string@"nu-complete tp"] {
    let result = (do { tp-cli ...$args } | complete)

    if $result.exit_code != 0 {
        error make { msg: ($result.stderr | str trim) }
    }

    let output = ($result.stdout | str trim)
    if ($output | str starts-with "__TP_CD__:") {
        cd ($output | str replace "__TP_CD__:" "")
    } else {
        print $output
    }
}
