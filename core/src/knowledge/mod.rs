pub mod embedder;
pub mod parser;
// people, indexer, retrieval — added in later tasks

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use parser::{ParsedFile, Section, parse_markdown_file};
