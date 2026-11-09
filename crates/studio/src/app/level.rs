//! Mesh › Set a Scan Straight: the scan's floating bits off, standing on
//! its base, its footprint along X and Y, as the iPhone does after a scan
//! (`c2f_geom::level_file`). The scan is rewritten in place, so the
//! designs that import it move with it: the user confirms, and the
//! original is kept in `originals/` beside it, which a project does not
//! take in.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Button, Id, Layout, RichText};

use super::StudioApp;
use crate::console::Level;
use crate::i18n::tr;
use crate::theme;

/// The folder beside a scan where its original is kept.
pub(super) const ORIGINALS: &str = "originals";

impl StudioApp {
    /// Asks for a scan to set straight, in the project's scans or the
    /// open file's folder.
    pub(super) fn level_scan_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr("Set a scan straight", "Remettre un scan d'aplomb"))
            .add_filter(tr("Meshes", "Maillages"), &["stl", "obj", "3mf"]);
        let scans = self.project.as_ref().map(|open| open.project.dir.join(c2f_project::Kind::Scan.folder()));
        if let Some(dir) = scans.filter(|d| d.is_dir()).or_else(|| self.doc.directory()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(file) = dialog.pick_file() {
            self.level_asked = Some(file);
        }
    }

    /// The confirmation, while a scan waits to be set straight.
    pub(super) fn level_dialog(&mut self, ctx: &egui::Context) {
        let Some(file) = self.level_asked.clone() else { return };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut choice = None;
        let modal = egui::Modal::new(Id::new("confirm-level")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(tr!("Set {name} straight?", "Remettre {name} d'aplomb ?")).heading().color(theme::palette().text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr(
                    "Its floating bits go, it stands on the flat face it rests on and its footprint turns along X and Y, centred. \
                     The designs that import it move with it. The original is kept in originals/ beside it.",
                    "Ses morceaux flottants partent, il se pose sur sa face d'appui et son empreinte s'aligne sur X et Y, centrée. \
                     Les pièces qui l'importent bougent avec lui. L'original est gardé dans originals/, à côté.",
                ))
                .color(theme::palette().text_dim),
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let go = Button::new(RichText::new(tr("Set Straight", "Remettre d'aplomb")).color(theme::palette().on_accent).strong())
                    .fill(theme::palette().accent);
                if ui.add(go).clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel", "Annuler")).clicked() {
                    choice = Some(false);
                }
            });
        });
        if choice.is_none() && modal.should_close() {
            choice = Some(false);
        }
        if let Some(go) = choice {
            self.level_asked = None;
            if go {
                self.level_scan(&file);
            }
        }
    }

    fn level_scan(&mut self, file: &Path) {
        match level_keeping_original(file) {
            Ok((leveled, original)) => {
                let [x, y, z] = leveled.size;
                let mut message = tr!(
                    "{} set straight: {x:.1} × {y:.1} × {z:.1} mm, turned {:.1}°",
                    "{} remis d'aplomb : {x:.1} × {y:.1} × {z:.1} mm, tourné de {:.1}°",
                    file.display(),
                    leveled.turn
                );
                if leveled.base {
                    message += &tr!(", tipped {:.1}°", ", redressé de {:.1}°", leveled.tilt);
                }
                if leveled.islands > 0 {
                    message += &tr!(", {} floating bit(s) off", ", {} morceau(x) flottant(s) retiré(s)", leveled.islands);
                }
                if let Some(original) = original {
                    message += &tr!("; the original is in {}", " ; l'original est dans {}", original.display());
                }
                self.doc.console.log(Level::Info, message);
                self.render_now();
            }
            Err(error) => self.doc.console.log(Level::Error, tr!("Cannot set {} straight: {error}", "Impossible de remettre {} d'aplomb : {error}", file.display())),
        }
    }
}

/// Sets `file` straight in place, its original first copied to
/// `originals/` beside it (unless one is there already: the first is the
/// one as scanned); the copy made, if any.
pub(super) fn level_keeping_original(file: &Path) -> Result<(c2f_geom::Leveled, Option<PathBuf>), String> {
    let name = file.file_name().ok_or_else(|| format!("{} names no file", file.display()))?;
    let folder = file.parent().unwrap_or(Path::new("")).join(ORIGINALS);
    let original = folder.join(name);
    let copied = if original.exists() {
        None
    } else {
        std::fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
        std::fs::copy(file, &original).map_err(|e| format!("cannot keep the original in {}: {e}", original.display()))?;
        Some(original)
    };
    c2f_geom::level_file(file, file).map(|leveled| (leveled, copied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projects_scan_is_set_straight_its_original_kept_out_of_the_project() {
        let root = std::env::temp_dir().join(format!("c2f-studio-level-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut project = c2f_project::create(&root.join("Lamp"), "Lamp").unwrap();
        let scan = project.dir.join("scans/scan.stl");
        // A 30 × 10 × 5 mm bar, turned 20° about Z.
        let (s, c) = 20f64.to_radians().sin_cos();
        let corner = |i: usize| {
            let p = [if i & 1 == 1 { 30.0 } else { 0.0 }, if i & 2 == 2 { 10.0 } else { 0.0 }, if i & 4 == 4 { 5.0 } else { 0.0 }];
            [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
        };
        let mut text = String::from("solid bar\n");
        for [a, b, cc, d] in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            for t in [[a, b, cc], [a, cc, d]] {
                text += "facet normal 0 0 0\nouter loop\n";
                for i in t {
                    let [x, y, z] = corner(i);
                    text += &format!("vertex {x} {y} {z}\n");
                }
                text += "endloop\nendfacet\n";
            }
        }
        text += "endsolid bar\n";
        std::fs::write(&scan, &text).unwrap();
        project.add(c2f_project::Kind::Scan, &scan).unwrap();
        project.save().unwrap();

        let (leveled, original) = level_keeping_original(&scan).unwrap();
        assert!((leveled.size[0] - 30.0).abs() < 0.01 && (leveled.size[1] - 10.0).abs() < 0.01, "{:?}", leveled.size);
        let original = original.unwrap();
        assert_eq!(std::fs::read_to_string(&original).unwrap(), text, "kept as scanned");
        // Again: the first original stays, the scan is already straight.
        let (again, none) = level_keeping_original(&scan).unwrap();
        assert!(none.is_none() && again.turn.abs() < 0.01);
        assert_eq!(std::fs::read_to_string(&original).unwrap(), text);
        // The project still names the scan only: originals/ is not one of
        // its scans.
        let read = c2f_project::Project::open(&project.file).unwrap();
        assert_eq!(read.scans, ["scans/scan.stl"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
