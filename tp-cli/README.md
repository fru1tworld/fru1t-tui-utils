# tp

디렉토리를 북마크하고 순간이동하는 CLI 도구

## Tech Stack

`TypeScript` `Node.js` `Vitest` `Biome`

## 설치

```bash
npm install -g @fru1tworld/tp
```

자식 프로세스는 부모 쉘의 디렉토리를 바꿀 수 없어서, 실제 `cd`는 쉘 함수가 맡는다. `tp-cli init`이 그 함수를 출력하니 설정 파일에 등록하고 터미널을 다시 열면 된다.

```bash
eval "$(tp-cli init bash)"   # ~/.bashrc
eval "$(tp-cli init zsh)"    # ~/.zshrc — compinit 뒤에 둘 것
```

```fish
tp-cli init fish | source    # ~/.config/fish/config.fish
```

```nu
# Nushell의 source는 상수 경로만 받으므로 파일로 저장해서 쓴다
tp-cli init nu | save -f ~/.tp/tp.nu
source ~/.tp/tp.nu           # ~/.config/nushell/config.nu
```

## 사용법

```bash
tp add <alias>      # 현재 디렉토리 북마크 추가 또는 갱신(UPSERT)
tp set <alias> <path> [<alias> <path> ...]
                    # 하나 이상의 북마크 경로를 지정하거나 갱신(UPSERT)
tp <alias>          # 북마크한 디렉토리로 이동
tp list             # 목록 — 별칭 UTF-8 바이트순
tp list -r          # 목록 — 등록 최신순
tp del <alias>      # 북마크 삭제
tp ch <old> <new>   # 별칭 변경
tp gc               # 사라진 경로 정리
tp help             # 도움말

tp-cli init <shell> # 쉘 함수 출력 (bash|zsh|fish|nu)
```

별칭 대소문자는 기본적으로 구분하지 않는다. 구분하려면 `~/.tp/config.json`에 `{"caseSensitive": true}`.

`tp add`는 UPSERT로 동작한다. 별칭이 없으면 새로 추가하고, 이미 있으면 해당 별칭의 경로를 현재 디렉토리로 갱신한다.

## 탭 자동완성

```bash
tp <TAB>        # 명령어 + 별칭
tp add <TAB>    # 갱신할 기존 별칭
tp del <TAB>    # 별칭
tp list <TAB>   # 정렬 플래그 (bash, zsh, fish)
```

## 예시

```bash
cd ~/projects/my-app
tp add app          # Added: app -> /Users/me/projects/my-app

cd ~/projects/renamed-app
tp add app          # Updated: 'app' /Users/me/projects/my-app -> /Users/me/projects/renamed-app

cd ~/projects
tp set api ./api web ./web  # 여러 북마크 경로를 한 번에 지정

cd /
tp app              # ~/projects/renamed-app으로 이동
```

## 데이터 저장 위치

`~/.tp/bookmarks.json`, `~/.tp/config.json`

## 요구사항

- Node.js >= 20
- macOS / Linux
- 지원 쉘: Bash, Zsh, Fish, Nushell

## 라이선스

MIT
