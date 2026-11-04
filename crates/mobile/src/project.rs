//! Projects for the app: the ones in a folder the user picked, a new one,
//! and a scan put in one, in its `scans/`, its `.c2fproj` updated for the
//! studio on the Mac to find it.

use std::path::{Path, PathBuf};

use c2f_project::{Kind, Project};

/// A project, as the app lists it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectSummary {
    pub name: String,
    /// Its folder.
    pub folder: String,
    pub scans: u32,
}

#[derive(Debug, uniffi::Error)]
pub enum ProjectError {
    Failed { message: String },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ProjectError {}

fn failed(message: impl Into<String>) -> ProjectError {
    ProjectError::Failed { message: message.into() }
}

fn summary(project: &Project) -> ProjectSummary {
    ProjectSummary { name: project.name.clone(), folder: project.dir.to_string_lossy().into_owned(), scans: project.scans.len() as u32 }
}

/// The project `folder` is, if it is one.
#[uniffi::export]
pub fn project_at(folder: String) -> Option<ProjectSummary> {
    let dir = PathBuf::from(folder);
    c2f_project::file_in(&dir).and_then(|file| Project::open(&file).ok()).map(|p| summary(&p))
}

/// The projects in `folder`: itself when it is one, else those of its
/// folders, by name.
#[uniffi::export]
pub fn projects_in(folder: String) -> Vec<ProjectSummary> {
    if let Some(project) = project_at(folder.clone()) {
        return vec![project];
    }
    let mut found: Vec<ProjectSummary> = std::fs::read_dir(&folder)
        .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| project_at(e.path().to_string_lossy().into_owned())).collect())
        .unwrap_or_default();
    found.sort_by_key(|p| p.name.to_lowercase());
    found
}

/// Creates the project `name` in a new folder of `parent`, named after it.
#[uniffi::export]
pub fn create_project(parent: String, name: String) -> Result<ProjectSummary, ProjectError> {
    // The name as a folder's: no separator takes it elsewhere.
    let folder_name: String = name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
    if folder_name.is_empty() || folder_name.starts_with('.') {
        return Err(failed("a project needs a name"));
    }
    let folder = Path::new(&parent).join(folder_name);
    c2f_project::create(&folder, &name).map(|p| summary(&p)).map_err(failed)
}

/// Copies the scan `file` into the `scans/` of the project in `folder`,
/// names it in the project's file, and gives its new path.
#[uniffi::export]
pub fn add_scan(folder: String, file: String) -> Result<String, ProjectError> {
    let dir = PathBuf::from(&folder);
    let project_file = c2f_project::file_in(&dir).ok_or_else(|| failed(format!("{folder} is not a project")))?;
    let mut project = Project::open(&project_file).map_err(|p| failed(p.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))?;
    let source = PathBuf::from(&file);
    let name = source.file_name().ok_or_else(|| failed(format!("{file} names no file")))?;
    let scans = dir.join(Kind::Scan.folder());
    std::fs::create_dir_all(&scans).map_err(|e| failed(format!("cannot create {}: {e}", scans.display())))?;
    let target = free(&scans, Path::new(name));
    std::fs::copy(&source, &target).map_err(|e| failed(format!("cannot copy the scan into {}: {e}", scans.display())))?;
    project.add(Kind::Scan, &target).map_err(failed)?;
    project.save().map_err(failed)?;
    Ok(target.to_string_lossy().into_owned())
}

/// `name` in `dir`, or `name_2`, `name_3`… when it is taken.
fn free(dir: &Path, name: &Path) -> PathBuf {
    let stem = name.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut path = dir.join(name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{n}{extension}"));
        n += 1;
    }
    path
}
