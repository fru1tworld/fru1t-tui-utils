use std::io::Write;
use std::process::{Command, Stdio};

/// 시스템 클립보드에 쓰는 외부 명령 후보. 앞에서부터 성공할 때까지 시도한다.
const COMMANDS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// 텍스트를 시스템 클립보드에 복사한다.
/// 후보 명령이 모두 실패하면 마지막 실패 사유를 담은 Err를 돌려준다.
/// (명령이 설치돼 있어도 디스플레이가 없어 실패할 수 있으므로 실패하면 다음 후보로 넘어간다.)
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

    // 쓰기가 실패해도 자식은 반드시 거둬들여 좀비가 남지 않게 한다.
    // wait_with_output은 stderr를 함께 읽어 파이프가 차서 막히는 일이 없다.
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    write_result.map_err(|e| e.to_string())?;

    if output.status.success() {
        return Ok(());
    }
    // 실패 사유(예: "failed to connect to a Wayland display")를 상태 표시줄에 그대로 보여준다.
    let reason = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned);
    Err(reason.unwrap_or_else(|| format!("종료 코드 {}", output.status)))
}
