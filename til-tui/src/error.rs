use thiserror::Error;

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("데이터베이스 오류: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("파일 시스템 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    InvalidInput(String),
    #[error("#{0} TIL 기록을 찾을 수 없습니다")]
    EntryNotFound(i64),
}
