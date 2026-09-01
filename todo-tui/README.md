# todo-tui

생성 시각(timestamp)을 함께 기록하고, 할 일에 **하위 목표(3뎁스)** 를 붙일 수 있는 터미널 To-Do 앱.
Rust + [ratatui](https://ratatui.rs) + SQLite([rusqlite], bundled) 로 구현했습니다.

## 실행

```sh
cargo run            # 디버그 실행
cargo build --release && ./target/release/todo-tui   # 릴리스 빌드 후 실행
```

## 화면 구성

```
┌ To-Do  1 기본 (2개)  2 회사   -- INSERT -- ─────────┐
├ 목록  [ ] 생성시각: 내용 ────────────────────────────┤
│ ▾ [ ] 2026-06-29 11:17: 이사 준비   (1/2)            │
│      ├ [x] 2026-06-29 11:18: 이삿짐센터 예약          │
│      └ [ ] 2026-06-29 11:19: 전입신고   ⏳ 2026-07-01 │
│   [x] 2026-06-28 09:30: 운동                         │
├ 새 할 일 (Enter 추가 · Esc 명령모드) ────────────────┤
└─────────────────────────────────────────────────────┘
```

개수(`N개`)는 **선택된 프로젝트 탭 안에** 붙고 **최상위 항목만** 셉니다. 부모 항목에는 접힘 표시(`▾`/`▸`)와 완료 진행도(`완료/전체`)가 붙고, 하위 목표는 한 단 들여쓴 뒤 `├`/`└`로 표시합니다.

## 조작 (vim 스타일 모달)

실행하면 바로 **Insert 모드**라 타이핑만으로 할 일을 추가합니다.
`Esc`를 누르면 명령용 **Normal 모드**로 넘어가고, 다시 `i`나 `Esc`로 돌아옵니다.

**Insert 모드** — 빠른 추가

| 키 | 동작 |
|----|------|
| 타이핑 + `Enter` | 입력한 내용으로 새 할 일 추가 |
| `↑` / `↓` | 선택 이동 |
| `←` / `→` | 접기 / 펼치기 |
| `Shift`+`←` / `→` | 하위로 넣기 / 최상위로 빼기 |
| `Shift`+`↑` / `↓` | 순서 이동 |
| `Esc` | Normal 모드로 전환 |

**Normal 모드** — 명령

| 키 | 동작 |
|----|------|
| `i` / `a` / `Esc` | Insert 모드로 전환 |
| `s` | 선택 항목에 하위 목표 추가 |
| `e` | 선택 항목 내용 편집 |
| `t` | 선택 항목 마감일 설정/해제 (`YYYY-MM-DD`, 비우면 해제) |
| `Space` | 완료/미완료 토글 (부모↔자식 연동) |
| `d` | 삭제 (부모 삭제 시 하위 목표도 함께) |
| `y` | 선택 항목을 하위 목표까지 마크다운 체크리스트로 클립보드에 복사 |
| `↑`/`k`, `↓`/`j` | 선택 이동 |
| `←`/`h`, `→`/`l` | 접기 / 펼치기 |
| `Shift`+`←` / `→` | **하위로 넣기** / **최상위로 빼기** |
| `Shift`+`↑`/`↓` | **순서 이동** (위/아래로 재정렬) |
| `q` | 종료 |

> - `y` 복사는 시스템 클립보드 명령(`pbcopy`/`wl-copy`/`xclip`/`xsel`) 중 있는 것을 씁니다. 하나도 없으면 상태 표시줄에 안내가 뜹니다.
> - 항목은 직접 정한 순서로 정렬·저장되며, 새 할 일은 맨 아래에 추가됩니다.
> - 하위 목표는 **3뎁스까지**만 가능합니다. `Shift`+`←`(넣기)는 선택 항목을 바로 위 항목의 하위로 넣고, 순서 이동 시 부모를 옮기면 하위 목표가 함께 따라갑니다.
> - 부모의 완료 상태는 **자식이 모두 완료되면 자동 완료**되고, 부모를 토글하면 자식 전체가 따라옵니다.

항목은 `[ ] 생성시각: 내용` 꼴로 보입니다. 생성 시각은 추가한 시점을 로컬 타임존 기준 `YYYY-MM-DD HH:MM`으로 적고, 마감일이 있으면 `⏳`를 덧붙입니다(지난 마감은 빨간색).

## 데이터 저장

모든 데이터는 SQLite 파일에 저장돼 앱을 껐다 켜도 유지됩니다.

- macOS: `~/Library/Application Support/todo-tui/todos.db`
- Linux: `~/.local/share/todo-tui/todos.db`

스키마:

```sql
CREATE TABLE todos (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    text       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,  -- UTC epoch seconds
    due_at     INTEGER,           -- 마감 (epoch seconds, 없으면 NULL)
    done       INTEGER NOT NULL DEFAULT 0,
    position   INTEGER NOT NULL DEFAULT 0,  -- 정렬 순서
    parent_id  INTEGER,           -- 부모 항목 id (최상위면 NULL)
    collapsed  INTEGER NOT NULL DEFAULT 0   -- 하위 목표 접힘 여부
);
```

[rusqlite]: https://github.com/rusqlite/rusqlite
