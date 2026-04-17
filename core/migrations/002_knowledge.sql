-- Tracks every indexed markdown file for incremental updates.
CREATE TABLE IF NOT EXISTS vault_files (
    path TEXT PRIMARY KEY,              -- Relative path from vault root, e.g. "05-People/Alix Moreau.md"
    content_hash TEXT NOT NULL,         -- blake3 hash of file content
    mtime_secs INTEGER NOT NULL,        -- File modification time (Unix timestamp)
    frontmatter_json TEXT,              -- Parsed YAML frontmatter as JSON (nullable — files may have none)
    para_folder TEXT,                   -- "00-Inbox", "01-Projects", "05-People", etc. (nullable)
    indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_vault_files_para ON vault_files(para_folder);

-- One row per chunk. Joined with the vec virtual table on `id`.
CREATE TABLE IF NOT EXISTS vault_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES vault_files(path) ON DELETE CASCADE,
    section_heading TEXT,               -- The ## heading that introduced this chunk, if any
    chunk_index INTEGER NOT NULL,       -- Order within the file (0-based)
    content TEXT NOT NULL,              -- The actual chunk text (without E5 prefix)
    token_count INTEGER NOT NULL,       -- Approximate token count for budget calculations
    para_folder TEXT                    -- Denormalized from vault_files for fast filtering
);
CREATE INDEX IF NOT EXISTS idx_vault_chunks_file ON vault_chunks(file_path);
CREATE INDEX IF NOT EXISTS idx_vault_chunks_para ON vault_chunks(para_folder);

-- sqlite-vec virtual table: 384-dim vectors (multilingual-e5-small output dimension).
-- The ROWID here matches vault_chunks.id — we keep them in sync via insert/delete triggers below.
CREATE VIRTUAL TABLE IF NOT EXISTS vault_chunk_vecs USING vec0(
    embedding FLOAT[384]
);

-- Structured people profiles extracted from 05-People/*.md frontmatter.
-- This is the "structured" half of dual-indexing — the chunks themselves
-- also go into vault_chunks for semantic search.
CREATE TABLE IF NOT EXISTS vault_people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL UNIQUE REFERENCES vault_files(path) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT,                          -- e.g. "Daughter (youngest)"
    tags_json TEXT,                     -- JSON array: ["person", "family", "children"]
    last_contact TEXT,                  -- ISO date string
    frontmatter_json TEXT NOT NULL      -- Full frontmatter preserved as JSON for flexible queries
);
CREATE INDEX IF NOT EXISTS idx_vault_people_name ON vault_people(name);

-- Contact-to-vault-person link. Populated when a message sender matches a
-- known address from this person's profile (via vault_people.frontmatter_json
-- email/phone fields — parsed at query time).
CREATE TABLE IF NOT EXISTS vault_people_addresses (
    person_id INTEGER NOT NULL REFERENCES vault_people(id) ON DELETE CASCADE,
    channel_type TEXT NOT NULL,
    address TEXT NOT NULL,
    PRIMARY KEY (person_id, channel_type, address)
);
CREATE INDEX IF NOT EXISTS idx_vault_people_addresses_lookup
    ON vault_people_addresses(channel_type, address);
