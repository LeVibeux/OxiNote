use crate::NotePath;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(NotePath),
    #[error("already exists: {0}")]
    AlreadyExists(NotePath),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
