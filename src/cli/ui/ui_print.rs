// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use tokio::time::Duration;

/// main UI handler for CLI output
/// respects quiet mode and stdout redirection
pub struct UiOutput {
    quiet: bool,
    is_stdout: bool,
    multi_progress: Option<Arc<MultiProgress>>,
}

impl UiOutput {
    pub fn new(quiet: bool, is_stdout: bool) -> Self {
        let multi_progress = if quiet {
            None
        } else {
            Some(Arc::new(MultiProgress::new()))
        };

        Self {
            quiet,
            is_stdout,
            multi_progress,
        }
    }

    /// print to stdout (respects quiet mode)
    pub fn println(&self, msg: impl AsRef<str>) {
        if self.quiet {
            return;
        }

        if let Some(mp) = &self.multi_progress {
            let _ = mp.println(msg.as_ref());
        } else {
            println!("{}", msg.as_ref());
        }
    }

    pub fn println_final(&self, msg: impl AsRef<str>) {
        if self.quiet {
            return;
        }
        println!("{}", msg.as_ref());
    }

    pub fn eprintln_final(&self, msg: impl AsRef<str>) {
        if self.quiet {
            return;
        }
        if self.is_stdout {
            eprintln!("{}", msg.as_ref());
        } else {
            println!("{}", msg.as_ref());
        }
    }

    /// print errors (ignores quiet mode)
    pub fn error(&self, msg: impl AsRef<str>) {
        eprintln!("{}", msg.as_ref());
    }

    pub fn update_spinner(&self, pb: &Option<ProgressBar>, message: impl Into<String>) {
        if let Some(spinner) = pb {
            spinner.set_message(message.into());
        }
    }

    /// finish spinner with message
    pub fn finish_spinner(&self, pb: Option<ProgressBar>, message: impl Into<String>) {
        if let Some(spinner) = pb {
            spinner.finish_with_message(message.into());
        }
    }

    /// create a spinner progress bar
    pub fn create_spinner(&self, message: impl Into<String>) -> Option<ProgressBar> {
        if self.quiet {
            return None;
        }

        self.multi_progress.as_ref().map(|mp| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.blue} {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(Duration::from_millis(300));
            pb.set_message(message.into());
            pb
        })
    }

    /// create a generic progress bar
    fn create_progress_bar_internal(
        &self,
        length: u64,
        message: impl Into<String>,
    ) -> Option<ProgressBar> {
        if self.quiet {
            return None;
        }

        self.multi_progress.as_ref().map(|mp| {
            let pb = mp.add(ProgressBar::new(length));
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/white}] {percent}% - {msg}")
                    .unwrap()
                    .progress_chars("▰▱ "),
            );
            pb.enable_steady_tick(Duration::from_secs(1));
            pb.set_message(message.into());
            pb
        })
    }

    /// create a progress bar wrapper for partition extraction
    pub fn create_extraction_progress(
        &self,
        partition_name: impl Into<String>,
    ) -> ExtractionProgress {
        let pb = self.create_progress_bar_internal(100, partition_name);
        ExtractionProgress { progress_bar: pb }
    }

    /// create a progress bar wrapper for download progress
    #[cfg(feature = "prefetch")]
    pub fn create_download_progress(&self, message: impl Into<String>) -> DownloadProgress {
        let pb = self.create_progress_bar_internal(100, message);
        DownloadProgress { progress_bar: pb }
    }

    /// clear all progress bars
    pub fn clear(&self) -> anyhow::Result<()> {
        if let Some(mp) = &self.multi_progress {
            mp.clear()?;
        }
        Ok(())
    }

    /// print through progress bar to stderr if stdout redirected
    pub fn pb_eprintln(&self, msg: impl AsRef<str>) {
        if self.quiet {
            return;
        }

        if self.is_stdout {
            eprintln!("{}", msg.as_ref());
        } else if let Some(mp) = &self.multi_progress {
            let _ = mp.println(msg.as_ref());
        } else {
            println!("{}", msg.as_ref());
        }
    }
}

/// wrapper for extraction progress bar
pub struct ExtractionProgress {
    progress_bar: Option<ProgressBar>,
}

impl ExtractionProgress {
    pub fn set_message(&self, message: impl Into<String>) {
        if let Some(pb) = &self.progress_bar {
            pb.set_message(message.into());
        }
    }

    pub fn set_position(&self, pos: u64) {
        if let Some(pb) = &self.progress_bar {
            pb.set_position(pos);
        }
    }

    pub fn finish_with_message(&self, message: impl Into<String>) {
        if let Some(pb) = &self.progress_bar {
            pb.finish_with_message(message.into());
        }
    }
}

/// wrapper for download progress bar
#[cfg(feature = "prefetch")]
pub struct DownloadProgress {
    progress_bar: Option<ProgressBar>,
}

#[cfg(feature = "prefetch")]
impl DownloadProgress {
    pub fn set_message(&self, message: impl Into<String>) {
        if let Some(pb) = &self.progress_bar {
            pb.set_message(message.into());
        }
    }

    pub fn set_position(&self, pos: u64) {
        if let Some(pb) = &self.progress_bar {
            pb.set_position(pos);
        }
    }

    pub fn finish_with_message(&self, message: impl Into<String>) {
        if let Some(pb) = &self.progress_bar {
            pb.finish_with_message(message.into());
        }
    }
}

/// helper macros
#[macro_export]
macro_rules! ui_println {
    ($ui:expr, $($arg:tt)*) => {
        $ui.println(format!($($arg)*))
    };
}

#[macro_export]
macro_rules! ui_error {
    ($ui:expr, $($arg:tt)*) => {
        $ui.error(format!($($arg)*))
    };
}
