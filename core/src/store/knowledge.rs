use rusqlite::params;

use crate::error::{CoreError, Result};
use crate::knowledge::{ParsedFile, VaultPerson};
use crate::store::Store;

/// Everything the indexer needs to persist for a single markdown file.
pub struct IndexedFile<'a> {
    pub path: &'a str,
    pub mtime_secs: i64,
    pub para_folder: Option<&'a str>,
    pub parsed: &'a ParsedFile,
    /// One embedding per section (same order as `parsed.sections`).
    pub chunk_embeddings: &'a [Vec<f32>],
    /// If this is a valid person profile, the extracted structured data.
    pub person: Option<&'a VaultPerson>,
}

impl Store {
    /// Insert or replace a file's knowledge representation transactionally.
    ///
    /// If the file already exists in `vault_files`, all its chunks, vectors,
    /// and person row are deleted and re-inserted. This keeps incremental
    /// updates simple: callers just re-call `upsert_indexed_file`.
    pub fn upsert_indexed_file(&self, file: &IndexedFile<'_>) -> Result<()> {
        if file.chunk_embeddings.len() != file.parsed.sections.len() {
            return Err(CoreError::Knowledge(format!(
                "chunk count mismatch: {} sections, {} embeddings",
                file.parsed.sections.len(),
                file.chunk_embeddings.len()
            )));
        }

        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.upsert_indexed_file_inner(file);
        match &result {
            Ok(_) => self.conn().execute_batch("COMMIT;")?,
            Err(_) => {
                let _ = self.conn().execute_batch("ROLLBACK;");
            }
        }
        result
    }

    fn upsert_indexed_file_inner(&self, file: &IndexedFile<'_>) -> Result<()> {
        // Delete existing chunks + vectors for this file (cascades via FK on vault_chunks).
        // We must delete from vault_chunk_vecs explicitly because it's a virtual table
        // and FK cascades don't reach it.
        let existing_ids: Vec<i64> = {
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM vault_chunks WHERE file_path = ?1")?;
            let ids: std::result::Result<Vec<i64>, _> =
                stmt.query_map([file.path], |row| row.get(0))?.collect();
            ids.map_err(CoreError::Database)?
        };
        for id in &existing_ids {
            self.conn()
                .execute("DELETE FROM vault_chunk_vecs WHERE rowid = ?1", [id])?;
        }

        // Delete the vault_files row — cascades to vault_chunks and vault_people.
        self.conn()
            .execute("DELETE FROM vault_files WHERE path = ?1", [file.path])?;

        // Insert vault_files.
        let frontmatter_json = file
            .parsed
            .frontmatter
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn().execute(
            "INSERT INTO vault_files (path, content_hash, mtime_secs, frontmatter_json, para_folder)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file.path,
                file.parsed.content_hash,
                file.mtime_secs,
                frontmatter_json,
                file.para_folder,
            ],
        )?;

        // Insert chunks + vectors.
        for (idx, (section, embedding)) in file
            .parsed
            .sections
            .iter()
            .zip(file.chunk_embeddings.iter())
            .enumerate()
        {
            self.conn().execute(
                "INSERT INTO vault_chunks (file_path, section_heading, chunk_index, content, token_count, para_folder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    file.path,
                    section.heading,
                    idx as i64,
                    section.content,
                    section.tokens as i64,
                    file.para_folder,
                ],
            )?;
            let chunk_id = self.conn().last_insert_rowid();

            let bytes = f32_slice_to_bytes(embedding);
            self.conn().execute(
                "INSERT INTO vault_chunk_vecs (rowid, embedding) VALUES (?1, ?2)",
                params![chunk_id, bytes],
            )?;
        }

        // Insert person row + addresses if present.
        if let Some(person) = file.person {
            let tags_json = serde_json::to_string(&person.tags)?;
            let fm_json = serde_json::to_string(&person.frontmatter)?;
            self.conn().execute(
                "INSERT INTO vault_people (file_path, name, role, tags_json, last_contact, frontmatter_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    person.file_path,
                    person.name,
                    person.role,
                    tags_json,
                    person.last_contact,
                    fm_json,
                ],
            )?;
            let person_id = self.conn().last_insert_rowid();

            for addr in &person.addresses {
                self.conn().execute(
                    "INSERT OR IGNORE INTO vault_people_addresses (person_id, channel_type, address)
                     VALUES (?1, ?2, ?3)",
                    params![person_id, addr.channel, addr.address],
                )?;
            }
        }

        Ok(())
    }

    /// Returns the content hash currently indexed for `path`, or None if the file isn't indexed.
    /// Callers compare this against the current file hash to decide whether to re-index.
    pub fn indexed_content_hash(&self, path: &str) -> Result<Option<String>> {
        let result: std::result::Result<String, rusqlite::Error> = self.conn().query_row(
            "SELECT content_hash FROM vault_files WHERE path = ?1",
            [path],
            |row| row.get(0),
        );
        match result {
            Ok(h) => Ok(Some(h)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    /// Delete a file's knowledge representation (chunks, vectors, person).
    /// Used when a file is deleted from the vault.
    pub fn delete_indexed_file(&self, path: &str) -> Result<()> {
        let existing_ids: Vec<i64> = {
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM vault_chunks WHERE file_path = ?1")?;
            let ids: std::result::Result<Vec<i64>, _> =
                stmt.query_map([path], |row| row.get(0))?.collect();
            ids.map_err(CoreError::Database)?
        };
        self.conn().execute_batch("BEGIN IMMEDIATE;")?;
        for id in &existing_ids {
            self.conn()
                .execute("DELETE FROM vault_chunk_vecs WHERE rowid = ?1", [id])?;
        }
        self.conn()
            .execute("DELETE FROM vault_files WHERE path = ?1", [path])?;
        self.conn().execute_batch("COMMIT;")?;
        Ok(())
    }

    /// Lookup a vault person by an address on a specific channel.
    /// Returns the person's name and file path, or None.
    pub fn find_vault_person_by_address(
        &self,
        channel_db_str: &str,
        address: &str,
    ) -> Result<Option<(String, String)>> {
        let result: std::result::Result<(String, String), rusqlite::Error> = self.conn().query_row(
            "SELECT vp.name, vp.file_path
             FROM vault_people_addresses vpa
             JOIN vault_people vp ON vp.id = vpa.person_id
             WHERE vpa.channel_type = ?1 AND vpa.address = ?2
             LIMIT 1",
            params![channel_db_str, address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(pair) => Ok(Some(pair)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }
}

/// Convert a slice of f32s into the little-endian byte representation that sqlite-vec expects.
fn f32_slice_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}
