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
}

pub type Result<T> = std::result::Result<T, CoreError>;
