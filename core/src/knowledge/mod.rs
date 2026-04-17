pub mod embedder;
pub mod indexer;
pub mod parser;
pub mod people;
pub mod retrieval;

pub use embedder::{EMBEDDING_DIM, Embedder};
pub use indexer::{IndexOutcome, Indexer, IndexingReport};
pub use parser::{ParsedFile, Section, parse_markdown_file};
pub use people::{PersonAddress, VaultPerson, extract_person};
pub use retrieval::{RetrievalFilters, RetrievedChunk, Retriever};
