use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use std::path::Path;

#[allow(dead_code)]
pub struct Cursor {
    pub last_post_id: String,
    pub last_post_utc: i64,
}

pub struct Stats {
    pub total_posts: u64,
    pub posts_by_source: Vec<(String, String, u64)>, // (source_type, source_name, count)
}

/// Parameters for [`Database::record_post`].
pub struct RecordPost<'a> {
    pub post_id: &'a str,
    pub source_type: &'a str,
    pub source_name: &'a str,
    pub title: &'a str,
    pub author: &'a str,
    pub permalink: &'a str,
    pub created_utc: f64,
    pub media_count: u32,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| Error::Db(e.to_string()))?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS posts (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_name TEXT NOT NULL,
                title TEXT,
                author TEXT,
                permalink TEXT,
                created_utc INTEGER,
                downloaded_at INTEGER NOT NULL,
                media_count INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS cursors (
                source_type TEXT NOT NULL,
                source_name TEXT NOT NULL,
                last_post_id TEXT NOT NULL,
                last_post_utc INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (source_type, source_name)
            );
        ",
            )
            .map_err(|e| Error::Db(e.to_string()))?;
        Ok(())
    }

    /// Check if a post has already been downloaded.
    pub fn is_downloaded(&self, post_id: &str) -> Result<bool> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM posts WHERE id = ?1",
                params![post_id],
                |row| row.get(0),
            )
            .map_err(|e| Error::Db(e.to_string()))?;
        Ok(count > 0)
    }

    /// Record a downloaded post. Uses INSERT OR IGNORE so duplicate calls are no-ops.
    pub fn record_post(&self, p: RecordPost<'_>) -> Result<()> {
        let downloaded_at = chrono::Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO posts
                    (id, source_type, source_name, title, author, permalink,
                     created_utc, downloaded_at, media_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    p.post_id,
                    p.source_type,
                    p.source_name,
                    p.title,
                    p.author,
                    p.permalink,
                    p.created_utc as i64,
                    downloaded_at,
                    p.media_count,
                ],
            )
            .map_err(|e| Error::Db(e.to_string()))?;
        Ok(())
    }

    /// Get the saved cursor for a source, if any.
    pub fn get_cursor(&self, source_type: &str, source_name: &str) -> Result<Option<Cursor>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT last_post_id, last_post_utc FROM cursors
                 WHERE source_type = ?1 AND source_name = ?2",
            )
            .map_err(|e| Error::Db(e.to_string()))?;

        let mut rows = stmt
            .query(params![source_type, source_name])
            .map_err(|e| Error::Db(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| Error::Db(e.to_string()))? {
            let last_post_id: String = row.get(0).map_err(|e| Error::Db(e.to_string()))?;
            let last_post_utc: i64 = row.get(1).map_err(|e| Error::Db(e.to_string()))?;
            Ok(Some(Cursor {
                last_post_id,
                last_post_utc,
            }))
        } else {
            Ok(None)
        }
    }

    /// Update (upsert) the cursor for a source after a successful sync.
    pub fn update_cursor(
        &self,
        source_type: &str,
        source_name: &str,
        last_post_id: &str,
        last_post_utc: i64,
    ) -> Result<()> {
        let updated_at = chrono::Utc::now().timestamp();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO cursors
                    (source_type, source_name, last_post_id, last_post_utc, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    source_type,
                    source_name,
                    last_post_id,
                    last_post_utc,
                    updated_at
                ],
            )
            .map_err(|e| Error::Db(e.to_string()))?;
        Ok(())
    }

    /// Get aggregate stats for the status command.
    pub fn get_stats(&self) -> Result<Stats> {
        let total_posts: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get::<_, i64>(0))
            .map_err(|e| Error::Db(e.to_string()))? as u64;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_type, source_name, COUNT(*) as cnt
                 FROM posts
                 GROUP BY source_type, source_name
                 ORDER BY source_type, source_name",
            )
            .map_err(|e| Error::Db(e.to_string()))?;

        let posts_by_source = stmt
            .query_map([], |row| {
                let source_type: String = row.get(0)?;
                let source_name: String = row.get(1)?;
                let count: i64 = row.get(2)?;
                Ok((source_type, source_name, count as u64))
            })
            .map_err(|e| Error::Db(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Db(e.to_string()))?;

        Ok(Stats {
            total_posts,
            posts_by_source,
        })
    }

    /// Get all cursors for display. Returns (source_type, source_name, cursor, updated_at).
    pub fn get_all_cursors(&self) -> Result<Vec<(String, String, Cursor, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_type, source_name, last_post_id, last_post_utc, updated_at
                 FROM cursors
                 ORDER BY source_type, source_name",
            )
            .map_err(|e| Error::Db(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let source_type: String = row.get(0)?;
                let source_name: String = row.get(1)?;
                let last_post_id: String = row.get(2)?;
                let last_post_utc: i64 = row.get(3)?;
                let updated_at: i64 = row.get(4)?;
                Ok((
                    source_type,
                    source_name,
                    Cursor {
                        last_post_id,
                        last_post_utc,
                    },
                    updated_at,
                ))
            })
            .map_err(|e| Error::Db(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Db(e.to_string()))?;

        Ok(rows)
    }

    /// Begin a transaction (for atomic batch operations).
    pub fn begin_transaction(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN TRANSACTION")
            .map_err(|e| Error::Db(e.to_string()))
    }

    /// Commit a transaction.
    pub fn commit(&self) -> Result<()> {
        self.conn
            .execute_batch("COMMIT")
            .map_err(|e| Error::Db(e.to_string()))
    }

    /// Rollback a transaction.
    #[allow(dead_code)]
    pub fn rollback(&self) -> Result<()> {
        self.conn
            .execute_batch("ROLLBACK")
            .map_err(|e| Error::Db(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open_temp_db() -> Database {
        let dir = tempdir().unwrap();
        // Keep dir alive by leaking it for the test — fine in unit tests
        let path = dir.keep().join("test.db");
        Database::open(&path).unwrap()
    }

    #[test]
    fn test_is_downloaded_returns_false_for_unknown_post() {
        let db = open_temp_db();
        assert!(!db.is_downloaded("t3_abc123").unwrap());
    }

    #[test]
    fn test_record_and_check_downloaded() {
        let db = open_temp_db();
        db.record_post(RecordPost {
            post_id: "t3_abc123",
            source_type: "subreddit",
            source_name: "rust",
            title: "Cool post",
            author: "ferris",
            permalink: "/r/rust/comments/abc123",
            created_utc: 1_700_000_000.0,
            media_count: 1,
        })
        .unwrap();
        assert!(db.is_downloaded("t3_abc123").unwrap());
    }

    #[test]
    fn test_record_post_is_idempotent() {
        let db = open_temp_db();
        for _ in 0..3 {
            db.record_post(RecordPost {
                post_id: "t3_dup",
                source_type: "subreddit",
                source_name: "pics",
                title: "Duplicate",
                author: "user",
                permalink: "/r/pics/dup",
                created_utc: 1_700_000_001.0,
                media_count: 0,
            })
            .unwrap();
        }
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.total_posts, 1);
    }

    #[test]
    fn test_cursor_roundtrip() {
        let db = open_temp_db();
        assert!(db.get_cursor("friends", "alice").unwrap().is_none());

        db.update_cursor("friends", "alice", "t3_xyz", 1_700_000_500)
            .unwrap();

        let cursor = db.get_cursor("friends", "alice").unwrap().unwrap();
        assert_eq!(cursor.last_post_id, "t3_xyz");
        assert_eq!(cursor.last_post_utc, 1_700_000_500);
    }

    #[test]
    fn test_cursor_upsert_updates_existing() {
        let db = open_temp_db();
        db.update_cursor("subreddit", "rust", "t3_old", 1_000)
            .unwrap();
        db.update_cursor("subreddit", "rust", "t3_new", 2_000)
            .unwrap();

        let cursor = db.get_cursor("subreddit", "rust").unwrap().unwrap();
        assert_eq!(cursor.last_post_id, "t3_new");
        assert_eq!(cursor.last_post_utc, 2_000);
    }

    #[test]
    fn test_get_stats_aggregates_by_source() {
        let db = open_temp_db();
        db.record_post(RecordPost {
            post_id: "p1",
            source_type: "subreddit",
            source_name: "rust",
            title: "T",
            author: "u",
            permalink: "/",
            created_utc: 0.0,
            media_count: 1,
        })
        .unwrap();
        db.record_post(RecordPost {
            post_id: "p2",
            source_type: "subreddit",
            source_name: "rust",
            title: "T",
            author: "u",
            permalink: "/",
            created_utc: 0.0,
            media_count: 1,
        })
        .unwrap();
        db.record_post(RecordPost {
            post_id: "p3",
            source_type: "friends",
            source_name: "alice",
            title: "T",
            author: "u",
            permalink: "/",
            created_utc: 0.0,
            media_count: 1,
        })
        .unwrap();

        let stats = db.get_stats().unwrap();
        assert_eq!(stats.total_posts, 3);
        assert_eq!(stats.posts_by_source.len(), 2);

        let rust_row = stats
            .posts_by_source
            .iter()
            .find(|(st, sn, _)| st == "subreddit" && sn == "rust")
            .unwrap();
        assert_eq!(rust_row.2, 2);

        let alice_row = stats
            .posts_by_source
            .iter()
            .find(|(st, sn, _)| st == "friends" && sn == "alice")
            .unwrap();
        assert_eq!(alice_row.2, 1);
    }

    #[test]
    fn test_get_all_cursors() {
        let db = open_temp_db();
        db.update_cursor("friends", "alice", "t3_a", 100).unwrap();
        db.update_cursor("subreddit", "rust", "t3_b", 200).unwrap();

        let cursors = db.get_all_cursors().unwrap();
        assert_eq!(cursors.len(), 2);
    }

    #[test]
    fn test_transaction_commit() {
        let db = open_temp_db();
        db.begin_transaction().unwrap();
        db.record_post(RecordPost {
            post_id: "tx1",
            source_type: "subreddit",
            source_name: "test",
            title: "T",
            author: "u",
            permalink: "/",
            created_utc: 0.0,
            media_count: 0,
        })
        .unwrap();
        db.commit().unwrap();
        assert!(db.is_downloaded("tx1").unwrap());
    }

    #[test]
    fn test_transaction_rollback() {
        let db = open_temp_db();
        db.begin_transaction().unwrap();
        db.record_post(RecordPost {
            post_id: "tx2",
            source_type: "subreddit",
            source_name: "test",
            title: "T",
            author: "u",
            permalink: "/",
            created_utc: 0.0,
            media_count: 0,
        })
        .unwrap();
        db.rollback().unwrap();
        assert!(!db.is_downloaded("tx2").unwrap());
    }
}
