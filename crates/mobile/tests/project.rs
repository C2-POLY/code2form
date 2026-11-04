//! The projects the app saves scans into, through the API Swift sees.

use c2f_mobile::{add_scan, create_project, project_at, projects_in};

#[test]
fn a_scan_goes_into_a_projects_scans_and_its_file() {
    let root = std::env::temp_dir().join(format!("c2f-mobile-projects-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let parent = root.to_string_lossy().into_owned();
    let made = create_project(parent.clone(), "Grogu".into()).unwrap();
    create_project(parent.clone(), "Lampe".into()).unwrap();
    let names: Vec<String> = projects_in(parent.clone()).into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["Grogu", "Lampe"]);
    // A scan the app wrote in its own folder.
    let scan = root.join("scan-2026-11-04-101500.stl");
    std::fs::write(&scan, "solid s\nendsolid s\n").unwrap();
    let saved = add_scan(made.folder.clone(), scan.to_string_lossy().into_owned()).unwrap();
    assert!(saved.ends_with("Grogu/scans/scan-2026-11-04-101500.stl"), "{saved}");
    let project = project_at(made.folder.clone()).unwrap();
    assert_eq!(project.scans, 1);
    // The studio reads what the app wrote.
    let read = c2f_project::Project::open(std::path::Path::new(&made.folder)).unwrap();
    assert_eq!(read.scans, ["scans/scan-2026-11-04-101500.stl"]);
    assert!(add_scan(parent, scan.to_string_lossy().into_owned()).is_err(), "not a project");
    let _ = std::fs::remove_dir_all(&root);
}
