use clap::{Parser, Subcommand};

use anyhow::{Context, anyhow, bail};

use crate::db::{ProjectId, Store, Todo, TodoId};
use crate::tree::{self, MAX_DEPTH};

#[derive(Parser)]
#[command(name = "todo-tui", about = "TUI 할 일 관리 + CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// 할 일 목록 조회
    List {
        /// 프로젝트 이름 (생략 시 전체)
        #[arg(short, long)]
        project: Option<String>,
        /// JSON 출력
        #[arg(long)]
        json: bool,
    },
    /// 할 일 추가
    Add {
        /// 할 일 내용
        text: String,
        /// 프로젝트 이름 (생략 시 기본)
        #[arg(short, long)]
        project: Option<String>,
        /// 상위 할 일 ID
        #[arg(long)]
        parent: Option<i64>,
    },
    /// 하위 목표 추가
    Subtask {
        /// 상위 할 일 ID
        parent_id: i64,
        /// 할 일 내용
        text: String,
    },
    /// 완료 처리
    Done {
        /// 할 일 ID
        id: i64,
    },
    /// 미완료 처리
    Undone {
        /// 할 일 ID
        id: i64,
    },
    /// 삭제
    #[command(name = "rm")]
    Delete {
        /// 할 일 ID
        id: i64,
    },
    /// 내용 수정
    Edit {
        /// 할 일 ID
        id: i64,
        /// 새 내용
        text: String,
    },
    /// 프로젝트 목록
    Projects {
        /// JSON 출력
        #[arg(long)]
        json: bool,
    },
    /// 프로젝트 추가
    AddProject {
        /// 프로젝트 이름
        name: String,
    },
}

pub(crate) fn run(cmd: Command) -> anyhow::Result<()> {
    let store = Store::open_default()?;

    match cmd {
        Command::List { project, json } => {
            let todos = match project {
                Some(name) => store.list_todos(resolve_project(&store, &name)?)?,
                None => store.list_all_todos()?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&todos)?);
            } else {
                for t in &todos {
                    let check = if t.done { "x" } else { " " };
                    let indent = "  ".repeat(tree::depth(&todos, t.id));
                    println!("{indent}[{check}] #{} {}", t.id, t.text);
                }
            }
        }
        Command::Add {
            text,
            project,
            parent,
        } => {
            let parent_id = parent.map(TodoId);
            let todos = store.list_all_todos()?;
            let project_id = resolve_target_project(&store, &todos, project.as_deref(), parent_id)?;
            if let Some(parent_id) = parent_id {
                ensure_accepts_child(&todos, parent_id)?;
            }
            println!("{}", store.add_todo(&text, parent_id, project_id)?);
        }
        Command::Subtask { parent_id, text } => {
            let parent_id = TodoId(parent_id);
            ensure_accepts_child(&store.list_all_todos()?, parent_id)?;
            println!("{}", store.add_subtask(&text, parent_id)?);
        }
        Command::Done { id } => set_done_with_tree_rules(&store, TodoId(id), true)?,
        Command::Undone { id } => set_done_with_tree_rules(&store, TodoId(id), false)?,
        Command::Delete { id } => store.delete_todo(TodoId(id))?,
        Command::Edit { id, text } => store.update_todo_text(TodoId(id), &text)?,
        Command::Projects { json } => {
            let projects = store.list_projects()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else {
                for p in &projects {
                    println!("#{} {}", p.id, p.name);
                }
            }
        }
        Command::AddProject { name } => {
            println!("{}", store.add_project(&name)?);
        }
    }
    Ok(())
}

fn find_todo(todos: &[Todo], id: TodoId) -> anyhow::Result<&Todo> {
    tree::find(todos, id).with_context(|| format!("#{id} 할 일을 찾을 수 없습니다"))
}

fn ensure_accepts_child(todos: &[Todo], parent_id: TodoId) -> anyhow::Result<()> {
    find_todo(todos, parent_id)?;
    if !tree::accepts_child(todos, parent_id) {
        bail!("하위 목표는 {MAX_DEPTH}단계까지만 넣을 수 있습니다");
    }
    Ok(())
}

fn resolve_target_project(
    store: &Store,
    todos: &[Todo],
    project_name: Option<&str>,
    parent_id: Option<TodoId>,
) -> anyhow::Result<ProjectId> {
    let requested_project_id = project_name
        .map(|name| resolve_project(store, name))
        .transpose()?;

    if let Some(parent_id) = parent_id {
        let parent_project_id = find_todo(todos, parent_id)?.project_id;
        if requested_project_id.is_some_and(|project_id| project_id != parent_project_id) {
            bail!("상위 할 일과 같은 프로젝트에만 하위 목표를 추가할 수 있습니다");
        }
        return Ok(parent_project_id);
    }

    if let Some(project_id) = requested_project_id {
        return Ok(project_id);
    }
    store
        .list_projects()?
        .first()
        .map(|project| project.id)
        .ok_or_else(|| anyhow!("프로젝트가 없습니다"))
}

fn set_done_with_tree_rules(store: &Store, id: TodoId, done: bool) -> anyhow::Result<()> {
    let todos = store.list_all_todos()?;
    find_todo(&todos, id)?;
    store.set_done_many(&tree::completion_updates(&todos, id, done))?;
    Ok(())
}

fn resolve_project(store: &Store, name: &str) -> anyhow::Result<ProjectId> {
    store
        .list_projects()?
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id)
        .with_context(|| format!("프로젝트 '{name}'을(를) 찾을 수 없습니다"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn target_project_follows_the_parent() {
        let store = store();
        let personal_project_id = store.list_projects().unwrap()[0].id;
        let parent_id = store.add_todo("상위", None, personal_project_id).unwrap();
        let todos = store.list_all_todos().unwrap();

        assert_eq!(
            resolve_target_project(&store, &todos, None, Some(parent_id)).unwrap(),
            personal_project_id
        );
    }

    #[test]
    fn target_project_rejects_a_project_that_differs_from_the_parent() {
        let store = store();
        let personal_project_id = store.list_projects().unwrap()[0].id;
        store.add_project("업무").unwrap();
        let parent_id = store.add_todo("상위", None, personal_project_id).unwrap();
        let todos = store.list_all_todos().unwrap();

        assert!(resolve_target_project(&store, &todos, Some("업무"), Some(parent_id)).is_err());
    }

    #[test]
    fn completion_changes_follow_the_whole_tree() {
        let store = store();
        let project_id = store.list_projects().unwrap()[0].id;
        let parent_id = store.add_todo("상위", None, project_id).unwrap();
        let child_id = store.add_todo("하위", Some(parent_id), project_id).unwrap();
        let leaf_id = store.add_todo("손자", Some(child_id), project_id).unwrap();

        set_done_with_tree_rules(&store, parent_id, true).unwrap();
        assert!(
            store
                .list_todos(project_id)
                .unwrap()
                .iter()
                .all(|todo| todo.done)
        );

        set_done_with_tree_rules(&store, leaf_id, false).unwrap();
        assert!(
            store
                .list_todos(project_id)
                .unwrap()
                .iter()
                .all(|todo| !todo.done)
        );
    }
}
