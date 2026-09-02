use chrono::{DateTime, Local};
use rusqlite::{Connection, Result, TransactionBehavior};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Todo {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
    pub done: bool,
    pub parent_id: Option<i64>,
    #[serde(skip)]
    pub collapsed: bool,
    pub position: i64,
    pub project_id: i64,
}

impl Todo {
    pub fn created_at_string(&self) -> String {
        format_epoch(self.created_at, "%Y-%m-%d %H:%M")
    }
}

const TODO_COLS: &str = "id, text, created_at, done, position, parent_id, collapsed, project_id";

fn todo_from_row(row: &rusqlite::Row) -> Result<Todo> {
    Ok(Todo {
        id: row.get(0)?,
        text: row.get(1)?,
        created_at: row.get(2)?,
        done: row.get(3)?,
        position: row.get(4)?,
        parent_id: row.get(5)?,
        collapsed: row.get(6)?,
        project_id: row.get(7)?,
    })
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn data_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))
    }

    pub fn open_default() -> Result<Self> {
        let path = default_db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self::open(path)
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self> {
        conn.set_transaction_behavior(TransactionBehavior::Immediate);
        let store = Self { conn };
        store.conn.busy_timeout(Duration::from_secs(5))?;
        store
            .conn
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        store.migrate()?;
        // 마이그레이션(테이블 재구축) 이후에만 외래 키 검사를 켠다.
        store.conn.pragma_update(None, "foreign_keys", true)?;
        Ok(store)
    }

    /// PRAGMA user_version 기반 버전 마이그레이션.
    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            let tx = self.conn.unchecked_transaction()?;
            migrate_v1(&tx)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = self.conn.unchecked_transaction()?;
            migrate_v2(&tx)?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = self.conn.unchecked_transaction()?;
            migrate_v3(&tx)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, position FROM projects ORDER BY position ASC, id ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_project(&self, name: &str) -> Result<i64> {
        let pos: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM projects",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO projects (name, position) VALUES (?1, ?2)",
            (name, pos),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 전달받은 순서대로 프로젝트 position을 1부터 다시 매긴다(탭 순서 변경용).
    pub fn set_project_positions(&self, order: &[i64]) -> Result<()> {
        self.renumber("UPDATE projects SET position = ?1 WHERE id = ?2", order)
    }

    /// 전달받은 순서대로 position을 1부터 다시 매기는 공통 루틴.
    fn renumber(&self, sql: &str, order: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(sql)?;
            for (i, id) in order.iter().enumerate() {
                ensure_row_changed(stmt.execute((i as i64 + 1, id))?)?;
            }
        }
        tx.commit()
    }

    pub fn rename_project(&self, id: i64, name: &str) -> Result<()> {
        let changed_rows = self
            .conn
            .execute("UPDATE projects SET name = ?1 WHERE id = ?2", (name, id))?;
        ensure_row_changed(changed_rows)
    }

    pub fn delete_project(&self, id: i64) -> Result<()> {
        // 소속 할 일은 project_id의 ON DELETE CASCADE가 지운다.
        let changed_rows = self
            .conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        ensure_row_changed(changed_rows)
    }

    /// 한 프로젝트의 할 일을 부모→자식→손자 순으로 중첩해 반환한다(최대 3단계).
    pub fn list(&self, project_id: i64) -> Result<Vec<Todo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TODO_COLS} FROM todos WHERE project_id = ?1
             ORDER BY done ASC, position ASC, id ASC"
        ))?;
        let rows = stmt.query_map([project_id], todo_from_row)?;
        let all: Vec<Todo> = rows.collect::<Result<_>>()?;

        let mut out = Vec::with_capacity(all.len());
        push_nested(&all, None, &mut out);
        Ok(out)
    }

    /// 전 프로젝트의 할 일을 id 순으로 반환한다(스냅숏·복원용, 부모가 항상 자식보다 앞).
    pub fn list_all(&self) -> Result<Vec<Todo>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {TODO_COLS} FROM todos ORDER BY id ASC"))?;
        let rows = stmt.query_map([], todo_from_row)?;
        rows.collect()
    }

    /// 외부 쓰기와 경합하지 않을 때만 두 테이블을 스냅숏 상태로 되돌린다.
    /// IMMEDIATE 트랜잭션을 먼저 확보하므로 버전 확인 이후 다른 writer가 끼어들 수 없다.
    pub fn replace_all_if_unchanged(
        &self,
        projects: &[Project],
        todos: &[Todo],
        expected_data_version: i64,
    ) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let current_data_version: i64 =
            tx.query_row("PRAGMA data_version", [], |row| row.get(0))?;
        if current_data_version != expected_data_version {
            return Ok(false);
        }
        // 순서 변경 뒤 들여쓰기하면 부모 id가 자식보다 클 수 있어 id 순 insert가
        // FK에 걸린다. 검사를 커밋 시점으로 미룬다(커밋 후 자동 원복).
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        tx.execute("DELETE FROM todos", [])?;
        tx.execute("DELETE FROM projects", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO projects (id, name, position) VALUES (?1, ?2, ?3)")?;
            for p in projects {
                stmt.execute((p.id, &p.name, p.position))?;
            }
            let mut stmt = tx.prepare(&format!(
                "INSERT INTO todos ({TODO_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ))?;
            for t in todos {
                stmt.execute((
                    t.id,
                    &t.text,
                    t.created_at,
                    t.done,
                    t.position,
                    t.parent_id,
                    t.collapsed,
                    t.project_id,
                ))?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn add(&self, text: &str, parent_id: Option<i64>, project_id: i64) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let id = insert_todo(&tx, text, parent_id, project_id)?;
        tx.commit()?;
        Ok(id)
    }

    /// 하위 목표 추가 + 부모 완료 해제·펼치기를 한 트랜잭션으로 처리한다.
    pub fn add_subtask(&self, text: &str, parent_id: i64) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let project_id: i64 = tx.query_row(
            "SELECT project_id FROM todos WHERE id = ?1",
            [parent_id],
            |r| r.get(0),
        )?;
        let id = insert_todo(&tx, text, Some(parent_id), project_id)?;
        ensure_row_changed(tx.execute(
            "UPDATE todos SET done = 0, collapsed = 0 WHERE id = ?1",
            [parent_id],
        )?)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn update_text(&self, id: i64, text: &str) -> Result<()> {
        let changed_rows = self
            .conn
            .execute("UPDATE todos SET text = ?1 WHERE id = ?2", (text, id))?;
        ensure_row_changed(changed_rows)
    }

    /// 여러 항목의 완료 상태를 한 트랜잭션으로 갱신한다(부모-자식 전파용).
    pub fn set_done_many(&self, updates: &[(i64, bool)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET done = ?1 WHERE id = ?2")?;
            for &(id, done) in updates {
                ensure_row_changed(stmt.execute((done, id))?)?;
            }
        }
        tx.commit()
    }

    /// 항목을 부모 밑으로 넣고 맨 뒤 position 부여, 부모 펼침(필요 시 완료 해제)까지 한 트랜잭션.
    pub fn indent(&self, id: i64, parent: i64, reopen_parent: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM todos",
            [],
            |r| r.get(0),
        )?;
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = ?1, position = ?2 WHERE id = ?3",
            (parent, pos, id),
        )?)?;
        ensure_row_changed(tx.execute("UPDATE todos SET collapsed = 0 WHERE id = ?1", [parent])?)?;
        if reopen_parent {
            ensure_row_changed(tx.execute("UPDATE todos SET done = 0 WHERE id = ?1", [parent])?)?;
        }
        tx.commit()
    }

    /// 전달받은 순서대로 position을 1부터 다시 매긴다(형제 그룹 재배열용).
    pub fn set_positions(&self, order: &[i64]) -> Result<()> {
        self.renumber("UPDATE todos SET position = ?1 WHERE id = ?2", order)
    }

    /// 항목과 하위 전체를 다른 프로젝트로 옮긴다. 옮긴 항목은 그쪽 최상위 맨 뒤가 된다.
    pub fn move_to_project(&self, root: i64, subtree: &[i64], project_id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM todos",
            [],
            |r| r.get(0),
        )?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET project_id = ?1 WHERE id = ?2")?;
            for id in subtree {
                ensure_row_changed(stmt.execute((project_id, id))?)?;
            }
        }
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = NULL, position = ?1 WHERE id = ?2",
            (pos, root),
        )?)?;
        tx.commit()
    }

    /// 항목을 한 단계 위(new_parent)로 빼고 그 단계의 형제 순서대로 position을 다시 매긴다.
    pub fn outdent(&self, id: i64, new_parent: Option<i64>, sibling_order: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = ?1 WHERE id = ?2",
            (new_parent, id),
        )?)?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET position = ?1 WHERE id = ?2")?;
            for (i, tid) in sibling_order.iter().enumerate() {
                ensure_row_changed(stmt.execute((i as i64 + 1, tid))?)?;
            }
        }
        tx.commit()
    }

    pub fn set_collapsed(&self, id: i64, collapsed: bool) -> Result<()> {
        let changed_rows = self.conn.execute(
            "UPDATE todos SET collapsed = ?1 WHERE id = ?2",
            (collapsed, id),
        )?;
        ensure_row_changed(changed_rows)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        // 자식 삭제는 parent_id의 ON DELETE CASCADE가 처리한다.
        let changed_rows = self.conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
        ensure_row_changed(changed_rows)
    }
}

