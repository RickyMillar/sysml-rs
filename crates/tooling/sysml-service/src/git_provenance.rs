//! Git provenance capture for baseline creation (B6, steward ruling
//! 2026-07-16): three read-only reads shelled out to the `git` binary —
//! deliberately NOT a git2/gix dependency (no object-graph traversal
//! needed; runs once per explicit baseline-create, never a hot path).
//!
//! The one home of "read git state for a workspace" — a second consumer
//! (e.g. session-archive provenance) must reuse this, not re-derive the
//! subprocess calls.
//!
//! Honesty split (binding): "not a git repository" is the LEGITIMATE
//! absent case and returns `None` silently; a broken git toolchain (binary
//! missing, command failure inside a real repo) is a genuine error that
//! must not masquerade as "not a repo" — it is logged loudly and also
//! returns `None` (provenance is corroborating metadata, never worth
//! failing the baseline over — the content digest is the identity).

use std::path::Path;
use std::process::Command;

use sysml_store::GitProvenance;

/// One `git -C <root> …` invocation; `Ok(stdout)` on zero exit.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git binary unavailable: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Capture git provenance for `root`, if it is inside a git work tree.
pub(crate) fn capture(root: &Path) -> Option<GitProvenance> {
    match git(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(v) if v == "true" => {}
        Err(e) if e.contains("not a git repository") => return None,
        other => {
            // Bare repo, missing binary, or another real failure — never
            // silently identical to "not a repo".
            let detail = match other {
                Ok(v) => format!("unexpected rev-parse output: {v}"),
                Err(e) => e,
            };
            tracing::warn!(root = %root.display(), detail, "git provenance unavailable");
            return None;
        }
    }

    // HEAD SHA. An unborn HEAD (repo with no commits) has no SHA to
    // record — that is a real gap in a real repo, so warn, then honest
    // absence.
    let sha = match git(root, &["rev-parse", "HEAD"]) {
        Ok(sha) => sha,
        Err(e) => {
            tracing::warn!(root = %root.display(), detail = e, "git repo has no resolvable HEAD — provenance not captured");
            return None;
        }
    };

    // Dirty = any uncommitted change UNDER THE WORKSPACE ROOT (`-- .`
    // scopes to it; the repo may be larger than the workspace).
    let dirty = match git(root, &["status", "--porcelain", "--", "."]) {
        Ok(out) => !out.is_empty(),
        Err(e) => {
            tracing::warn!(root = %root.display(), detail = e, "git status failed — provenance not captured");
            return None;
        }
    };

    // Branch: absent on a detached HEAD (symbolic-ref exits non-zero).
    let branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty());

    Some(GitProvenance { sha, dirty, branch })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn non_repo_is_silent_none_and_repo_states_are_captured() {
        let dir = tempfile::tempdir().unwrap();

        // Plain directory: the legitimate absent case.
        assert!(capture(dir.path()).is_none());

        // Repo with no commits: unborn HEAD — honest absence.
        run(dir.path(), &["init", "-q", "-b", "main"]);
        assert!(capture(dir.path()).is_none());

        // Clean commit on a branch.
        std::fs::write(dir.path().join("m.sysml"), "package P;").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "init"]);
        let p = capture(dir.path()).expect("provenance in a real repo");
        assert_eq!(p.sha.len(), 40);
        assert!(!p.dirty);
        assert_eq!(p.branch.as_deref(), Some("main"));

        // Dirty work tree is recorded, never refused.
        std::fs::write(dir.path().join("m.sysml"), "package Q;").unwrap();
        let p = capture(dir.path()).expect("provenance");
        assert!(p.dirty);

        // Detached HEAD: sha yes, branch honestly absent.
        run(dir.path(), &["checkout", "-q", "--detach"]);
        let p = capture(dir.path()).expect("provenance");
        assert!(p.branch.is_none());
    }
}
