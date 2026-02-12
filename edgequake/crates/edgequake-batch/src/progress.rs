//! Terminal progress bars for batch pipeline.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Progress tracking for the batch pipeline.
pub struct BatchProgress {
    multi: MultiProgress,
}

impl BatchProgress {
    pub fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
        }
    }

    /// Create a progress bar for document processing.
    pub fn document_bar(&self, total: u64, label: &str) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new(total));
        bar.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "{{spinner:.green}} {} [{{bar:40.cyan/blue}}] {{pos}}/{{len}} ({{eta}})",
                    label
                ))
                .unwrap()
                .progress_chars("#>-"),
        );
        bar
    }

    /// Create a spinner for an indeterminate operation.
    pub fn spinner(&self, message: &str) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new_spinner());
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        bar.set_message(message.to_string());
        bar
    }

    /// Create a progress bar for batch polling.
    pub fn batch_bar(&self, batch_count: u64) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new(batch_count));
        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} Batch jobs [{bar:40.cyan/blue}] {pos}/{len} completed",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        bar
    }
}

impl Default for BatchProgress {
    fn default() -> Self {
        Self::new()
    }
}