/// 같은 부모를 가진 항목들을 순서대로 밀어 넣고, 각 항목 뒤에 그 하위를 재귀로 잇는다.
fn push_nested(all: &[Todo], parent: Option<i64>, out: &mut Vec<Todo>) {
    for t in all.iter().filter(|t| t.parent_id == parent) {
        out.push(t.clone());
        push_nested(all, Some(t.id), out);
    }
}

fn insert_todo(
    conn: &Connection,
    text: &str,
    parent_id: Option<i64>,
    project_id: i64,
) -> Result<i64> {
    let now = Local::now().timestamp();
    let pos: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM todos",
        [],
        |r| r.get(0),
    )?;
    let changed_rows = conn.execute(
        "INSERT INTO todos (text, created_at, done, position, parent_id, project_id)
         SELECT ?1, ?2, 0, ?3, ?4, ?5
         WHERE ?4 IS NULL
            OR EXISTS (
                SELECT 1 FROM todos parent
                WHERE parent.id = ?4 AND parent.project_id = ?5
            )",
        (text, now, pos, parent_id, project_id),
    )?;
    ensure_row_changed(changed_rows)?;
    Ok(conn.last_insert_rowid())
}

fn ensure_row_changed(changed_rows: usize) -> Result<()> {
    if changed_rows == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

/// v1: 최종 스키마로 재구축한다. 레거시 테이블(누락 컬럼 포함)을 흡수하고
/// parent_id에 ON DELETE CASCADE 외래 키를 건다.
fn migrate_v1(conn: &Connection) -> Result<()> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'todos')",
        [],
        |r| r.get(0),
    )?;

    if has_table {
        // 레거시 스키마에 없는 컬럼을 먼저 채워 아래 복사 SELECT 목록을 통일한다.
        let existing = column_names(conn)?;
        let added = [
            ("due_at", "ALTER TABLE todos ADD COLUMN due_at INTEGER"),
            (
                "position",
                "ALTER TABLE todos ADD COLUMN position INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "parent_id",
                "ALTER TABLE todos ADD COLUMN parent_id INTEGER",
            ),
            (
                "collapsed",
                "ALTER TABLE todos ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0",
            ),
        ];
        for (col, ddl) in added {
            if !existing.iter().any(|c| c == col) {
                conn.execute(ddl, [])?;
                if col == "position" {
                    conn.execute("UPDATE todos SET position = id", [])?;
                }
            }
        }
    }

    conn.execute_batch(
        "CREATE TABLE todos_v1 (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            text       TEXT    NOT NULL,
            created_at INTEGER NOT NULL,
            due_at     INTEGER,
            done       INTEGER NOT NULL DEFAULT 0,
            position   INTEGER NOT NULL DEFAULT 0,
            parent_id  INTEGER REFERENCES todos_v1(id) ON DELETE CASCADE,
            collapsed  INTEGER NOT NULL DEFAULT 0
        )",
    )?;
    if has_table {
        conn.execute_batch(
            "INSERT INTO todos_v1 (id, text, created_at, due_at, done, position, parent_id, collapsed)
             SELECT id, text, created_at, due_at, done, position, parent_id, collapsed FROM todos;
             DROP TABLE todos;",
        )?;
    }
    conn.execute("ALTER TABLE todos_v1 RENAME TO todos", [])?;
    Ok(())
}

