use crate::config::FileNaming;
use std::path::PathBuf;

pub struct OutputManager {
    base_dir: PathBuf,
    file_naming: FileNaming,
}

impl OutputManager {
    pub fn new(base_dir: PathBuf, file_naming: FileNaming) -> Self {
        Self {
            base_dir,
            file_naming,
        }
    }

    /// Ensure the source directory exists, e.g., {base}/friends/{username}/
    pub fn ensure_source_dir(
        &self,
        source_type: &str,
        source_name: &str,
    ) -> std::io::Result<PathBuf> {
        let dir = self.base_dir.join(source_type).join(source_name);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Compute the file path for a post's media file
    pub fn post_path(
        &self,
        source_type: &str,
        source_name: &str,
        post_id: &str,
        title: Option<&str>,
        extension: &str,
    ) -> PathBuf {
        let dir = self.base_dir.join(source_type).join(source_name);
        let filename = match self.file_naming {
            FileNaming::Id => format!("{}.{}", post_id, extension),
            FileNaming::Title => {
                let safe_title = sanitize_filename(title.unwrap_or(post_id));
                format!("{}.{}", safe_title, extension)
            }
            FileNaming::IdTitle => {
                let safe_title = sanitize_filename(title.unwrap_or(""));
                if safe_title.is_empty() {
                    format!("{}.{}", post_id, extension)
                } else {
                    format!("{}-{}.{}", post_id, safe_title, extension)
                }
            }
        };
        dir.join(filename)
    }

    /// Get the path for a gallery subdirectory
    pub fn gallery_dir(&self, source_type: &str, source_name: &str, post_id: &str) -> PathBuf {
        self.base_dir
            .join(source_type)
            .join(source_name)
            .join(format!("{}_gallery", post_id))
    }

    /// Get the database path
    pub fn db_path(&self) -> PathBuf {
        self.base_dir.join("reddit-dl.db")
    }
}

/// Sanitize a string for use as a filename.
/// Replaces invalid chars with underscores and truncates to 100 chars.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(100)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FileNaming;
    use std::path::PathBuf;

    fn make_manager(naming: FileNaming) -> OutputManager {
        OutputManager::new(PathBuf::from("/tmp/reddit-archive"), naming)
    }

    // --- sanitize_filename ---

    #[test]
    fn sanitize_keeps_alphanumeric_and_allowed_chars() {
        assert_eq!(sanitize_filename("hello-world_123"), "hello-world_123");
    }

    #[test]
    fn sanitize_replaces_slash_and_special_chars() {
        let result = sanitize_filename("foo/bar:baz?qux");
        assert_eq!(result, "foo_bar_baz_qux");
    }

    #[test]
    fn sanitize_trims_leading_and_trailing_spaces() {
        assert_eq!(sanitize_filename("  hello  "), "hello");
    }

    #[test]
    fn sanitize_truncates_to_100_chars() {
        let long = "a".repeat(200);
        let result = sanitize_filename(&long);
        assert_eq!(result.len(), 100);
    }

    #[test]
    fn sanitize_empty_string_returns_empty() {
        assert_eq!(sanitize_filename(""), "");
    }

    // --- post_path ---

    #[test]
    fn post_path_id_naming_uses_id() {
        let mgr = make_manager(FileNaming::Id);
        let path = mgr.post_path("subreddit", "rust", "abc123", Some("Cool Post!"), "jpg");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/abc123.jpg")
        );
    }

    #[test]
    fn post_path_title_naming_uses_sanitized_title() {
        let mgr = make_manager(FileNaming::Title);
        let path = mgr.post_path("subreddit", "rust", "abc123", Some("Cool Post!"), "jpg");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/Cool Post_.jpg")
        );
    }

    #[test]
    fn post_path_title_naming_falls_back_to_id_when_no_title() {
        let mgr = make_manager(FileNaming::Title);
        let path = mgr.post_path("subreddit", "rust", "abc123", None, "png");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/abc123.png")
        );
    }

    #[test]
    fn post_path_id_title_naming_combines_both() {
        let mgr = make_manager(FileNaming::IdTitle);
        let path = mgr.post_path("subreddit", "rust", "abc123", Some("Great article"), "mp4");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/abc123-Great article.mp4")
        );
    }

    #[test]
    fn post_path_id_title_falls_back_to_id_when_title_is_empty() {
        let mgr = make_manager(FileNaming::IdTitle);
        let path = mgr.post_path("subreddit", "rust", "abc123", Some(""), "gif");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/abc123.gif")
        );
    }

    #[test]
    fn post_path_id_title_falls_back_to_id_when_no_title() {
        let mgr = make_manager(FileNaming::IdTitle);
        let path = mgr.post_path("subreddit", "rust", "abc123", None, "gif");
        assert_eq!(
            path,
            PathBuf::from("/tmp/reddit-archive/subreddit/rust/abc123.gif")
        );
    }

    // --- ensure_source_dir ---

    #[test]
    fn ensure_source_dir_creates_directory_and_returns_path() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let mgr = OutputManager::new(tmp.path().to_path_buf(), FileNaming::Id);
        let result = mgr.ensure_source_dir("friends", "alice");
        assert!(result.is_ok());
        let dir = result.unwrap();
        assert!(dir.exists());
        assert!(dir.is_dir());
        assert_eq!(dir, tmp.path().join("friends").join("alice"));
    }

    #[test]
    fn ensure_source_dir_is_idempotent() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let mgr = OutputManager::new(tmp.path().to_path_buf(), FileNaming::Id);
        // Call twice — should not error
        mgr.ensure_source_dir("subreddit", "pics").unwrap();
        let result = mgr.ensure_source_dir("subreddit", "pics");
        assert!(result.is_ok());
    }
}
