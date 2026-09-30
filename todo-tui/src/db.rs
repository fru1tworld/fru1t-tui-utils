use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Local};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, Result, TransactionBehavior};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TodoId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProjectId(pub i64);

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl ToSql for TodoId {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl ToSql for ProjectId {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for TodoId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self)
    }
}

impl FromSql for ProjectId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Todo {
    pub id: TodoId,
    pub text: String,
    pub created_at: i64,
    pub done: bool,
    pub parent_id: Option<TodoId>,
    #[serde(skip)]
    pub collapsed: bool,
    pub position: i64,
    pub project_id: ProjectId,
}

impl Todo {
    pub fn created_at_string(&self) -> String {
        DateTime::from_timestamp(self.created_at, 0).map_or_else(
            || "?".to_string(),
            |dt| {
                dt.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentCompletion {
    Reopen,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    ChangedExternally,
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

const MIGRATIONS: [fn(&Connection) -> Result<()>; 3] = [migrate_v1, migrate_v2, migrate_v3];

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open_default() -> anyhow::Result<Self> {
        let path = default_db_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| {
                format!("데이터 디렉터리를 만들 수 없습니다: {}", dir.display())
            })?;
        }
        Self::open(&path).with_context(|| format!("DB를 열 수 없습니다: {}", path.display()))
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self> {
        conn.set_transaction_behavior(TransactionBehavior::Immediate);
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        let store = Self { conn };
        store.migrate()?;
        store.conn.pragma_update(None, "foreign_keys", true)?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let current: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (version, migration) in (1_i64..).zip(MIGRATIONS) {
            if current < version {
                let tx = self.conn.unchecked_transaction()?;
                migration(&tx)?;
                tx.pragma_update(None, "user_version", version)?;
                tx.commit()?;
            }
        }
        Ok(())
    }

    pub fn data_version(&self) -> Result<i64> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))
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

    pub fn add_project(&self, name: &str) -> Result<ProjectId> {
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM projects",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO projects (name, position) VALUES (?1, ?2)",
            (name, position),
        )?;
        Ok(ProjectId(self.conn.last_insert_rowid()))
    }

    pub fn set_project_positions(&self, order: &[ProjectId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        renumber(
            &tx,
            "UPDATE projects SET position = ?1 WHERE id = ?2",
            order,
        )?;
        tx.commit()
    }

    pub fn rename_project(&self, id: ProjectId, name: &str) -> Result<()> {
        let changed_rows = self
            .conn
            .execute("UPDATE projects SET name = ?1 WHERE id = ?2", (name, id))?;
        ensure_row_changed(changed_rows)
    }

    pub fn delete_project(&self, id: ProjectId) -> Result<()> {
        let changed_rows = self
            .conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        ensure_row_changed(changed_rows)
    }

    pub fn list_todos(&self, project_id: ProjectId) -> Result<Vec<Todo>> {
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

    pub fn list_all_todos(&self) -> Result<Vec<Todo>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {TODO_COLS} FROM todos ORDER BY id ASC"))?;
        let rows = stmt.query_map([], todo_from_row)?;
        rows.collect()
    }

    pub fn restore_if_unchanged(
        &self,
        projects: &[Project],
        todos: &[Todo],
        expected_data_version: i64,
    ) -> Result<RestoreOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let current_data_version: i64 =
            tx.query_row("PRAGMA data_version", [], |row| row.get(0))?;
        if current_data_version != expected_data_version {
            return Ok(RestoreOutcome::ChangedExternally);
        }
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
        Ok(RestoreOutcome::Restored)
    }

