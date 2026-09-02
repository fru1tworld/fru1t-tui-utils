# til-tui

오늘 배운 내용을 주간 단위로 쌓는 Rust 터미널 메모장입니다. todo-tui와 같은
ratatui + SQLite 구성을 따르되, TIL 날짜와 기록이라는 작은 도메인에 맞춰 구성했습니다.

## 실행

    cargo run
    cargo build --release

최초 데이터베이스는 빈 상태로 만들어집니다. README의 기록은 화면 구조를 설명하기
위한 예시이며 실제 데이터에는 자동으로 추가되지 않습니다.

## TUI

실행하면 바로 Insert 모드입니다.

- 내용 + Enter: 화면에 표시된 날짜에 하나의 기록 추가
- Shift+Enter: 같은 기록 안에서 개행
- out + Enter: 표시 중인 날짜를 출력하고 종료
- 위쪽/아래쪽: 날짜와 기록 선택
- 왼쪽/오른쪽: 이전 주/다음 주 이동
- 날짜에서 Enter: 접기/펼치기
- Esc: Normal 모드

Normal 모드에서는 i로 입력, e로 편집, d로 삭제, u로 되돌리기,
y로 날짜 전체 복사, o로 출력, t로 오늘 이동이 가능합니다.
Shift+왼쪽/오른쪽은 선택한 기록을 전날이나 다음 날로 옮깁니다.
TUI는 SQLite 변경 여부를 1초마다 확인하므로 다른 터미널의 CLI에서 추가하거나
수정한 기록도 자동으로 반영됩니다. 추가·수정·삭제·날짜 이동은 최근 5개까지
u로 되돌릴 수 있습니다.

화면은 월요일부터 일요일까지 한 주를 보여 줍니다. 주차는 주의 목요일이 속한
달에 귀속하며, 그 달의 첫 목요일이 포함된 주를 1주차로 계산합니다. 따라서
2026-08-24~30은 8월 4주차, 2026-08-31~09-06은 9월 1주차입니다.

화면과 출력은 날짜가 1단계, 그날 배운 각 기록이 2단계인 구조입니다.
하나의 기록에 Shift+Enter로 입력한 여러 줄이 있으면 같은 2단계 항목의
이어지는 내용으로 유지됩니다.

    - 2026-08-31
      - 유니코드에는 서로게이트이라는 개념이 있다.
        UTF-16에서는 서로게이트 페어가 사용될 수 있다.
      - Lens Pattern
        - Get-Set
        - Set-Get
        - Set-Set

## CLI

add는 날짜를 반드시 받습니다. 날짜에는 today, yesterday, 오늘, 어제,
또는 YYYY-MM-DD를 사용할 수 있습니다.

    til-tui add --date today "UTF-16 서로게이트 페어를 공부했다"
    til-tui add --date 2026-08-31 "과거 날짜 기록"
    til-tui list --date today
    til-tui list --date yesterday --json
    til-tui out --date 2026-08-31
    til-tui edit 12 "수정한 내용"
    til-tui move 12 --date yesterday
    til-tui rm 12

JSON 기록의 필드는 id, content, ISO 8601 형식의 recorded_at입니다.
자동화나 테스트에서는 TIL_TUI_DB 환경 변수로 데이터베이스 경로를 지정할 수 있습니다.

## 데이터 위치

- macOS: ~/Library/Application Support/til-tui/til.db
- Linux: ~/.local/share/til-tui/til.db

## 코드 구성

- domain: TIL 기록과 내용 검증
- db: SQLite 저장, 조회, 마이그레이션
- cli: 명령행 입력 파싱과 출력
- app / action: TUI 상태와 사용자 동작
- ui: 화면 렌더링
- output: 중첩 마크다운 직렬화

저장소 구현이 하나뿐이므로 저장소 trait이나 서비스 객체는 두지 않았습니다.
TUI의 undo는 DB 전체를 복원하지 않고 각 작업의 역연산만 보관합니다.
