use rusqlite::params;
use std::sync::Arc;

use crate::error::{CoreError, Result};
use crate::knowledge::Embedder;
use crate::store::Store;

/// A retrieved chunk with similarity score and provenance.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub file_path: String,
    pub section_heading: Option<String>,
    pub content: String,
    pub para_folder: Option<String>,
    /// L2 distance — lower is more similar.
    pub distance: f32,
}

/// Optional filters for retrieval.
#[derive(Debug, Clone, Default)]
pub struct RetrievalFilters {
    /// Restrict to specific PARA folders (e.g. ["05-People"]).
    pub para_folders: Option<Vec<String>>,
    /// Maximum chunks to return (default 5 if None).
    pub top_k: Option<usize>,
}

pub struct Retriever {
    embedder: Arc<Embedder>,
}

impl Retriever {
    pub fn new(embedder: Arc<Embedder>) -> Self {
        Self { embedder }
    }

    /// Semantic search over indexed vault chunks.
    pub fn search(
        &self,
        store: &Store,
        query: &str,
        filters: &RetrievalFilters,
    ) -> Result<Vec<RetrievedChunk>> {
        let query_vec = self.embedder.embed_query(query)?;
        let query_bytes = f32_slice_to_bytes(&query_vec);
        let top_k = filters.top_k.unwrap_or(5);

        // We fetch top_k * 4 from the vec index then filter by para_folder,
        // which is a reasonable tradeoff (sqlite-vec's MATCH doesn't support
        // our external filter). If the user has few chunks in the filtered
        // folder, `top_k * 4` will still return enough; if they have many,
        // the caller can always raise top_k.
        let over_fetch = top_k.saturating_mul(4).max(20);

        let para_filter_clause = filters
            .para_folders
            .as_ref()
            .map(|_| " AND vc.para_folder IN (SELECT value FROM json_each(?3))")
            .unwrap_or("");

        let sql = format!(
            "SELECT vc.file_path, vc.section_heading, vc.content, vc.para_folder, v.distance
             FROM vault_chunk_vecs v
             JOIN vault_chunks vc ON vc.id = v.rowid
             WHERE v.embedding MATCH ?1 AND k = ?2{}
             ORDER BY v.distance",
            para_filter_clause
        );

        let mut stmt = store.conn().prepare(&sql)?;
        let rows: Vec<RetrievedChunk> = match &filters.para_folders {
            Some(folders) => {
                let folders_json = serde_json::to_string(folders)?;
                let result: std::result::Result<Vec<RetrievedChunk>, rusqlite::Error> = stmt
                    .query_map(
                        params![query_bytes, over_fetch as i64, folders_json],
                        row_to_chunk,
                    )?
                    .collect();
                result.map_err(CoreError::Database)?
            }
            None => {
                let result: std::result::Result<Vec<RetrievedChunk>, rusqlite::Error> = stmt
                    .query_map(params![query_bytes, over_fetch as i64], row_to_chunk)?
                    .collect();
                result.map_err(CoreError::Database)?
            }
        };

        Ok(rows.into_iter().take(top_k).collect())
    }
}

fn row_to_chunk(row: &rusqlite::Row) -> std::result::Result<RetrievedChunk, rusqlite::Error> {
    Ok(RetrievedChunk {
        file_path: row.get(0)?,
        section_heading: row.get(1)?,
        content: row.get(2)?,
        para_folder: row.get(3)?,
        distance: row.get::<_, f64>(4)? as f32,
    })
}

