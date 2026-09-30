use std::io::Write;
use std::process::{Command, Stdio};

const COMMANDS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

pub(crate) fn copy(text: &str) -> Result<(), String> {
    let mut last_error = None;
    for (program, args) in COMMANDS {
        match write_to(program, args, text) {
            Ok(()) => return Ok(()),
            Err(reason) => last_error = Some(format!("{program}: {reason}")),
        }
    }
    Err(last_error
        .unwrap_or_else(|| "클립보드 명령을 찾을 수 없어요 (pbcopy/wl-copy/xclip/xsel)".into()))
}

fn write_to(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let write_result = child
        .stdin
        .take()
        .expect("stdin은 piped로 열어 두었다")
        .write_all(text.as_bytes());

    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    write_result.map_err(|e| e.to_string())?;

    if output.status.success() {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned);
    Err(reason.unwrap_or_else(|| format!("종료 코드 {}", output.status)))
}
