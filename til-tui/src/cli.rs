use chrono::{Local, NaiveDate};
use clap::{Parser, Subcommand};
use serde_json::json;

use crate::db::SqliteTilRepository;
use crate::domain::validate_content;
use crate::error::{Error, Result};
use crate::output;

#[derive(Parser)]
#[command(name = "til-tui", about = "오늘 배운 것을 기록하는 TUI + CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// 날짜별 기록 조회
    List {
        /// today, yesterday 또는 YYYY-MM-DD
        #[arg(short, long, default_value = "today")]
        date: String,
        /// AI가 읽기 좋은 JSON 출력
        #[arg(long)]
        json: bool,
    },
    /// 기록 추가
    Add {
        /// 기록 내용
        text: String,
        /// 기록할 날짜: today, yesterday 또는 YYYY-MM-DD
        #[arg(short, long)]
        date: String,
    },
    /// 날짜별 기록을 마크다운으로 출력
    Out {
        /// today, yesterday 또는 YYYY-MM-DD
        #[arg(short, long, default_value = "today")]
        date: String,
        /// AI가 읽기 좋은 JSON 출력
        #[arg(long)]
        json: bool,
    },
    /// 기록 내용 수정
    Edit {
        /// 기록 ID
        id: i64,
        /// 새 내용
        text: String,
    },
    /// 기록을 다른 날짜로 이동
    Move {
        /// 기록 ID
        id: i64,
        /// 이동할 날짜: today, yesterday 또는 YYYY-MM-DD
        #[arg(short, long)]
        date: String,
    },
    /// 기록 삭제
    #[command(name = "rm")]
    Delete {
        /// 기록 ID
        id: i64,
    },
}

pub(crate) fn run(command: Command) -> anyhow::Result<()> {
    let repository = SqliteTilRepository::open_default()?;
    match command {
        Command::List { date, json } => {
            let date = parse_date(&date)?;
            let entries = repository.entries_on(date)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "date": date.to_string(),
                        "entries": entries,
                    }))?
                );
            } else {
                for entry in entries {
                    println!("#{} {} {}", entry.id, entry.time_label(), entry.content);
                }
            }
        }
        Command::Add { text, date } => {
            let entry = repository.create_on(parse_date(&date)?, validate_content(&text)?)?;
            println!("{}", entry.id);
        }
        Command::Out { date, json } => {
            let date = parse_date(&date)?;
            let entries = repository.entries_on(date)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "date": date.to_string(),
                        "entries": entries,
                    }))?
                );
            } else {
                print!("{}", output::format_day_as_markdown(date, &entries));
            }
        }
        Command::Edit { id, text } => {
            repository.update_content(id, validate_content(&text)?)?;
        }
        Command::Move { id, date } => {
            repository.move_to_date(id, parse_date(&date)?)?;
        }
        Command::Delete { id } => {
            repository.delete(id)?;
        }
    }
    Ok(())
}

fn parse_date(input: &str) -> Result<NaiveDate> {
    let today = Local::now().date_naive();
    match input.trim().to_lowercase().as_str() {
        "today" | "오늘" => Ok(today),
        "yesterday" | "어제" => Ok(today - chrono::Duration::days(1)),
        value => NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
            Error::InvalidInput("날짜는 today, yesterday 또는 YYYY-MM-DD로 입력하세요".into())
        }),
    }
}
