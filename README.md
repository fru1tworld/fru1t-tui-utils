# fru1t-tui-utils

후르츠의 CLI와 TUI 도구들.

터미널에서 매일 쓰는 작은 도구를 한 저장소에 모아 두었습니다.
Rust 앱은 루트 Cargo workspace로 묶여 있고, `tp-cli`는 pnpm으로 관리합니다.

## 도구

| 디렉터리 | 종류 | 설명 |
| --- | --- | --- |
| [`todo-tui`](./todo-tui) | TUI (Rust) | 프로젝트 탭과 3뎁스 하위 목표를 지원하는 vim 스타일 할 일 앱 |
| [`til-tui`](./til-tui) | TUI (Rust) | 오늘 배운 내용을 주 단위로 쌓는 터미널 메모장 |
| [`tp-cli`](./tp-cli) | CLI (TypeScript) | 디렉터리를 북마크하고 한 번에 이동하는 `tp` 명령 |

`todo-tui`와 `til-tui`는 ratatui + SQLite 구성을 공유합니다.
`tp-cli`는 npm에 `@fru1tworld/tp`로 배포되며, bash·zsh·fish·nushell 셸 함수를 제공합니다.

## 빌드와 검사

mise가 설치된 환경에서 포맷·린트·테스트·빌드를 한 번에 실행합니다.

    mise run check          # 전체
    mise run check:rust     # Rust workspace만
    mise run check:tp       # tp-cli만

mise 없이 실행하려면 Rust는 `cargo build --release`, tp-cli는 `pnpm install && pnpm build`를 사용합니다.
GitHub Actions에서도 같은 검사를 실행합니다.

자세한 사용법과 키 조작은 각 디렉터리의 README를 참고하세요.
