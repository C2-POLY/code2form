//! The bookkeeping of the tabs, apart from what they hold: their order and
//! which one is active, which tab a render reply belongs to, and the open
//! tabs kept between launches.
//!
//! The active tab lives outside the list, in the app, so that the code
//! working on it reaches its fields directly; the list holds the others,
//! in order, and knows where the active one sits among them.

use std::path::{Path, PathBuf};

/// The tabs other than the active one, in order around it.
#[derive(Debug)]
pub struct Shelf<T> {
    others: Vec<T>,
    /// Where the active tab sits, from 0.
    active: usize,
}

impl<T> Default for Shelf<T> {
    fn default() -> Self {
        Shelf { others: Vec::new(), active: 0 }
    }
}

impl<T> Shelf<T> {
    /// How many tabs there are, the active one included.
    pub fn len(&self) -> usize {
        self.others.len() + 1
    }

    pub fn active(&self) -> usize {
        self.active
    }

    /// The position in `others` of tab `index`, which is not the active one.
    fn slot(&self, index: usize) -> usize {
        if index < self.active { index } else { index - 1 }
    }

    /// The tabs in order, `current` being the active one.
    pub fn iter<'a>(&'a self, current: &'a T) -> impl Iterator<Item = &'a T> {
        let (before, after) = self.others.split_at(self.active);
        before.iter().chain(std::iter::once(current)).chain(after)
    }

    /// Tab `index`, when it is not the active one.
    pub fn other_mut(&mut self, index: usize) -> Option<&mut T> {
        if index == self.active || index >= self.len() {
            return None;
        }
        let slot = self.slot(index);
        self.others.get_mut(slot)
    }

    /// Makes tab `index` the active one: it moves into `current`, and the
    /// tab that was there goes back to its place.
    pub fn select(&mut self, current: &mut T, index: usize) {
        if index == self.active || index >= self.len() {
            return;
        }
        let slot = self.slot(index);
        std::mem::swap(current, &mut self.others[slot]);
        // The tab put back sits at the new tab's place: move it to its own.
        if index < self.active {
            self.others[index..self.active].rotate_left(1);
        } else {
            self.others[self.active..=slot].rotate_right(1);
        }
        self.active = index;
    }

    /// Adds `tab` after the others and makes it the active one.
    pub fn push(&mut self, current: &mut T, tab: T) {
        let previous = std::mem::replace(current, tab);
        self.others.insert(self.active, previous);
        self.active = self.others.len();
    }

    /// Closes tab `index` and returns it, unless it is the last one. The
    /// active tab gives its place to the next one, or to the previous one
    /// when it was the last.
    pub fn close(&mut self, current: &mut T, index: usize) -> Option<T> {
        if self.others.is_empty() || index >= self.len() {
            return None;
        }
        if index != self.active {
            let slot = self.slot(index);
            if index < self.active {
                self.active -= 1;
            }
            return Some(self.others.remove(slot));
        }
        let next = self.active.min(self.others.len() - 1);
        let closed = std::mem::replace(current, self.others.remove(next));
        self.active = next;
        Some(closed)
    }
}

/// The tab after `index` (or before it), going round at either end.
pub fn step(index: usize, count: usize, forward: bool) -> usize {
    match count {
        0 => 0,
        _ if forward => (index + 1) % count,
        _ => (index + count - 1) % count,
    }
}

/// The name of each tab: its file's, with its folder's when another tab
/// has a file of the same name, or "Untitled".
pub fn names(paths: &[Option<&Path>]) -> Vec<String> {
    let name = |path: Option<&Path>| {
        let untitled = || crate::i18n::tr("Untitled", "Sans titre").to_string();
        path.and_then(Path::file_name).map_or_else(untitled, |n| n.to_string_lossy().into_owned())
    };
    let names: Vec<String> = paths.iter().map(|&p| name(p)).collect();
    paths
        .iter()
        .zip(&names)
        .map(|(path, n)| {
            let folder = path.and_then(Path::parent).and_then(Path::file_name).map(|f| f.to_string_lossy());
            match folder {
                Some(folder) if names.iter().filter(|other| *other == n).count() > 1 => format!("{n} — {folder}"),
                _ => n.clone(),
            }
        })
        .collect()
}

/// Whether two paths name the same file, links and `..` resolved.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || canonical(a) == canonical(b)
}

