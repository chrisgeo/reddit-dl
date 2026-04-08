use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct ProgressTracker {
    multi: MultiProgress,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
        }
    }

    /// Create a spinner for the fetch phase (total posts unknown).
    /// Format: "[friends/alice] ⣾ Fetching posts..."
    pub fn add_fetch_spinner(&self, source_type: &str, source_name: &str) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new_spinner());
        let style = ProgressStyle::with_template(
            "{prefix} {spinner} {msg}",
        )
        .unwrap()
        .tick_chars("⣾⣽⣻⢿⡿⣟⣯⣷");

        pb.set_style(style);
        pb.set_prefix(format!("[{}/{}]", source_type, source_name));
        pb.set_message("Fetching posts...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb
    }

    /// Create a progress bar for the download phase (total is known).
    /// Format: "[friends/alice] ████░░░░ 15/50 posts (3 downloaded, 12 skipped)"
    pub fn add_source_bar(
        &self,
        source_type: &str,
        source_name: &str,
        total: u64,
    ) -> ProgressBar {
        let pb = self.multi.add(ProgressBar::new(total));
        let style = ProgressStyle::with_template(
            "{prefix} {bar:30.cyan/blue} {pos}/{len} posts ({msg})",
        )
        .unwrap()
        .progress_chars("█▓░");

        pb.set_style(style);
        pb.set_prefix(format!("[{}/{}]", source_type, source_name));
        pb.set_message("0 downloaded, 0 skipped");
        pb
    }
}
