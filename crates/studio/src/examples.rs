//! The examples that come with the app, for the Examples menu: a folder
//! for each subject (`game`, `grogu`), its designs and circuits in it.
//!
//! An example opens as a copy in `~/Documents/Code2Form/Examples`, its
//! whole folder copied, so that the files it names (the circuit an
//! enclosure reads) come along, and so that it can be changed and saved
//! without touching the app's own files. A copy already there opens as it
//! is, with the changes made to it.

use std::path::{Path, PathBuf};

/// A design or circuit of the examples.
#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    /// The folder it is in, as the menu shows it: `Game`.
    pub subject: String,
    pub file_name: String,
    /// The first sentence of its opening comment.
    pub summary: String,
    /// Where it is among the app's examples.
    pub path: PathBuf,
}

/// The app's examples: in the installed app's resources, or the
/// repository's `examples` folder when it runs where it was built.
pub fn source_dir() -> Option<PathBuf> {
    let bundled = c2f_eval::app_resources().map(|resources| resources.join("examples"));
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    bundled.filter(|dir| dir.is_dir()).or_else(|| repository.is_dir().then_some(repository))
}

/// Where examples are copied to before they open.
pub fn user_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Documents/Code2Form/Examples"))
}

/// The examples in `dir`: its subfolders by name, the designs and circuits
/// in each by name.
pub fn list(dir: &Path) -> Vec<Example> {
    let mut examples = Vec::new();
    for folder in sorted(dir).into_iter().filter(|path| path.is_dir()) {
        let Some(name) = folder.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let subject = capitalized(&name);
        for path in sorted(&folder) {
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let opens = path.extension().is_some_and(|e| e == "c2f" || e == "c2fc" || e == "scad");
            if !opens {
                continue;
            }
            let summary = std::fs::read_to_string(&path).map(|text| summary(&text)).unwrap_or_default();
            examples.push(Example { subject: subject.clone(), file_name, summary, path });
        }
    }
    examples
}

/// Copies the folder of `example`, from `source` (the app's examples) to
/// `user` (the user's), leaving the files already there as they are, and
/// returns where the copy of the example is.
pub fn copy_out(example: &Example, source: &Path, user: &Path) -> std::io::Result<PathBuf> {
    let relative = example.path.strip_prefix(source).map_err(std::io::Error::other)?;
    let folder = relative.parent().unwrap_or(Path::new(""));
    copy_missing(&source.join(folder), &user.join(folder))?;
    Ok(user.join(relative))
}

fn copy_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn sorted(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> =
        std::fs::read_dir(dir).map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect()).unwrap_or_default();
    paths.sort();
    paths
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The first sentence of the comment a file opens with (`//` in a design,
/// `#` in a circuit), its lines joined.
fn summary(text: &str) -> String {
    let mut comment = String::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(words) = line.strip_prefix("//").or_else(|| line.strip_prefix('#')) else { break };
        let words = words.trim();
        if words.is_empty() {
            break;
        }
        if !comment.is_empty() {
            comment.push(' ');
        }
        comment.push_str(words);
    }
    match comment.find(". ") {
        Some(end) => comment[..=end].to_string(),
        None => comment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("c2f-examples-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_summary_is_the_first_sentence_of_the_opening_comment() {
        assert_eq!(summary("// A box: with a lid. It opens.\n// Then more.\ncube(1);"), "A box: with a lid.");
        assert_eq!(summary("# A circuit\n#\n# More.\n[parts]"), "A circuit");
        assert_eq!(summary("cube(1); // not this"), "");
    }

    #[test]
    fn the_repositorys_examples_are_listed_by_subject() {
        let examples = list(&source_dir().expect("the repository's examples"));
        let names: Vec<(&str, &str)> = examples.iter().map(|e| (e.subject.as_str(), e.file_name.as_str())).collect();
        for expected in [("Game", "chest.c2f"), ("Game", "gears.c2f"), ("Game", "walker.c2f"), ("Grogu", "grogu.c2fc"), ("Robot", "arm.c2f")] {
            assert!(names.contains(&expected), "{expected:?} in {names:?}");
        }
        assert!(examples.iter().all(|e| !e.summary.is_empty()), "{examples:?}");
    }

    #[test]
    fn an_example_opens_as_a_copy_that_keeps_its_changes() {
        let source = temp("source");
        std::fs::create_dir_all(source.join("box")).unwrap();
        std::fs::write(source.join("box/lid.c2f"), "// A lid.\ncube(1);").unwrap();
        std::fs::write(source.join("box/box.c2fc"), "# Its circuit.").unwrap();
        std::fs::write(source.join("box/notes.txt"), "kept too").unwrap();
        let user = temp("user");
        let example = list(&source).into_iter().find(|e| e.file_name == "lid.c2f").unwrap();
        assert_eq!((example.subject.as_str(), example.summary.as_str()), ("Box", "A lid."));

        let copy = copy_out(&example, &source, &user).unwrap();
        assert_eq!(copy, user.join("box/lid.c2f"));
        // The folder comes along: what the example names is next to it.
        assert!(user.join("box/box.c2fc").exists() && user.join("box/notes.txt").exists());
        // Changed, then opened again: the change stays.
        std::fs::write(&copy, "cube(2);").unwrap();
        copy_out(&example, &source, &user).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "cube(2);");
        let _ = std::fs::remove_dir_all(&source);
        let _ = std::fs::remove_dir_all(&user);
    }
}