/// Which tab asked for each render on its way: the worker numbers the
/// renders of all tabs in one sequence.
#[derive(Debug, Default)]
pub struct Owners {
    /// Generations and the tab (by id) that asked, oldest first.
    pending: Vec<(u64, u64)>,
}

impl Owners {
    pub fn claim(&mut self, generation: u64, tab: u64) {
        self.pending.push((generation, tab));
    }

    /// The tab that asked for render `generation`.
    pub fn owner(&self, generation: u64) -> Option<u64> {
        self.pending.iter().find(|(g, _)| *g == generation).map(|&(_, tab)| tab)
    }

    /// Render `generation` came back: the worker answers in order, so no
    /// reply to it or to an older one follows.
    pub fn finished(&mut self, generation: u64) {
        self.pending.retain(|(g, _)| *g > generation);
    }
}

/// The tabs open when the app was last closed: their files, and which one
/// was active. Untitled tabs are not kept.
#[derive(Debug, Default, PartialEq)]
pub struct Session {
    pub paths: Vec<PathBuf>,
    pub active: Option<usize>,
}

impl Session {
    /// Reads the session from `store`; missing or unreadable, it is empty.
    pub fn load(store: Option<&Path>) -> Self {
        store.and_then(|s| std::fs::read_to_string(s).ok()).map_or_else(Session::default, |text| parse(&text))
    }

    pub fn save(&self, store: Option<&Path>) -> std::io::Result<()> {
        let Some(store) = store else { return Ok(()) };
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(store, format(self))
    }
}

/// The session's file in the settings folder, but in screenshot runs,
/// which neither read nor change it.
pub fn store() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(crate::recent::settings_dir()?.join("open-tabs.txt"))
}

/// The active tab's line starts with this.
const ACTIVE: &str = "* ";

