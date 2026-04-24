# MessageHub

A modular Rust library for intelligent message processing with local AI capabilities.

## Overview

MessageHub provides building blocks for applications that need to:
- Fetch messages from email (IMAP/SMTP) and Telegram
- Classify and enrich messages using local LLMs (Ollama)
- Store messages in encrypted SQLite with semantic search
- Build knowledge bases from markdown notes with vector embeddings

## Project Status

✅ **Alpha** - Core functionality implemented, test coverage good. API may change.

## Features

- **AI Pipeline**: Classify messages by category/priority with graceful degradation
- **Local LLM Support**: Works with Ollama for private, on-device inference
- **Multi-Protocol**: Email (IMAP/SMTP) and Telegram adapters
- **Encrypted Storage**: SQLCipher for encrypted SQLite databases
- **Semantic Search**: Vector embeddings for knowledge retrieval
- **Knowledge Graph**: Extract and query people/organizations from notes

## Quick Start

### Prerequisites

- Rust 1.70+ (edition 2021)
- SQLite (bundled)
- Optional: Ollama for AI features

### Build and Test

```bash
# Run all tests (unit + integration)
cargo test --workspace

# Run tests with output
cargo test --workspace -- --nocapture

# Run specific test
cargo test test_classify_happy_path --workspace
```

### Usage as a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
messagehub-core = "0.1.0"
```

Example: Classify a message using Ollama

```rust
use messagehub_core::ai::{LlmBackend, Classifier, AiPipeline};
use messagehub_core::types::{Message, Category, Priority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to local Ollama instance
    let llm = LlmBackend::ollama("http://localhost:11434", "llama3.2");

    // Create a classifier
    let classifier = Classifier::new(llm);

    // Classify a message
    let message = Message {
        subject: Some("Urgent: Server down".to_string()),
        body: "Production database is not responding".to_string(),
        ..Default::default()
    };

    let result = classifier.classify(&message).await?;
    println!("Category: {:?}, Priority: {:?}", result.category, result.priority);

    Ok(())
}
```

## Architecture

```
messagehub-core/
├── adapters/      # Protocol implementations (Email, Telegram)
├── ai/            # LLM integration and classification
├── knowledge/     # Knowledge base with embeddings
└── store/         # SQLite storage with encryption
```

### Key Components

#### Adapters (`adapters/`)
- **EmailAdapter**: IMAP fetch + SMTP send with credential parsing
- **TelegramAdapter**: Bot API integration
- **AdapterManager**: Lifecycle management for multiple adapters

#### AI (`ai/`)
- **LlmBackend**: Abstraction over Ollama HTTP API
- **Classifier**: Message categorization (Category + Priority)
- **AiPipeline**: Orchestrates classification with enrichment and logging
- **RagContext**: Builds retrieval-augmented generation context
- **Prompts**: Template-based prompt engineering

#### Knowledge (`knowledge/`)
- **Indexer**: Walks markdown vault, extracts sections
- **Embedder**: FastEmbed for vector generation (384-dim)
- **Retrieval**: SQLite-vec for similarity search
- **Parser**: Frontmatter and section extraction

#### Store (`store/`)
- **Messages**: CRUD with FTS5 search
- **Contacts**: Identity resolution across channels
- **Knowledge**: Upsert chunks with embeddings
- **AiLog**: Decision logging for audit/debug

## Configuration

### Ollama Setup

```bash
# Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# Pull a model
ollama pull llama3.2

# Run (default: http://localhost:11434)
ollama serve
```

### Database Encryption

```rust
use messagehub_core::store::MessagesStore;

let store = MessagesStore::new_in_memory("encryption_key")?;
// All data is encrypted at rest using SQLCipher
```

## Testing

### Unit Tests
Fast, no external dependencies:
```bash
cargo test --workspace --lib
```

### Integration Tests
Some require Ollama running:
```bash
# Start Ollama first
ollama serve

# Run AI tests
cargo test ai --workspace
```

### Ignored Tests
- Tests requiring ~120MB model download (embeddings)
- Tests requiring running Ollama instance

Run ignored tests explicitly:
```bash
cargo test --workspace -- --ignored
```

## Development

### Project Structure

This is a **workspace** with one member:
```
MessageHub/
├── Cargo.toml          # Workspace config
└── core/
    ├── Cargo.toml      # Library crate
    └── src/            # Source code
```

### Adding Features

1. New adapter? Implement `Adapter` trait in `adapters/`
2. New category? Add to `Category` enum in `types/`
3. New LLM backend? Extend `LlmBackend` in `ai/`

## Design Decisions

- **Library over binary**: Reusable components, not a standalone app
- **Local-first AI**: Ollama for privacy and offline use
- **Graceful degradation**: AI failures don't crash the app
- **SQLite over PostgreSQL**: Simpler deployment, good enough for scale
- **Async/await**: Tokio runtime for concurrent I/O

## Roadmap

- [ ] REST API wrapper
- [ ] Web dashboard
- [ ] More LLM providers (OpenAI, Anthropic)
- [ ] Streaming message processing
- [ ] Export/import tools
- [ ] Performance benchmarks

## License

AGPL-3.0

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Submit a pull request

## Resources

- [Ollama Documentation](https://ollama.com/docs)
- [SQLite-vec](https://github.com/asg017/sqlite-vec)
- [FastEmbed](https://github.com/Anush008/fastembed-rs)

## Changelog

### 0.1.0 (Current)
- Initial release
- Email and Telegram adapters
- Ollama integration for classification
- Encrypted SQLite storage
- Knowledge base with semantic search
