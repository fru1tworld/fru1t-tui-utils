use clap::{Parser, Subcommand};

use crate::app::MAX_DEPTH;
use crate::db::{Store, Todo};

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
                Some(name) => store.list(resolve_project(&store, &name)?)?,
                None => store.list_all()?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&todos)?);
            } else {
                for t in &todos {
                    let check = if t.done { "x" } else { " " };
                    let indent = "  ".repeat(depth_of(&todos, t));
                    println!("{indent}[{check}] #{} {}", t.id, t.text);
                }
            }
        }
        Command::Add {
            text,
            project,
            parent,
        } => {
            let todos = store.list_all()?;
            let project_id = resolve_target_project(&store, &todos, project.as_deref(), parent)?;
            if let Some(parent_id) = parent {
                ensure_depth(&todos, parent_id)?;
            }
            println!("{}", store.add(&text, parent, project_id)?);
        }
        Command::Subtask { parent_id, text } => {
            ensure_depth(&store.list_all()?, parent_id)?;
            println!("{}", store.add_subtask(&text, parent_id)?);
        }
        Command::Done { id } => set_done_with_tree_rules(&store, id, true)?,
        Command::Undone { id } => set_done_with_tree_rules(&store, id, false)?,
        Command::Delete { id } => store.delete(id)?,
        Command::Edit { id, text } => store.update_text(id, &text)?,
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

/// 부모 아래에 항목을 하나 더 넣어도 최대 깊이를 넘지 않는지 확인한다.
fn ensure_depth(todos: &[Todo], parent_id: i64) -> anyhow::Result<()> {
    let parent = todos
        .iter()
        .find(|t| t.id == parent_id)
        .ok_or_else(|| anyhow::anyhow!("#{parent_id} 할 일을 찾을 수 없습니다"))?;
    if depth_of(todos, parent) + 1 >= MAX_DEPTH {
        anyhow::bail!("하위 목표는 {MAX_DEPTH}단계까지만 넣을 수 있습니다");
    }
    Ok(())
}

fn resolve_target_project(
    store: &Store,
    todos: &[Todo],
    project_name: Option<&str>,
    parent_id: Option<i64>,
) -> anyhow::Result<i64> {
    let requested_project_id = project_name
        .map(|name| resolve_project(store, name))
        .transpose()?;

    if let Some(parent_id) = parent_id {
        let parent_project_id = todos
            .iter()
            .find(|todo| todo.id == parent_id)
            .map(|todo| todo.project_id)
            .ok_or_else(|| anyhow::anyhow!("#{parent_id} 할 일을 찾을 수 없습니다"))?;
        if requested_project_id.is_some_and(|project_id| project_id != parent_project_id) {
            anyhow::bail!("상위 할 일과 같은 프로젝트에만 하위 목표를 추가할 수 있습니다");
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
        .ok_or_else(|| anyhow::anyhow!("프로젝트가 없습니다"))
}

fn set_done_with_tree_rules(store: &Store, id: i64, done: bool) -> anyhow::Result<()> {
    let todos = store.list_all()?;
    let target = todos
        .iter()
        .find(|todo| todo.id == id)
        .ok_or_else(|| anyhow::anyhow!("#{id} 할 일을 찾을 수 없습니다"))?;
    let mut updates = vec![(target.id, done)];
    append_descendants(&todos, target.id, done, &mut updates);

    if !done {
        let mut parent_id = target.parent_id;
        while let Some(id) = parent_id {
            let parent = todos
                .iter()
                .find(|todo| todo.id == id)
                .ok_or_else(|| anyhow::anyhow!("#{id} 상위 할 일을 찾을 수 없습니다"))?;
            updates.push((parent.id, false));
            parent_id = parent.parent_id;
        }
    }

    store.set_done_many(&updates)?;
    Ok(())
}

fn append_descendants(todos: &[Todo], parent_id: i64, done: bool, updates: &mut Vec<(i64, bool)>) {
    for child in todos
        .iter()
        .filter(|todo| todo.parent_id == Some(parent_id))
    {
        updates.push((child.id, done));
        append_descendants(todos, child.id, done, updates);
    }
}

/// 목록 안에서의 깊이(최상위 = 0). 부모를 따라 올라가며 센다.
fn depth_of(todos: &[Todo], todo: &Todo) -> usize {
    std::iter::successors(Some(todo), |t| {
        t.parent_id
            .and_then(|pid| todos.iter().find(|p| p.id == pid))
    })
    .count()
        - 1
}

fn resolve_project(store: &Store, name: &str) -> anyhow::Result<i64> {
    store
        .list_projects()?
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id)
        .ok_or_else(|| anyhow::anyhow!("프로젝트 '{name}'을(를) 찾을 수 없습니다"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(std::path::PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn target_project_follows_the_parent() {
        let store = store();
        let personal_project_id = store.list_projects().unwrap()[0].id;
        let parent_id = store.add("상위", None, personal_project_id).unwrap();
        let todos = store.list_all().unwrap();

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
        let parent_id = store.add("상위", None, personal_project_id).unwrap();
        let todos = store.list_all().unwrap();

        assert!(resolve_target_project(&store, &todos, Some("업무"), Some(parent_id)).is_err());
    }

    #[test]
    fn completion_changes_follow_the_whole_tree() {
        let store = store();
        let project_id = store.list_projects().unwrap()[0].id;
        let parent_id = store.add("상위", None, project_id).unwrap();
        let child_id = store.add("하위", Some(parent_id), project_id).unwrap();
        let leaf_id = store.add("손자", Some(child_id), project_id).unwrap();

        set_done_with_tree_rules(&store, parent_id, true).unwrap();
        assert!(store.list(project_id).unwrap().iter().all(|todo| todo.done));

        set_done_with_tree_rules(&store, leaf_id, false).unwrap();
        assert!(
            store
                .list(project_id)
                .unwrap()
                .iter()
                .all(|todo| !todo.done)
        );
    }
}
