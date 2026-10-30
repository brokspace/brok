//! Terminal output helpers. Colors follow `NO_COLOR` / TTY detection.

use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

/// Prints an error to stderr.
pub fn error(message: &str) {
    eprintln!("{} {message}", style("error:").red().bold());
}

/// Prints a warning to stderr.
pub fn warn(message: &str) {
    eprintln!("{} {message}", style("warning:").yellow().bold());
}

/// Prints a success line.
pub fn success(message: &str) {
    println!("{} {message}", style("✓").green().bold());
}

/// Prints an informational step.
pub fn step(message: &str) {
    println!("{} {message}", style("•").cyan());
}

/// Like [`step`], on stderr: for side notes that are not the command's
/// output (stdout may be piped or parsed as JSON).
pub fn note(message: &str) {
    eprintln!("{} {message}", style("•").cyan());
}

/// Prints a dimmed hint.
pub fn hint(message: &str) {
    println!("  {}", style(message).dim());
}

/// Section header.
pub fn header(title: &str) {
    println!("{}", style(title).bold().underlined());
}

/// Whether stdin and stdout are interactive terminals.
pub fn interactive() -> bool {
    console::user_attended() && console::Term::stdout().is_term()
}

/// A spinner for work of unknown length.
pub fn spinner(message: impl Into<String>) -> ProgressBar {
    let bar = ProgressBar::new_spinner();
    bar.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("valid template"));
    bar.set_message(message.into());
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
}

/// A progress bar counting items.
pub fn counter(total: u64, message: impl Into<String>) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template("{msg:>12} [{bar:30.cyan/blue}] {pos}/{len} ({eta})")
            .expect("valid template")
            .progress_chars("=> "),
    );
    bar.set_message(message.into());
    bar
}

/// A progress bar counting bytes.
pub fn bytes(total: Option<u64>, message: impl Into<String>) -> ProgressBar {
    let bar = ProgressBar::new(total.unwrap_or(0));
    bar.set_style(
        ProgressStyle::with_template("{msg:>24} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec}")
            .expect("valid template")
            .progress_chars("=> "),
    );
    bar.set_message(message.into());
    bar
}
