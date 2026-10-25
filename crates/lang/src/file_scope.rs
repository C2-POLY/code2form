//! The folders a program may read files from: when an agent runs a model,
//! its includes, `use`s, imported data files and circuit parts are read
//! only there, so that its text cannot reach the rest of the disk.

use std::path::{Component, Path, PathBuf, Prefix};

/// Folders, as the disk has them (links followed), that the files a
/// program reads must be in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileScope {
    roots: Vec<PathBuf>,
}

impl FileScope {
    /// The files in `roots` and below.
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> FileScope {
        FileScope { roots: roots.into_iter().map(|root| resolve(&root)).collect() }
    }

    /// The folders, resolved.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether `path` (relative to the current directory when relative)
    /// is in one of the folders, once its links are followed.
    pub fn allows(&self, path: &Path) -> bool {
        let full = resolve(path);
        self.roots.iter().any(|root| full.starts_with(root))
    }

    /// What a program reads when a file it names is refused; it names the
    /// file as the program wrote it, and never quotes it.
    pub fn refusal(name: &str) -> String {
        format!("'{name}' is outside the folders the agent may read (its project folder and the libraries): not read")
    }
}

/// `path` without `.` and with each `..` taking off the component before
/// it, without reading the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` as the disk has it: absolute, links followed for the part that
/// exists, the rest as written. On Windows a path on a drive is written
/// `C:\...`, as the user writes it, never `\\?\C:\...` as
/// `canonicalize` gives it, so that every path it returns compares (and
/// reads) the same way.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = match path.is_relative() {
        true => std::env::current_dir().unwrap_or_default().join(path),
        false => path.to_path_buf(),
    };
    let path = normalize(&absolute);
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            return rest.iter().rev().fold(on_drive(real), |full, part| full.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return path,
        }
    }
}

/// `\\?\C:\a\b` as `C:\a\b`; any other path as it is.
fn on_drive(path: PathBuf) -> PathBuf {
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Prefix(prefix)), Some(Component::RootDir)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => {
                let mut plain = PathBuf::from(format!("{}:\\", char::from(drive)));
                plain.extend(components);
                plain
            }
            _ => path,
        },
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_in_the_folders_are_allowed() {
        let root = std::env::temp_dir().join(format!("c2f-file-scope-{}", std::process::id()));
        std::fs::create_dir_all(root.join("project/sub")).unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();
        let scope = FileScope::new([root.join("project")]);
        assert!(scope.allows(&root.join("project/sub/new.scad")), "a file yet to be made");
        assert!(scope.allows(&root.join("project/sub/../a.scad")));
        assert!(!scope.allows(&root.join("project/../secret.txt")));
        assert!(!scope.allows(&root.join("secret.txt")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("secret.txt"), root.join("project/link.txt")).unwrap();
            assert!(!scope.allows(&root.join("project/link.txt")), "a link is followed");
        }
        assert!(!FileScope::refusal("/etc/passwd").contains("root:"));
        #[cfg(windows)]
        {
            let resolved = resolve(&root.join("project"));
            assert!(!resolved.to_string_lossy().starts_with(r"\\?\"), "{}", resolved.display());
            let verbatim = root.join("project").canonicalize().unwrap();
            assert!(scope.allows(&verbatim.join("sub/x.scad")), "a verbatim path, the same folder");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