/// v2: projects 테이블을 만들고 기존 할 일을 기본 프로젝트로 옮긴다.
/// todos는 project_id(ON DELETE CASCADE) 외래 키를 갖도록 재구축한다.
fn migrate_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE projects (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            name     TEXT    NOT NULL,
            position INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO projects (name, position) VALUES ('기본', 1);
        CREATE TABLE todos_v2 (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            text       TEXT    NOT NULL,
            created_at INTEGER NOT NULL,
            due_at     INTEGER,
            done       INTEGER NOT NULL DEFAULT 0,
            position   INTEGER NOT NULL DEFAULT 0,
            parent_id  INTEGER REFERENCES todos_v2(id) ON DELETE CASCADE,
            collapsed  INTEGER NOT NULL DEFAULT 0,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE
        );
        INSERT INTO todos_v2 (id, text, created_at, due_at, done, position, parent_id, collapsed, project_id)
            SELECT id, text, created_at, due_at, done, position, parent_id, collapsed,
                   (SELECT id FROM projects ORDER BY id LIMIT 1)
            FROM todos;
        DROP TABLE todos;
        ALTER TABLE todos_v2 RENAME TO todos;",
    )
}

/// v3: 더 이상 사용하지 않는 due_at 컬럼과 저장된 마감 값을 제거한다.
fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE todos_v3 (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            text       TEXT    NOT NULL,
            created_at INTEGER NOT NULL,
            done       INTEGER NOT NULL DEFAULT 0,
            position   INTEGER NOT NULL DEFAULT 0,
            parent_id  INTEGER REFERENCES todos_v3(id) ON DELETE CASCADE,
            collapsed  INTEGER NOT NULL DEFAULT 0,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE
        );
        INSERT INTO todos_v3 (id, text, created_at, done, position, parent_id, collapsed, project_id)
            SELECT id, text, created_at, done, position, parent_id, collapsed, project_id
            FROM todos;
        DROP TABLE todos;
        ALTER TABLE todos_v3 RENAME TO todos;",
    )
}

