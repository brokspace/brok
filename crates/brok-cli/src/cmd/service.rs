//! `brok start`, `brok stop`, `brok restart`, `brok logs`: the background
//! daemon, so nobody has to keep a terminal open for `brok run`.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::{daemon, ui};

/// Arguments of `brok logs`.
#[derive(Debug, clap::Args)]
pub struct LogsArgs {
    /// Keep printing new lines (Ctrl+C to stop).
    #[arg(short, long)]
    follow: bool,
    /// Number of lines to show.
    #[arg(short = 'n', long, default_value_t = 40)]
    lines: usize,
}

pub async fn start(cwd: &Path) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    if daemon::is_running(&workspace).await {
        ui::success("brok is already running");
        return Ok(());
    }
    let spinner = ui::spinner("Starting brok (bus, index, agent)");
    let pid = daemon::start(&workspace).await;
    spinner.finish_and_clear();
    let pid = pid?;
    ui::success(&format!("brok is running in the background (pid {pid})"));
    ui::hint(&format!("logs: brok logs -f   ({})", daemon::log_file(&workspace).display()));
    ui::hint("stop: brok stop");
    Ok(())
}

pub async fn stop(cwd: &Path) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    if daemon::stop(&workspace).await? {
        ui::success("brok stopped");
    } else {
        ui::step("brok is not running");
    }
    Ok(())
}

pub async fn restart(cwd: &Path) -> Result<()> {
    stop(cwd).await?;
    start(cwd).await
}

pub async fn logs(cwd: &Path, args: LogsArgs) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let path = daemon::log_file(&workspace);
    let Ok(mut file) = std::fs::File::open(&path) else {
        ui::step(&format!("no logs yet ({} does not exist); start with `brok start`", path.display()));
        return Ok(());
    };
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines[lines.len().saturating_sub(args.lines)..] {
        println!("{line}");
    }
    if !args.follow {
        return Ok(());
    }
    let mut position = file.seek(SeekFrom::End(0))?;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            () = tokio::time::sleep(Duration::from_millis(300)) => {}
        }
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(position);
        if len < position {
            position = 0; // truncated or rotated
        }
        if len > position {
            file.seek(SeekFrom::Start(position))?;
            let mut chunk = String::new();
            file.read_to_string(&mut chunk)?;
            print!("{chunk}");
            position = len;
        }
    }
}
