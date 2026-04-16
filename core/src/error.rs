use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("channel error: {0}")]
    Channel(String),

    #[error("connection error: {0}")]
    Connection(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("parse error: {0}")]
    Parse(String),
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::Network(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