fn column_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA table_info(todos)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

fn format_epoch(epoch: i64, fmt: &str) -> String {
    DateTime::from_timestamp(epoch, 0).map_or_else(
        || "?".to_string(),
        |dt| dt.with_timezone(&Local).format(fmt).to_string(),
    )
}

fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("todo-tui")
        .join("todos.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn default_project(s: &Store) -> i64 {
        s.list_projects().unwrap()[0].id
    }

    fn position_of(s: &Store, id: i64) -> i64 {
        s.conn
            .query_row("SELECT position FROM todos WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn texts(s: &Store, project_id: i64) -> Vec<String> {
        s.list(project_id)
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect()
    }

    #[test]
    fn add_list_update_toggle_delete_roundtrip() {
        let s = mem_store();
        let pid = default_project(&s);
        assert!(s.list(pid).unwrap().is_empty());

        let id = s.add("첫 번째 할 일", None, pid).unwrap();
        let todos = s.list(pid).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "첫 번째 할 일");
        assert_eq!(todos[0].project_id, pid);
        assert!(!todos[0].done);
        assert!(todos[0].created_at > 0);

        s.update_text(id, "수정됨").unwrap();
        let t = &s.list(pid).unwrap()[0];
        assert_eq!(t.text, "수정됨");

        s.set_done_many(&[(id, true)]).unwrap();
        assert!(s.list(pid).unwrap()[0].done);

        s.delete(id).unwrap();
        assert!(s.list(pid).unwrap().is_empty());
    }

    #[test]
    fn add_assigns_increasing_position() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        let b = s.add("b", None, pid).unwrap();
        let c = s.add("c", None, pid).unwrap();
        assert!(position_of(&s, a) < position_of(&s, b));
        assert!(position_of(&s, b) < position_of(&s, c));
        assert_eq!(texts(&s, pid), ["a", "b", "c"]);
    }

    #[test]
    fn done_items_sink_to_bottom() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        s.add("b", None, pid).unwrap();
        s.add("c", None, pid).unwrap();

        s.set_done_many(&[(a, true)]).unwrap();
        assert_eq!(texts(&s, pid), ["b", "c", "a"]);

        s.set_done_many(&[(a, false)]).unwrap();
        assert_eq!(texts(&s, pid), ["a", "b", "c"]);
    }

    #[test]
    fn done_children_sink_within_parent() {
        let s = mem_store();
        let pid = default_project(&s);
        let p = s.add("p", None, pid).unwrap();
        let c1 = s.add("c1", Some(p), pid).unwrap();
        s.add("c2", Some(p), pid).unwrap();
        s.add("q", None, pid).unwrap();

        s.set_done_many(&[(c1, true)]).unwrap();
        assert_eq!(texts(&s, pid), ["p", "c2", "c1", "q"]);
    }

    #[test]
    fn set_positions_reorders() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        let b = s.add("b", None, pid).unwrap();
        let c = s.add("c", None, pid).unwrap();
        s.set_positions(&[c, a, b]).unwrap();
        assert_eq!(texts(&s, pid), ["c", "a", "b"]);
    }

    #[test]
    fn migrates_legacy_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE todos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                done INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO todos (text, created_at) VALUES ('old', 100)",
            [],
        )
        .unwrap();
        let store = Store::from_connection(conn).unwrap();

        let pid = default_project(&store);
        let t = &store.list(pid).unwrap()[0];
        assert_eq!(t.text, "old");
        assert_eq!(t.project_id, pid);
        assert_eq!(position_of(&store, t.id), t.id);
    }

    #[test]
    fn migrates_v0_full_schema_and_cascades() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE todos (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                text       TEXT    NOT NULL,
                created_at INTEGER NOT NULL,
                due_at     INTEGER,
                done       INTEGER NOT NULL DEFAULT 0,
                position   INTEGER NOT NULL DEFAULT 0,
                parent_id  INTEGER,
                collapsed  INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO todos (id, text, created_at, position) VALUES (1, 'p', 100, 1);
            INSERT INTO todos (id, text, created_at, position, parent_id)
                VALUES (2, 'c', 100, 2, 1);",
        )
        .unwrap();
        let store = Store::from_connection(conn).unwrap();

        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
        assert!(
            !column_names(&store.conn)
                .unwrap()
                .contains(&"due_at".into())
        );

        let pid = default_project(&store);
        assert_eq!(texts(&store, pid), ["p", "c"]);

        // 재구축된 테이블의 FK CASCADE로 자식까지 지워진다.
        store.delete(1).unwrap();
        assert!(store.list(pid).unwrap().is_empty());
    }

    #[test]
    fn v3_migration_removes_saved_due_dates_without_losing_todos() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 2;
            CREATE TABLE projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO projects (id, name, position) VALUES (1, '기본', 1);
            CREATE TABLE todos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                due_at INTEGER,
                done INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL DEFAULT 0,
                parent_id INTEGER REFERENCES todos(id) ON DELETE CASCADE,
                collapsed INTEGER NOT NULL DEFAULT 0,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE
            );
            INSERT INTO todos
                (id, text, created_at, due_at, done, position, collapsed, project_id)
                VALUES (1, '마감이 있던 항목', 100, 200, 0, 1, 0, 1);",
        )
        .unwrap();

        let store = Store::from_connection(conn).unwrap();

        assert_eq!(texts(&store, 1), ["마감이 있던 항목"]);
        assert!(
            !column_names(&store.conn)
                .unwrap()
                .contains(&"due_at".into())
        );
        assert_eq!(
            store
                .conn
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn list_nests_children_after_parent() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        s.add("b", None, pid).unwrap();
        s.add("a2", Some(a), pid).unwrap();
        s.add("a1", Some(a), pid).unwrap();
        assert_eq!(texts(&s, pid), ["a", "a2", "a1", "b"]);
    }

    #[test]
    fn list_nests_three_levels() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        s.add("b", None, pid).unwrap();
        let a1 = s.add("a1", Some(a), pid).unwrap();
        s.add("a1-1", Some(a1), pid).unwrap();
        assert_eq!(texts(&s, pid), ["a", "a1", "a1-1", "b"]);
    }

    #[test]
    fn delete_parent_removes_children() {
        let s = mem_store();
        let pid = default_project(&s);
        let p = s.add("p", None, pid).unwrap();
        s.add("c1", Some(p), pid).unwrap();
        s.add("c2", Some(p), pid).unwrap();
        assert_eq!(s.list(pid).unwrap().len(), 3);
        s.delete(p).unwrap();
        assert!(s.list(pid).unwrap().is_empty());
    }

    #[test]
    fn projects_are_isolated() {
        let s = mem_store();
        let p1 = default_project(&s);
        let p2 = s.add_project("업무").unwrap();
        s.add("개인 일", None, p1).unwrap();
        s.add("회사 일", None, p2).unwrap();
        assert_eq!(texts(&s, p1), ["개인 일"]);
        assert_eq!(texts(&s, p2), ["회사 일"]);
    }

    #[test]
    fn add_rejects_a_parent_from_another_project() {
        let store = mem_store();
        let personal_project_id = default_project(&store);
        let work_project_id = store.add_project("업무").unwrap();
        let parent_id = store.add("개인 상위", None, personal_project_id).unwrap();

        assert!(
            store
                .add("잘못된 하위", Some(parent_id), work_project_id)
                .is_err()
        );
        assert_eq!(texts(&store, personal_project_id), ["개인 상위"]);
        assert!(store.list(work_project_id).unwrap().is_empty());
    }

    #[test]
    fn mutations_report_missing_rows() {
        let store = mem_store();

        assert!(store.update_text(999, "없음").is_err());
        assert!(store.delete(999).is_err());
        assert!(store.rename_project(999, "없음").is_err());
    }

    #[test]
    fn delete_project_cascades_todos() {
        let s = mem_store();
        let p1 = default_project(&s);
        let p2 = s.add_project("업무").unwrap();
        let t = s.add("회사 일", None, p2).unwrap();
        s.add("하위", Some(t), p2).unwrap();
        s.delete_project(p2).unwrap();
        assert!(s.list_all().unwrap().is_empty());
        assert_eq!(s.list_projects().unwrap().len(), 1);
        assert_eq!(s.list_projects().unwrap()[0].id, p1);
    }

    #[test]
    fn replace_all_restores_snapshot() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        s.add("a1", Some(a), pid).unwrap();

        let projects = s.list_projects().unwrap();
        let todos = s.list_all().unwrap();

        s.delete(a).unwrap();
        s.add_project("임시").unwrap();
        assert!(s.list(pid).unwrap().is_empty());

        let data_version = s.data_version().unwrap();
        assert!(
            s.replace_all_if_unchanged(&projects, &todos, data_version)
                .unwrap()
        );
        assert_eq!(texts(&s, pid), ["a", "a1"]);
        assert_eq!(s.list_projects().unwrap(), projects);
    }

    #[test]
    fn replace_all_handles_parent_with_larger_id() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add("a", None, pid).unwrap();
        let b = s.add("b", None, pid).unwrap();
        // b를 위로 올린 뒤 a를 그 밑에 넣으면 부모(b) id가 자식(a)보다 커진다.
        s.set_positions(&[b, a]).unwrap();
        s.indent(a, b, true).unwrap();

        let projects = s.list_projects().unwrap();
        let todos = s.list_all().unwrap();
        let data_version = s.data_version().unwrap();
        assert!(
            s.replace_all_if_unchanged(&projects, &todos, data_version)
                .unwrap()
        );
        assert_eq!(texts(&s, pid), ["b", "a"]);
    }
}