/// One absolute path a line, the active one marked.
fn parse(text: &str) -> Session {
    let mut session = Session::default();
    for line in text.lines() {
        let (active, line) = match line.strip_prefix(ACTIVE) {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let path = PathBuf::from(line);
        if !path.is_absolute() || session.paths.contains(&path) {
            continue;
        }
        if active {
            session.active = Some(session.paths.len());
        }
        session.paths.push(path);
    }
    session
}

/// Paths that a line cannot hold (a line break, or not UTF-8) are left out.
fn format(session: &Session) -> String {
    let mut text = String::new();
    for (i, path) in session.paths.iter().enumerate() {
        let Some(line) = path.to_str().filter(|p| !p.contains(['\n', '\r'])) else { continue };
        if session.active == Some(i) {
            text.push_str(ACTIVE);
        }
        text.push_str(line);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tabs in order, and the active one.
    fn order(shelf: &Shelf<char>, current: &char) -> (String, char) {
        (shelf.iter(current).collect(), *current)
    }

    /// Tabs `a` to `e`, the first one active.
    fn five() -> (Shelf<char>, char) {
        let (mut shelf, mut current) = (Shelf::default(), 'a');
        for tab in ['b', 'c', 'd', 'e'] {
            shelf.push(&mut current, tab);
        }
        shelf.select(&mut current, 0);
        (shelf, current)
    }

    #[test]
    fn new_tabs_go_last_and_become_active() {
        let (mut shelf, mut current) = (Shelf::default(), 'a');
        assert_eq!(shelf.len(), 1);
        shelf.push(&mut current, 'b');
        shelf.push(&mut current, 'c');
        assert_eq!(order(&shelf, &current), ("abc".to_string(), 'c'));
        assert_eq!(shelf.active(), 2);
    }

    #[test]
    fn selecting_keeps_the_order() {
        let (mut shelf, mut current) = five();
        for (index, tab) in [(3, 'd'), (1, 'b'), (4, 'e'), (0, 'a'), (2, 'c'), (2, 'c')] {
            shelf.select(&mut current, index);
            assert_eq!(order(&shelf, &current), ("abcde".to_string(), tab));
            assert_eq!(shelf.active(), index);
        }
        shelf.select(&mut current, 9);
        assert_eq!(current, 'c', "out of range: nothing changes");
        assert_eq!(shelf.other_mut(1).copied(), Some('b'));
        assert_eq!(shelf.other_mut(3).copied(), Some('d'));
        assert_eq!(shelf.other_mut(2), None, "the active tab is not on the shelf");
    }

    #[test]
    fn closing_moves_to_the_next_tab_then_the_previous() {
        let (mut shelf, mut current) = five();
        shelf.select(&mut current, 2);
        // Another tab: the active one stays.
        assert_eq!(shelf.close(&mut current, 0), Some('a'));
        assert_eq!(order(&shelf, &current), ("bcde".to_string(), 'c'));
        assert_eq!(shelf.active(), 1);
        assert_eq!(shelf.close(&mut current, 3), Some('e'));
        assert_eq!(order(&shelf, &current), ("bcd".to_string(), 'c'));
        // The active one: the next takes its place.
        assert_eq!(shelf.close(&mut current, 1), Some('c'));
        assert_eq!(order(&shelf, &current), ("bd".to_string(), 'd'));
        // The last one, active: the one before it.
        assert_eq!(shelf.close(&mut current, 1), Some('d'));
        assert_eq!(order(&shelf, &current), ("b".to_string(), 'b'));
        // The only tab is never closed.
        assert_eq!(shelf.close(&mut current, 0), None);
        assert_eq!(order(&shelf, &current), ("b".to_string(), 'b'));
    }

    #[test]
    fn steps_go_round() {
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(1, 3, false), 0);
        assert_eq!(step(0, 1, true), 0);
        assert_eq!(step(0, 1, false), 0);
    }

    #[test]
    fn names_tell_files_of_the_same_name_apart() {
        let (a, b, c) = (Path::new("/x/plate.c2f"), Path::new("/y/plate.c2f"), Path::new("/y/grogu.c2fc"));
        assert_eq!(
            names(&[Some(a), None, Some(b), Some(c), None]),
            ["plate.c2f — x", "Untitled", "plate.c2f — y", "grogu.c2fc", "Untitled"]
        );
    }

    #[test]
    fn the_same_file_by_another_path() {
        let dir = std::env::temp_dir().join(format!("code2form-tabs-same-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("part.c2f");
        std::fs::write(&file, "cube(1);").unwrap();
        assert!(same_file(&file, &dir.join("sub").join("..").join("part.c2f")));
        assert!(same_file(&file, &dir.join(".").join("part.c2f")));
        assert!(!same_file(&file, &dir.join("other.c2f")));
        // Not on disk yet: only the same path is the same file.
        assert!(same_file(&dir.join("new.c2f"), &dir.join("new.c2f")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replies_go_to_the_tab_that_asked() {
        let mut owners = Owners::default();
        owners.claim(5, 1);
        owners.claim(6, 2);
        owners.claim(7, 1);
        assert_eq!(owners.owner(6), Some(2));
        // A circuit's first reply leaves its generation pending.
        assert_eq!(owners.owner(5), Some(1));
        owners.finished(6);
        assert_eq!(owners.owner(5), None, "older replies no longer come");
        assert_eq!(owners.owner(6), None);
        assert_eq!(owners.owner(7), Some(1));
        assert_eq!(owners.owner(8), None, "nobody asked");
    }

    #[test]
    fn the_session_survives_a_save_and_load() {
        let dir = std::env::temp_dir().join(format!("code2form-tabs-session-{}", std::process::id()));
        let store = dir.join("settings").join("open-tabs.txt");
        assert_eq!(Session::load(Some(&store)), Session::default(), "no file yet");
        let path = |name: &str| dir.join(name);
        let session = Session { paths: vec![path("a.c2f"), path("with space.c2fc"), path("b.scad")], active: Some(1) };
        session.save(Some(&store)).unwrap();
        assert_eq!(Session::load(Some(&store)), session);
        // A path a line cannot hold is left out, and the active one moves.
        let odd = Session { paths: vec![path("bad\nname.c2f"), path("a.c2f")], active: Some(1) };
        odd.save(Some(&store)).unwrap();
        assert_eq!(Session::load(Some(&store)), Session { paths: vec![path("a.c2f")], active: Some(0) });
        assert!(Session::load(None).paths.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ignores_bad_session_lines() {
        let a = std::env::temp_dir().join("a.c2f");
        let text = format!("relative.c2f\n\n* {}\n{}\n", a.display(), a.display());
        assert_eq!(parse(&text), Session { paths: vec![a], active: Some(0) });
    }
}
