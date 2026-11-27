//! The metrics recorder of this brok process: the workspace it runs in
//! (`brok_core::metrics`), or nothing outside a workspace.

use std::path::Path;
use std::sync::OnceLock;

use brok_core::Workspace;
use brok_core::metrics::Recorder;

static RECORDER: OnceLock<Recorder> = OnceLock::new();

/// Finds the workspace of `cwd` once; later calls keep the first answer.
pub fn init(cwd: &Path) {
    // Every command passes here first, so older workspaces are repaired before
    // anything is written to them.
    if let Ok(workspace) = Workspace::discover(cwd) {
        match brok_core::init::protect_private_files(&workspace.paths().brokspace) {
            Ok(added) if !added.is_empty() => crate::ui::note(&format!(
                "Added {} to .brokspace/.gitignore so private files are not committed (`brok doctor` checks for ones already tracked)",
                added.join(", ")
            )),
            Ok(_) => {}
            Err(e) => crate::ui::warn(&format!("cannot keep private files out of git: {e}")),
        }
    }
    let recorder = Workspace::discover(cwd)
        .map(|workspace| Recorder::new(workspace.paths(), &workspace.config().metrics))
        .unwrap_or_else(|_| Recorder::off());
    let _ = RECORDER.set(recorder);
}

/// The recorder (off until [`init`], and outside a workspace).
pub fn recorder() -> &'static Recorder {
    RECORDER.get_or_init(Recorder::off)
}