fn f32_slice_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::parser::{ParsedFile, Section};
    use crate::store::knowledge::IndexedFile;
    use rusqlite::params;

    #[test]
    fn test_f32_slice_to_bytes_little_endian() {
        // 1.0_f32 in little-endian IEEE 754 is 00 00 80 3F.
        let bytes = f32_slice_to_bytes(&[1.0_f32]);
        assert_eq!(bytes, vec![0x00, 0x00, 0x80, 0x3F]);
    }

    #[test]
    fn test_f32_slice_to_bytes_multiple() {
        let input = [1.0_f32, -1.0_f32, 0.0_f32];
        let bytes = f32_slice_to_bytes(&input);
        assert_eq!(bytes.len(), input.len() * 4);
        // Round-trip: reconstruct f32s from the bytes and verify.
        let round_trip: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(round_trip, input);
    }

    #[test]
    fn test_retrieval_filters_default() {
        let f = RetrievalFilters::default();
        assert!(f.para_folders.is_none());
        assert!(f.top_k.is_none());
    }

    /// Smoke test: insert hand-crafted vectors (no embedder needed) and run
    /// the exact retrieval SQL to verify sqlite-vec MATCH + k= + JOIN work,
    /// and that the optional `json_each` para-folder filter works.
    #[test]
    fn test_retrieval_sql_with_handcrafted_vectors() {
        let store = Store::open_in_memory().unwrap();

        // Two 384-dim vectors. vec_near is near the origin; vec_far is far from it.
        let vec_near: Vec<f32> = (0..384).map(|i| (i as f32) * 0.001).collect();
        let vec_far: Vec<f32> = (0..384).map(|i| 5.0 + (i as f32) * 0.1).collect();

        // Build a ParsedFile with exactly two sections (one per embedding).
        let parsed = ParsedFile {
            frontmatter: None,
            sections: vec![
                Section {
                    heading: Some("Role".to_string()),
                    content: "close content".to_string(),
                    level: 2,
                    tokens: 2,
                },
                Section {
                    heading: Some("About".to_string()),
                    content: "far content".to_string(),
                    level: 2,
                    tokens: 2,
                },
            ],
            content_hash: "h1".to_string(),
            total_tokens: 4,
        };
        let embeddings = vec![vec_near.clone(), vec_far];
        let file = IndexedFile {
            path: "05-People/Alice.md",
            mtime_secs: 0,
            para_folder: Some("05-People"),
            parsed: &parsed,
            chunk_embeddings: &embeddings,
            person: None,
        };
        store.upsert_indexed_file(&file).unwrap();

        // Also index a chunk in a different PARA folder so filter testing is meaningful.
        let parsed2 = ParsedFile {
            frontmatter: None,
            sections: vec![Section {
                heading: Some("Notes".to_string()),
                content: "project chunk".to_string(),
                level: 2,
                tokens: 2,
            }],
            content_hash: "h2".to_string(),
            total_tokens: 2,
        };
        let embeddings2 = vec![vec_near.clone()];
        let file2 = IndexedFile {
            path: "01-Projects/Stuff.md",
            mtime_secs: 0,
            para_folder: Some("01-Projects"),
            parsed: &parsed2,
            chunk_embeddings: &embeddings2,
            person: None,
        };
        store.upsert_indexed_file(&file2).unwrap();

        // --- Path 1: no folder filter. Query with vec_near; nearest chunk wins. ---
        let query_bytes = f32_slice_to_bytes(&vec_near);
        let sql_all = "SELECT vc.file_path, vc.section_heading, vc.content, vc.para_folder, v.distance
                       FROM vault_chunk_vecs v
                       JOIN vault_chunks vc ON vc.id = v.rowid
                       WHERE v.embedding MATCH ?1 AND k = ?2
                       ORDER BY v.distance";
        let mut stmt = store.conn().prepare(sql_all).unwrap();
        let rows: Vec<RetrievedChunk> = stmt
            .query_map(params![query_bytes, 10i64], row_to_chunk)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(rows.len(), 3, "expected 3 chunks indexed, got {}", rows.len());
        // Distances must be monotonically non-decreasing.
        for pair in rows.windows(2) {
            assert!(
                pair[0].distance <= pair[1].distance,
                "distances not sorted: {} > {}",
                pair[0].distance,
                pair[1].distance
            );
        }
        // The 'far' chunk must be last (highest distance).
        assert!(rows.last().unwrap().content.contains("far"));

        // --- Path 2: with json_each() folder filter to 05-People only. ---
        let sql_filtered = "SELECT vc.file_path, vc.section_heading, vc.content, vc.para_folder, v.distance
                            FROM vault_chunk_vecs v
                            JOIN vault_chunks vc ON vc.id = v.rowid
                            WHERE v.embedding MATCH ?1 AND k = ?2
                              AND vc.para_folder IN (SELECT value FROM json_each(?3))
                            ORDER BY v.distance";
        let folders_json = serde_json::to_string(&vec!["05-People".to_string()]).unwrap();
        let mut stmt2 = store.conn().prepare(sql_filtered).unwrap();
        let filtered: Vec<RetrievedChunk> = stmt2
            .query_map(params![query_bytes, 10i64, folders_json], row_to_chunk)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!filtered.is_empty());
        for r in &filtered {
            assert_eq!(r.para_folder.as_deref(), Some("05-People"));
        }
    }
}
