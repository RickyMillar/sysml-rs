//! Generic walk-up-the-directory-tree helper.
//!
//! Used by `discovery::find_manifest` / `find_workspace` here, and by
//! `sysml-project::discover` for `.workspace.json` / `.project.json`
//! lookups. The single abstraction keeps both call sites honest about
//! termination (`pop` until `false`) and avoids drift between two
//! near-identical loops.

use std::path::Path;

/// Walk up from `start`, invoking `probe` on each directory.
///
/// Returns the first `Some(r)` produced by `probe`, or `None` when the
/// filesystem root is exhausted. `start` itself is probed first.
///
/// `probe` is passed a borrowed path — implementors can join filenames
/// onto it and call `.exists()` / `.is_file()` / parse a manifest, then
/// short-circuit by returning `Some(r)`.
pub fn walk_up<F, R>(start: &Path, mut probe: F) -> Option<R>
where
    F: FnMut(&Path) -> Option<R>,
{
    let mut current = start.to_path_buf();
    loop {
        if let Some(found) = probe(&current) {
            return Some(found);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn returns_first_match() {
        let dir = std::env::temp_dir().join(format!(
            "sysml-manifest-walkup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = dir.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.join("marker"), "").unwrap();

        let found = walk_up(&nested, |p| {
            let candidate = p.join("marker");
            if candidate.is_file() {
                Some(p.to_path_buf())
            } else {
                None
            }
        });

        assert_eq!(found.as_deref(), Some(dir.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_at_root() {
        // Probe always returns None; walk_up must terminate.
        let result: Option<()> = walk_up(Path::new("/"), |_| None);
        assert!(result.is_none());
    }
}
