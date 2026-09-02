# utils

개인적으로 사용하는 CLI와 TUI 도구를 모아 둔 저장소입니다.

| 디렉터리 | 설명 |
| --- | --- |
| `todo-tui` | 프로젝트와 하위 목표를 관리하는 Rust 터미널 할 일 앱 |
| `til-tui` | 날짜별로 배운 내용을 기록하는 Rust 터미널 메모장 |
| `tp-cli` | 자주 사용하는 디렉터리를 등록하고 이동하는 TypeScript CLI |

Rust 앱은 루트 Cargo workspace로 묶여 있고, `tp-cli`는 pnpm을 사용합니다.
전체 포맷·린트·테스트·빌드는 mise가 설치된 환경에서 한 번에 실행할 수 있습니다.

    mise run check

Rust만 확인하려면 `mise run check:rust`, tp-cli만 확인하려면
`mise run check:tp`를 사용합니다. GitHub Actions에서도 같은 검사를 실행합니다.
자세한 사용법은 각 디렉터리의 README를 참고하세요.
