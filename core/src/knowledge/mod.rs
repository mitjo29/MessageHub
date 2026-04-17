pub mod embedder;
pub mod parser;
pub mod people;
// indexer, retrieval — added in later tasks

pub use embedder::{Embedder, EMBEDDING_DIM};
pub use parser::{ParsedFile, Section, parse_markdown_file};
pub use people::{PersonAddress, VaultPerson, extract_person};
