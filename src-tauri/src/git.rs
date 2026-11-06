//! Version control for schematics through the system `git` (arguments passed directly, never
//! through a shell): status, per-file history, file at a revision, commit, init.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A child process without a console window (Windows GUI apps would flash one per call).
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Inside a git work tree? Checked on the file system, so a plain folder never runs `git`
/// (on a Mac without developer tools that would pop up the install dialog).
fn in_repo(file: &Path) -> bool {
    file.ancestors().skip(1).any(|d| d.join(".git").exists())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    // quotePath=false: non-ASCII file names (Verstärker.asc) come back as they are
    // literal pathspecs: `amp[12].asc` means that file, not a glob over amp1/amp2
    let out = command("git").arg("--literal-pathspecs").arg("-C").arg(dir).args(["-c", "core.quotePath=false"]).args(args).output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// (repo root, path of `file` relative to it, with `/`)
fn locate(file: &Path) -> Result<(PathBuf, String), String> {
    let dir = file.parent().ok_or("no folder")?;
    let root = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim());
    let file = std::fs::canonicalize(file).unwrap_or(file.to_path_buf());
    let root_c = std::fs::canonicalize(&root).unwrap_or(root.clone());
    let rel = file.strip_prefix(&root_c).map_err(|_| "file is outside the repository")?;
    Ok((root, rel.to_string_lossy().replace('\\', "/")))
}

#[derive(Serialize, Default)]
pub struct Status {
    /// false: the file's folder is not inside a git repository
    pub repo: bool,
    pub branch: String,
    /// "clean" | "modified" | "untracked" | "added" | "new" (not on disk yet)
    pub file: String,
}

pub fn status(file: &Path) -> Status {
    if !in_repo(file) {
        return Status::default();
    }
    let Ok((root, rel)) = locate(file) else { return Status::default() };
    // symbolic-ref also names the branch of a repository without commits yet
    let branch = git(&root, &["symbolic-ref", "--short", "HEAD"]).or_else(|_| git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])).unwrap_or_default().trim().to_string();
    let st = git(&root, &["status", "--porcelain=v1", "--", &rel]).unwrap_or_default();
    let code = st.lines().next().map(|l| l.get(..2).unwrap_or("").to_string()).unwrap_or_default();
    let file = match code.as_str() {
        "" if file.exists() => "clean",
        "" => "new",
        "??" => "untracked",
        c if c.starts_with('A') => "added",
        _ => "modified",
    };
    Status { repo: true, branch: if branch == "HEAD" { "(detached)".into() } else { branch }, file: file.into() }
}

#[derive(Serialize)]
pub struct Commit {
    pub hash: String,
    pub date: String,
    pub author: String,
    pub subject: String,
}

/// Commits that touched the file (following renames), newest first.
pub fn log(file: &Path, limit: usize) -> Result<Vec<Commit>, String> {
    if !in_repo(file) {
        return Ok(vec![]);
    }
    let (root, rel) = locate(file)?;
    if git(&root, &["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        return Ok(vec![]); // a repository without commits yet
    }
    let out = git(&root, &["log", "--follow", &format!("-n{limit}"), "--date=format:%Y-%m-%d %H:%M", "--format=%H%x1f%ad%x1f%an%x1f%s", "--", &rel])?;
    Ok(out.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\x1f').collect();
        Some(Commit { hash: f.first()?.to_string(), date: f.get(1)?.to_string(), author: f.get(2)?.to_string(), subject: f.get(3)?.to_string() })
    }).collect())
}

/// The file's bytes at a revision (`<hash>:<path>`; the path it had then is found by --follow).
pub fn show(file: &Path, hash: &str) -> Result<Vec<u8>, String> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid revision".into());
    }
    let (root, rel) = locate(file)?;
    // the name it had in that commit (renames)
    let names = git(&root, &["log", "--follow", "--name-only", "--format=%H", "--", &rel])?;
    let mut lines = names.lines().filter(|l| !l.is_empty());
    let mut path = rel.clone();
    while let Some(h) = lines.next() {
        let name = lines.next().unwrap_or(&rel);
        if h == hash {
            path = name.to_string();
            break;
        }
    }
    let out = command("git").arg("-C").arg(&root).args(["-c", "core.quotePath=false", "show", &format!("{hash}:{path}")]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(out.stdout) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Stage and commit just this file.
pub fn commit(file: &Path, message: &str) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("commit message is empty".into());
    }
    let (root, rel) = locate(file)?;
    git(&root, &["add", "--", &rel])?;
    git(&root, &["commit", "-m", message, "--", &rel])?;
    Ok(git(&root, &["rev-parse", "--short", "HEAD"])?.trim().to_string())
}

/// `git init` in the file's folder.
pub fn init(file: &Path) -> Result<(), String> {
    git(file.parent().ok_or("no folder")?, &["init"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_log_show_commit() {
        let dir = std::env::temp_dir().join(format!("nomospice-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Verstärker.asc");
        assert!(!status(&f).repo);
        init(&f).unwrap();
        git(&dir, &["config", "user.email", "t@example.com"]).unwrap();
        git(&dir, &["config", "user.name", "T"]).unwrap();
        std::fs::write(&f, "Version 4\nWIRE 0 0 16 0\n").unwrap();
        assert_eq!(status(&f).file, "untracked");
        commit(&f, "first").unwrap();
        assert_eq!(status(&f).file, "clean");
        std::fs::write(&f, "Version 4\nWIRE 0 0 32 0\n").unwrap();
        assert_eq!(status(&f).file, "modified");
        commit(&f, "second; with `quotes` and $x").unwrap();
        let log = log(&f, 10).unwrap();
        assert_eq!(log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["second; with `quotes` and $x", "first"]);
        assert_eq!(show(&f, &log[1].hash).unwrap(), b"Version 4\nWIRE 0 0 16 0\n");
        assert!(show(&f, "HEAD; rm -rf /").is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