    pub fn add_todo(
        &self,
        text: &str,
        parent_id: Option<TodoId>,
        project_id: ProjectId,
    ) -> Result<TodoId> {
        let tx = self.conn.unchecked_transaction()?;
        let id = insert_todo(&tx, text, parent_id, project_id)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn add_subtask(&self, text: &str, parent_id: TodoId) -> Result<TodoId> {
        let tx = self.conn.unchecked_transaction()?;
        let project_id: ProjectId = tx.query_row(
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

    pub fn update_todo_text(&self, id: TodoId, text: &str) -> Result<()> {
        let changed_rows = self
            .conn
            .execute("UPDATE todos SET text = ?1 WHERE id = ?2", (text, id))?;
        ensure_row_changed(changed_rows)
    }

    pub fn set_done_many(&self, updates: &[(TodoId, bool)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET done = ?1 WHERE id = ?2")?;
            for &(id, done) in updates {
                ensure_row_changed(stmt.execute((done, id))?)?;
            }
        }
        tx.commit()
    }

    pub fn indent(
        &self,
        id: TodoId,
        parent: TodoId,
        parent_completion: ParentCompletion,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let position = next_todo_position(&tx)?;
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = ?1, position = ?2 WHERE id = ?3",
            (parent, position, id),
        )?)?;
        ensure_row_changed(tx.execute("UPDATE todos SET collapsed = 0 WHERE id = ?1", [parent])?)?;
        if parent_completion == ParentCompletion::Reopen {
            ensure_row_changed(tx.execute("UPDATE todos SET done = 0 WHERE id = ?1", [parent])?)?;
        }
        tx.commit()
    }

    pub fn set_todo_positions(&self, order: &[TodoId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        renumber(&tx, "UPDATE todos SET position = ?1 WHERE id = ?2", order)?;
        tx.commit()
    }

    pub fn move_to_project(
        &self,
        root: TodoId,
        subtree: &[TodoId],
        project_id: ProjectId,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let position = next_todo_position(&tx)?;
        {
            let mut stmt = tx.prepare("UPDATE todos SET project_id = ?1 WHERE id = ?2")?;
            for id in subtree {
                ensure_row_changed(stmt.execute((project_id, id))?)?;
            }
        }
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = NULL, position = ?1 WHERE id = ?2",
            (position, root),
        )?)?;
        tx.commit()
    }

    pub fn outdent(
        &self,
        id: TodoId,
        new_parent: Option<TodoId>,
        sibling_order: &[TodoId],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        ensure_row_changed(tx.execute(
            "UPDATE todos SET parent_id = ?1 WHERE id = ?2",
            (new_parent, id),
        )?)?;
        renumber(
            &tx,
            "UPDATE todos SET position = ?1 WHERE id = ?2",
            sibling_order,
        )?;
        tx.commit()
    }

    pub fn set_collapsed(&self, id: TodoId, collapsed: bool) -> Result<()> {
        let changed_rows = self.conn.execute(
            "UPDATE todos SET collapsed = ?1 WHERE id = ?2",
            (collapsed, id),
        )?;
        ensure_row_changed(changed_rows)
    }

    pub fn delete_todo(&self, id: TodoId) -> Result<()> {
        let changed_rows = self.conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
        ensure_row_changed(changed_rows)
    }
}

fn push_nested(all: &[Todo], parent: Option<TodoId>, out: &mut Vec<Todo>) {
    for t in all.iter().filter(|t| t.parent_id == parent) {
        out.push(t.clone());
        push_nested(all, Some(t.id), out);
    }
}

fn renumber(conn: &Connection, sql: &str, order: &[impl ToSql]) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    for (position, id) in (1_i64..).zip(order) {
        ensure_row_changed(stmt.execute((position, id))?)?;
    }
    Ok(())
}

fn next_todo_position(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM todos",
        [],
        |r| r.get(0),
    )
}

fn insert_todo(
    conn: &Connection,
    text: &str,
    parent_id: Option<TodoId>,
    project_id: ProjectId,
) -> Result<TodoId> {
    let now = Local::now().timestamp();
    let position = next_todo_position(conn)?;
    let changed_rows = conn.execute(
        "INSERT INTO todos (text, created_at, done, position, parent_id, project_id)
         SELECT ?1, ?2, 0, ?3, ?4, ?5
         WHERE ?4 IS NULL
            OR EXISTS (
                SELECT 1 FROM todos parent
                WHERE parent.id = ?4 AND parent.project_id = ?5
            )",
        (text, now, position, parent_id, project_id),
    )?;
    ensure_row_changed(changed_rows)?;
    Ok(TodoId(conn.last_insert_rowid()))
}

