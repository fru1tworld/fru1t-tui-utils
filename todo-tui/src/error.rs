use thiserror::Error;

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("데이터베이스 오류: {0}")]
    Db(#[from] rusqlite::Error),
    /// 사용자 입력 검증 실패. 상태 표시줄에 보여주고 편집을 계속한다.
    #[error("{0}")]
    Invalid(String),
}