fn ensure_row_changed(changed_rows: usize) -> Result<()> {
    if changed_rows == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'todos')",
        [],
        |r| r.get(0),
    )?;

    if has_table {
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

    fn default_project(s: &Store) -> ProjectId {
        s.list_projects().unwrap()[0].id
    }

    fn position_of(s: &Store, id: TodoId) -> i64 {
        s.conn
            .query_row("SELECT position FROM todos WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn texts(s: &Store, project_id: ProjectId) -> Vec<String> {
        s.list_todos(project_id)
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect()
    }

    #[test]
    fn add_list_update_toggle_delete_roundtrip() {
        let s = mem_store();
        let pid = default_project(&s);
        assert!(s.list_todos(pid).unwrap().is_empty());

        let id = s.add_todo("첫 번째 할 일", None, pid).unwrap();
        let todos = s.list_todos(pid).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "첫 번째 할 일");
        assert_eq!(todos[0].project_id, pid);
        assert!(!todos[0].done);
        assert!(todos[0].created_at > 0);

        s.update_todo_text(id, "수정됨").unwrap();
        let t = &s.list_todos(pid).unwrap()[0];
        assert_eq!(t.text, "수정됨");

        s.set_done_many(&[(id, true)]).unwrap();
        assert!(s.list_todos(pid).unwrap()[0].done);

        s.delete_todo(id).unwrap();
        assert!(s.list_todos(pid).unwrap().is_empty());
    }

    #[test]
    fn done_items_sink_to_bottom() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add_todo("a", None, pid).unwrap();
        s.add_todo("b", None, pid).unwrap();
        s.add_todo("c", None, pid).unwrap();

        s.set_done_many(&[(a, true)]).unwrap();
        assert_eq!(texts(&s, pid), ["b", "c", "a"]);

        s.set_done_many(&[(a, false)]).unwrap();
        assert_eq!(texts(&s, pid), ["a", "b", "c"]);
    }

    #[test]
    fn done_children_sink_within_parent() {
        let s = mem_store();
        let pid = default_project(&s);
        let p = s.add_todo("p", None, pid).unwrap();
        let c1 = s.add_todo("c1", Some(p), pid).unwrap();
        s.add_todo("c2", Some(p), pid).unwrap();
        s.add_todo("q", None, pid).unwrap();

        s.set_done_many(&[(c1, true)]).unwrap();
        assert_eq!(texts(&s, pid), ["p", "c2", "c1", "q"]);
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
        let t = &store.list_todos(pid).unwrap()[0];
        assert_eq!(t.text, "old");
        assert_eq!(t.project_id, pid);
        assert_eq!(position_of(&store, t.id), t.id.0);
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

        store.delete_todo(TodoId(1)).unwrap();
        assert!(store.list_todos(pid).unwrap().is_empty());
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

        assert_eq!(texts(&store, ProjectId(1)), ["마감이 있던 항목"]);
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
        let a = s.add_todo("a", None, pid).unwrap();
        s.add_todo("b", None, pid).unwrap();
        s.add_todo("a2", Some(a), pid).unwrap();
        s.add_todo("a1", Some(a), pid).unwrap();
        assert_eq!(texts(&s, pid), ["a", "a2", "a1", "b"]);
    }

    #[test]
    fn list_nests_three_levels() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add_todo("a", None, pid).unwrap();
        s.add_todo("b", None, pid).unwrap();
        let a1 = s.add_todo("a1", Some(a), pid).unwrap();
        s.add_todo("a1-1", Some(a1), pid).unwrap();
        assert_eq!(texts(&s, pid), ["a", "a1", "a1-1", "b"]);
    }

    #[test]
    fn delete_parent_removes_children() {
        let s = mem_store();
        let pid = default_project(&s);
        let p = s.add_todo("p", None, pid).unwrap();
        s.add_todo("c1", Some(p), pid).unwrap();
        s.add_todo("c2", Some(p), pid).unwrap();
        assert_eq!(s.list_todos(pid).unwrap().len(), 3);
        s.delete_todo(p).unwrap();
        assert!(s.list_todos(pid).unwrap().is_empty());
    }

    #[test]
    fn projects_are_isolated() {
        let s = mem_store();
        let p1 = default_project(&s);
        let p2 = s.add_project("업무").unwrap();
        s.add_todo("개인 일", None, p1).unwrap();
        s.add_todo("회사 일", None, p2).unwrap();
        assert_eq!(texts(&s, p1), ["개인 일"]);
        assert_eq!(texts(&s, p2), ["회사 일"]);
    }

    #[test]
    fn add_rejects_a_parent_from_another_project() {
        let store = mem_store();
        let personal_project_id = default_project(&store);
        let work_project_id = store.add_project("업무").unwrap();
        let parent_id = store
            .add_todo("개인 상위", None, personal_project_id)
            .unwrap();

        assert!(
            store
                .add_todo("잘못된 하위", Some(parent_id), work_project_id)
                .is_err()
        );
        assert_eq!(texts(&store, personal_project_id), ["개인 상위"]);
        assert!(store.list_todos(work_project_id).unwrap().is_empty());
    }

    #[test]
    fn mutations_report_missing_rows() {
        let store = mem_store();

        assert!(store.update_todo_text(TodoId(999), "없음").is_err());
        assert!(store.delete_todo(TodoId(999)).is_err());
        assert!(store.rename_project(ProjectId(999), "없음").is_err());
    }

    #[test]
    fn delete_project_cascades_todos() {
        let s = mem_store();
        let p1 = default_project(&s);
        let p2 = s.add_project("업무").unwrap();
        let t = s.add_todo("회사 일", None, p2).unwrap();
        s.add_todo("하위", Some(t), p2).unwrap();
        s.delete_project(p2).unwrap();
        assert!(s.list_all_todos().unwrap().is_empty());
        assert_eq!(s.list_projects().unwrap().len(), 1);
        assert_eq!(s.list_projects().unwrap()[0].id, p1);
    }

    #[test]
    fn replace_all_restores_snapshot() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add_todo("a", None, pid).unwrap();
        s.add_todo("a1", Some(a), pid).unwrap();

        let projects = s.list_projects().unwrap();
        let todos = s.list_all_todos().unwrap();

        s.delete_todo(a).unwrap();
        s.add_project("임시").unwrap();
        assert!(s.list_todos(pid).unwrap().is_empty());

        let data_version = s.data_version().unwrap();
        assert_eq!(
            s.restore_if_unchanged(&projects, &todos, data_version)
                .unwrap(),
            RestoreOutcome::Restored
        );
        assert_eq!(texts(&s, pid), ["a", "a1"]);
        assert_eq!(s.list_projects().unwrap(), projects);
    }

    #[test]
    fn replace_all_handles_parent_with_larger_id() {
        let s = mem_store();
        let pid = default_project(&s);
        let a = s.add_todo("a", None, pid).unwrap();
        let b = s.add_todo("b", None, pid).unwrap();
        s.set_todo_positions(&[b, a]).unwrap();
        s.indent(a, b, ParentCompletion::Reopen).unwrap();

        let projects = s.list_projects().unwrap();
        let todos = s.list_all_todos().unwrap();
        let data_version = s.data_version().unwrap();
        assert_eq!(
            s.restore_if_unchanged(&projects, &todos, data_version)
                .unwrap(),
            RestoreOutcome::Restored
        );
        assert_eq!(texts(&s, pid), ["b", "a"]);
    }
}
