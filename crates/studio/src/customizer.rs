//! The customizer panel: a control for each parameter the file declares
//! with customizer comments, grouped as in the file, and the parameter
//! sets saved next to it (`<model>.json`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Button, ComboBox, CornerRadius, DragValue, Frame, Margin, RichText, Slider, SliderClamping, Stroke, TextEdit,
};
use c2f_eval::customizer::{
    DEFAULT_GROUP, HIDDEN_GROUP, Literal, Parameter, ParameterSets, Widget, parse_typed, set_text,
};

use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme;

/// The preset entry that stands for the values written in the file.
fn design_defaults() -> &'static str {
    tr("Design default values", "Valeurs par défaut du fichier")
}

/// How long a parameter the properties panel opens stays outlined.
const REVEAL_SECONDS: f64 = 1.5;

#[derive(Default)]
pub struct Customizer {
    /// The file's parameters, as of the last render that parsed.
    parameters: Vec<Parameter>,
    /// Values the user set, by parameter name; others keep the file's.
    values: HashMap<String, Literal>,
    /// Text fields for non-string values, which may not read as a value
    /// yet, by parameter name.
    drafts: HashMap<String, Draft>,
    /// `<model>.json`, when the model has been saved.
    sets_path: Option<PathBuf>,
    sets: ParameterSets,
    /// Why the presets file could not be read: it is not written until it
    /// reads again, so as not to replace it.
    unreadable: Option<String>,
    /// The preset last chosen or saved.
    selected: Option<String>,
    /// The name typed for saving a preset.
    new_name: String,
    /// The last problem with presets, until the next action.
    problem: Option<String>,
    /// The parameter the properties panel asked to show, and when it was
    /// first scrolled to.
    revealed: Option<(String, Option<f64>)>,
}

/// A text field and the value it was last in step with.
struct Draft {
    text: String,
    value: Literal,
}

impl Draft {
    fn new(value: &Literal) -> Self {
        Draft { text: set_text(value), value: value.clone() }
    }
}

/// Whether a value set in the panel still applies to `parameters`: the
/// parameter is there, with a value of the same type in the file.
pub fn applies(parameters: &[Parameter], name: &str, value: &Literal) -> bool {
    parameters.iter().any(|p| p.name == name && std::mem::discriminant(&p.default) == std::mem::discriminant(value))
}

impl Customizer {
    /// Forgets the parameters and values, and reads the presets of the
    /// model at `path`.
    pub fn open(&mut self, path: Option<&Path>) {
        self.parameters.clear();
        self.values.clear();
        self.drafts.clear();
        self.selected = None;
        self.read_sets(path);
    }

    /// The model was saved under a new name: its presets file follows,
    /// keeping the parameters and values.
    pub fn moved(&mut self, path: &Path) {
        if self.sets_path.as_deref() != Some(&path.with_extension("json")) {
            self.selected = None;
            self.read_sets(Some(path));
        }
    }

    fn read_sets(&mut self, path: Option<&Path>) {
        self.problem = None;
        self.sets_path = path.map(|p| p.with_extension("json"));
        let read = self.sets_path.as_deref().map_or(Ok(ParameterSets::default()), read_sets);
        (self.sets, self.unreadable) = match read {
            Ok(sets) => (sets, None),
            Err(err) => (ParameterSets::default(), Some(err)),
        };
    }

    /// Takes the parameters of a new render, dropping values that no
    /// longer apply (the render did not use them either).
    pub fn set_parameters(&mut self, parameters: Vec<Parameter>) {
        self.values.retain(|name, value| applies(&parameters, name, value));
        self.drafts.retain(|name, _| parameters.iter().any(|p| &p.name == name));
        self.parameters = parameters;
    }

    /// Whether the file has parameters to show.
    pub fn has_parameters(&self) -> bool {
        self.parameters.iter().any(|p| p.group != HIDDEN_GROUP)
    }

    /// The values to render with, in a stable order.
    pub fn values(&self) -> Vec<(String, Literal)> {
        let mut values: Vec<(String, Literal)> = self.values.iter().map(|(n, v)| (n.clone(), v.clone())).collect();
        values.sort_by(|a, b| a.0.cmp(&b.0));
        values
    }

    fn value<'a>(&'a self, parameter: &'a Parameter) -> &'a Literal {
        self.values.get(&parameter.name).unwrap_or(&parameter.default)
    }

    fn set(&mut self, name: &str, value: Literal) {
        self.problem = None;
        self.values.insert(name.to_string(), value);
    }

    /// Back to the values written in the file.
    fn reset(&mut self) {
        self.problem = None;
        self.values.clear();
        self.drafts.clear();
        self.selected = None;
    }

    /// Draws the card's title, and under it, unless `folded`, the presets
    /// and the parameters; returns whether a value changed. A click on the
    /// title folds or unfolds it.
    pub fn show(&mut self, ui: &mut egui::Ui, folded: &mut bool) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let title = RichText::new(tr("Parameters", "Paramètres")).strong().size(13.5).color(theme::palette().text);
            let toggle = ui.add(Button::new(title).frame(false));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (icon, hint) = if *folded {
                    (Icon::FoldDown, tr("Show the parameters", "Afficher les paramètres"))
                } else {
                    (Icon::FoldUp, tr("Fold the parameters", "Replier les paramètres"))
                };
                if icons::button(ui, icon, false, hint).clicked() || toggle.clicked() {
                    *folded = !*folded;
                }
                if !*folded && !self.values.is_empty() {
                    let hint = tr(
                        "Reset all: back to the values written in the file",
                        "Tout réinitialiser : revenir aux valeurs écrites dans le fichier",
                    );
                    if icons::button(ui, Icon::Reset, false, hint).clicked() {
                        self.reset();
                        changed = true;
                    }
                }
            });
        });
        if *folded {
            return changed;
        }
        ui.add_space(4.0);
        changed |= self.presets(ui);
        for problem in [&self.unreadable, &self.problem].into_iter().flatten() {
            ui.label(RichText::new(problem).size(12.0).color(theme::palette().warning));
        }
        ui.add_space(6.0);

        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            if !self.has_parameters() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr("This file declares no parameters.", "Ce fichier ne déclare aucun paramètre.")).color(theme::palette().text_dim));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr(
                        "Top-level assignments of plain values become parameters, \
                         with an optional comment for their range or choices:",
                        "Les affectations de valeurs simples au premier niveau deviennent des paramètres, \
                         avec un commentaire facultatif pour leur plage ou leurs choix :",
                    ))
                    .size(12.0)
                    .color(theme::palette().text_faint),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("/* [Size] */\n// Width of the box\nwidth = 20; // [10:100]\nstyle = \"round\"; // [round, square]")
                        .monospace()
                        .size(12.0)
                        .color(theme::palette().code_text),
                );
                return;
            }
            for group in self.groups() {
                // The heading of a file without groups is ours to translate.
                let heading = if group == DEFAULT_GROUP { tr(DEFAULT_GROUP, "Paramètres") } else { group.as_str() };
                let title = RichText::new(heading).strong().size(13.0).color(theme::palette().text);
                egui::CollapsingHeader::new(title).id_salt(("customizer-group", &group)).default_open(true).show(
                    ui,
                    |ui| {
                        let parameters: Vec<Parameter> =
                            self.parameters.iter().filter(|p| p.group == group).cloned().collect();
                        for parameter in &parameters {
                            changed |= self.parameter(ui, parameter);
                            ui.add_space(8.0);
                        }
                    },
                );
                ui.add_space(4.0);
            }
        });
        changed
    }

    /// Group names in order of first appearance, without the hidden one.
    fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = Vec::new();
        for p in &self.parameters {
            if p.group != HIDDEN_GROUP && !groups.contains(&p.group) {
                groups.push(p.group.clone());
            }
        }
        // A file without groups gets a single "Parameters" heading.
        if groups.is_empty() { vec![DEFAULT_GROUP.into()] } else { groups }
    }

    /// The preset row: choose, save and delete parameter sets.
    fn presets(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let mut choice = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Preset", "Préréglage")).size(12.0).color(theme::palette().text_dim));
            let current = self.selected.clone().unwrap_or_else(|| design_defaults().into());
            ComboBox::from_id_salt("customizer-preset").selected_text(&current).width(ui.available_width() - 34.0).show_ui(
                ui,
                |ui| {
                    if ui.selectable_label(self.selected.is_none(), design_defaults()).clicked() {
                        choice = Some(None);
                    }
                    for (name, _) in &self.sets.sets {
                        if ui.selectable_label(self.selected.as_ref() == Some(name), name).clicked() {
                            choice = Some(Some(name.clone()));
                        }
                    }
                },
            );
            let delete = ui
                .add_enabled(self.selected.is_some(), Button::new("🗑").frame(false))
                .on_hover_text(tr("Delete this preset", "Supprimer ce préréglage"))
                .on_disabled_hover_text(tr("Choose a preset to delete it", "Choisissez un préréglage pour le supprimer"));
            if delete.clicked() {
                if let Some(name) = self.selected.clone() {
                    if self.update_sets(|sets| sets.remove(&name)) {
                        self.selected = None;
                    }
                }
            }
        });
        match choice {
            Some(None) => {
                self.reset();
                changed = true;
            }
            Some(Some(name)) => {
                self.apply_set(&name);
                changed = true;
            }
            None => {}
        }

        ui.horizontal(|ui| {
            // With an unreadable file, saving reads it again and refuses.
            let can_save = self.sets_path.is_some();
            let field = TextEdit::singleline(&mut self.new_name)
                .hint_text(tr("Name for a new preset", "Nom du nouveau préréglage"))
                .desired_width(ui.available_width() - 64.0);
            let field = ui.add_enabled(can_save, field).on_disabled_hover_text(tr("Save the file first", "Enregistrez d'abord le fichier"));
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = self.new_name.trim().to_string();
            let save = ui
                .add_enabled(can_save && !name.is_empty(), Button::new(tr("Save", "Enregistrer")))
                .on_hover_text(tr(
                    "Save the current values as a preset, next to the file",
                    "Enregistrer les valeurs actuelles comme préréglage, à côté du fichier",
                ));
            if (save.clicked() || enter) && can_save && !name.is_empty() {
                let values: Vec<(String, String)> =
                    self.parameters.iter().map(|p| (p.name.clone(), set_text(self.value(p)))).collect();
                if self.update_sets(|sets| sets.insert(&name, values)) {
                    self.selected = Some(name);
                    self.new_name.clear();
                }
            }
        });
        changed
    }

    /// Takes the values of a preset, each read by its parameter's type.
    fn apply_set(&mut self, name: &str) {
        let Some(set) = self.sets.get(name).map(<[_]>::to_vec) else { return };
        self.values.clear();
        self.drafts.clear();
        self.selected = Some(name.to_string());
        self.problem = None;
        for parameter in self.parameters.clone() {
            for (_, raw) in set.iter().filter(|(n, _)| *n == parameter.name) {
                match parse_typed(&parameter.default, raw) {
                    Ok(Some(value)) => self.set(&parameter.name, value),
                    Ok(None) => {}
                    Err(why) => {
                        self.problem = Some(tr!(
                            "Preset \"{name}\" stops at {}: {why}",
                            "Le préréglage « {name} » s'arrête à {} : {why}",
                            parameter.name
                        ));
                        return;
                    }
                }
            }
        }
    }

    /// Changes the presets file. It is read again first, keeping changes
    /// made since (in another app, say), and replaced whole through a
    /// temporary file. Returns whether it was written.
    fn update_sets(&mut self, change: impl FnOnce(&mut ParameterSets)) -> bool {
        let Some(path) = self.sets_path.clone() else { return false };
        self.problem = None;
        let mut sets = match read_sets(&path) {
            Ok(sets) => sets,
            Err(err) => {
                self.unreadable = Some(err);
                return false;
            }
        };
        self.unreadable = None;
        change(&mut sets);
        let temp = path.with_extension("json.tmp");
        match std::fs::write(&temp, sets.to_json()).and_then(|()| std::fs::rename(&temp, &path)) {
            Ok(()) => {
                self.sets = sets;
                true
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                self.problem = Some(tr!("Cannot write {}: {err}", "Impossible d'écrire {} : {err}", file_name(&path)));
                false
            }
        }
    }

    /// One parameter: its name, description and control.
    fn parameter(&mut self, ui: &mut egui::Ui, parameter: &Parameter) -> bool {
        let mut changed = false;
        let value = self.value(parameter).clone();
        let modified = value != parameter.default;
        let top = ui.cursor().min;
        ui.horizontal(|ui| {
            let mut name = RichText::new(&parameter.name).size(13.0).color(theme::palette().text);
            if modified {
                name = name.color(theme::palette().accent_strong);
            }
            if let (Widget::Checkbox, &Literal::Bool(on)) = (&parameter.widget, &value) {
                let mut on = on;
                if ui.checkbox(&mut on, name).changed() {
                    self.set(&parameter.name, Literal::Bool(on));
                    changed = true;
                }
            } else {
                ui.label(name);
            }
            if modified {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let default = parameter.default.to_source();
                    let reset = ui.add(Button::new(RichText::new("↺").size(13.0)).frame(false));
                    if reset.on_hover_text(tr!("Back to {default}", "Revenir à {default}")).clicked() {
                        self.problem = None;
                        self.values.remove(&parameter.name);
                        changed = true;
                    }
                });
            }
        });
        if let Some(description) = &parameter.description {
            ui.label(RichText::new(description).size(11.5).color(theme::palette().text_dim));
        }
        changed |= self.control(ui, parameter, value);
        self.paint_revealed(ui, parameter, top);
        changed
    }

    /// Scrolls to the parameter [`Self::reveal`] asked for, the first time
    /// it is drawn, and outlines it for a moment.
    fn paint_revealed(&mut self, ui: &mut egui::Ui, parameter: &Parameter, top: egui::Pos2) {
        let Some((name, since)) = &mut self.revealed else { return };
        if *name != parameter.name {
            return;
        }
        let now = ui.input(|i| i.time);
        let rect = egui::Rect::from_min_max(top, egui::pos2(ui.max_rect().right(), ui.cursor().top())).expand(3.0);
        if since.is_none() {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
            *since = Some(now);
        }
        if now - since.unwrap_or(now) < REVEAL_SECONDS {
            ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, theme::palette().accent), egui::StrokeKind::Outside);
            ui.ctx().request_repaint();
        } else {
            self.revealed = None;
        }
    }

    /// The file's parameters with the values they render with, for the
    /// properties panel.
    pub fn parameter_values(&self) -> Vec<(String, Literal)> {
        self.parameters.iter().map(|p| (p.name.clone(), self.value(p).clone())).collect()
    }

    /// Scrolls the panel to parameter `name` and outlines it, as the
    /// properties panel asks for a value that names it.
    pub fn reveal(&mut self, name: &str) {
        self.revealed = Some((name.to_string(), None));
    }

    fn control(&mut self, ui: &mut egui::Ui, parameter: &Parameter, value: Literal) -> bool {
        let name = parameter.name.as_str();
        match (&parameter.widget, value) {
            (Widget::Checkbox, _) => false,
            (&Widget::Slider { min, max, step }, Literal::Number(current)) => {
                ui.spacing_mut().slider_width = (ui.available_width() - 76.0).max(60.0);
                let mut n = current;
                // `Edits` clamps what the user does, not the value shown: a
                // default off the grid stays as written until touched.
                let response = ui.add(
                    Slider::new(&mut n, min..=max)
                        .clamping(SliderClamping::Edits)
                        .smart_aim(false)
                        .max_decimals(decimals(step).max(decimals(current))),
                );
                // Steps count from the minimum, as in the reference.
                let snapped = (min + ((n - min) / step).round() * step).clamp(min, max);
                if response.changed() && snapped != current {
                    self.set(name, Literal::Number(snapped));
                    return true;
                }
                false
            }
            (&Widget::Spinbox { step }, Literal::Number(mut n)) => {
                let field = DragValue::new(&mut n).speed(step).max_decimals(decimals(step).max(6));
                if ui.add_sized([ui.available_width().min(140.0), 22.0], field).changed() {
                    self.set(name, Literal::Number(n));
                    return true;
                }
                false
            }
            (Widget::Dropdown { options }, value) => {
                let shown = options.iter().find(|(v, _)| *v == value).map_or_else(|| set_text(&value), |(_, l)| l.clone());
                let mut chosen = None;
                ComboBox::from_id_salt(("customizer", name)).selected_text(shown).width(ui.available_width()).show_ui(
                    ui,
                    |ui| {
                        for (option, label) in options {
                            if ui.selectable_label(*option == value, label).clicked() {
                                chosen = Some(option.clone());
                            }
                        }
                    },
                );
                match chosen {
                    Some(option) if option != value => {
                        self.set(name, option);
                        true
                    }
                    _ => false,
                }
            }
            (&Widget::Vector { step }, Literal::Vector(items)) => {
                let mut numbers: Vec<f64> =
                    items.iter().map(|item| if let Literal::Number(n) = item { *n } else { 0.0 }).collect();
                let mut changed = false;
                ui.horizontal(|ui| {
                    let width = (ui.available_width() / numbers.len() as f32 - 6.0).max(40.0);
                    for n in &mut numbers {
                        let field = DragValue::new(n).speed(step).max_decimals(decimals(step).max(6));
                        changed |= ui.add_sized([width, 22.0], field).changed();
                    }
                });
                if changed {
                    self.set(name, Literal::Vector(numbers.into_iter().map(Literal::Number).collect()));
                }
                changed
            }
            (&Widget::Text { max_length }, Literal::Str(mut text)) => {
                let mut field = TextEdit::singleline(&mut text).desired_width(f32::INFINITY);
                if let Some(max) = max_length {
                    field = field.char_limit(max);
                }
                if ui.add(field).changed() {
                    self.set(name, Literal::Str(text));
                    return true;
                }
                false
            }
            // Any other value is typed in .scad notation; it is used
            // once it reads as a value of the same type.
            (_, value) => {
                let read = |text: &str| parse_typed(&parameter.default, text).ok().flatten();
                let draft = self.drafts.entry(name.to_string()).or_insert_with(|| Draft::new(&value));
                // The value changed elsewhere (the file, a preset, a reset).
                if draft.value != value {
                    *draft = Draft::new(&value);
                }
                let readable = read(&draft.text).is_some();
                let mut field = TextEdit::singleline(&mut draft.text).desired_width(f32::INFINITY).code_editor();
                if !readable {
                    field = field.text_color(theme::palette().error);
                }
                let response = ui.add(field);
                let edited = response.changed();
                if !readable {
                    response.on_hover_text(tr(
                        "Not a value of the same type as in the file",
                        "Pas une valeur du même type que dans le fichier",
                    ));
                }
                match read(&draft.text) {
                    Some(new) if edited && new != value => {
                        draft.value = new.clone();
                        self.set(name, new);
                        true
                    }
                    _ => false,
                }
            }
        }
    }
}

/// The presets file at `path`; none when it does not exist.
fn read_sets(path: &Path) -> Result<ParameterSets, String> {
    if !path.exists() {
        return Ok(ParameterSets::default());
    }
    std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|json| ParameterSets::parse(&json))
        .map_err(|err| {
            tr!(
                "Cannot read {} ({err}); presets are not saved until it is fixed",
                "Impossible de lire {} ({err}) ; les préréglages ne seront pas enregistrés avant sa correction",
                file_name(path)
            )
        })
}

/// How many decimals a step needs.
fn decimals(step: f64) -> usize {
    (0..6).find(|&d| (step * 10f64.powi(d as i32)).fract().abs() < 1e-9).unwrap_or(6)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The panel's frame.
pub fn frame() -> Frame {
    Frame::new()
        .fill(theme::palette().panel)
        .inner_margin(Margin { left: 12, right: 12, top: 10, bottom: 8 })
        .stroke(Stroke::new(1.0, theme::palette().border))
        .corner_radius(CornerRadius::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(name: &str, default: Literal) -> Parameter {
        Parameter {
            name: name.into(),
            group: DEFAULT_GROUP.into(),
            description: None,
            widget: Widget::Spinbox { step: 1.0 },
            default,
        }
    }

    #[test]
    fn presets_are_read_by_type() {
        let mut customizer = Customizer::default();
        customizer.set_parameters(vec![
            parameter("n", Literal::Number(1.0)),
            parameter("on", Literal::Bool(true)),
            parameter("s", Literal::Str("a".into())),
        ]);
        customizer.sets.insert("good", vec![("n".into(), "5".into()), ("s".into(), "hi there".into())]);
        customizer.sets.insert("bad", vec![("n".into(), "7".into()), ("on".into(), "maybe".into())]);
        customizer.apply_set("good");
        assert_eq!(
            customizer.values(),
            [("n".to_string(), Literal::Number(5.0)), ("s".to_string(), Literal::Str("hi there".into()))]
        );
        customizer.apply_set("bad");
        // Read in parameter order: n is set before the bad boolean stops it.
        assert_eq!(customizer.values(), [("n".to_string(), Literal::Number(7.0))]);
        assert!(customizer.problem.is_some());
    }

    #[test]
    fn values_of_a_changed_type_are_dropped() {
        let mut customizer = Customizer::default();
        customizer.set_parameters(vec![parameter("n", Literal::Number(1.0)), parameter("m", Literal::Number(1.0))]);
        customizer.set("n", Literal::Number(4.0));
        customizer.set("m", Literal::Number(4.0));
        customizer.set_parameters(vec![parameter("n", Literal::Str("x".into())), parameter("m", Literal::Number(2.0))]);
        assert_eq!(customizer.values(), [("m".to_string(), Literal::Number(4.0))]);
    }

    fn temp_model(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-presets-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("model.scad")
    }

    #[test]
    fn an_unreadable_presets_file_is_never_replaced() {
        let model = temp_model("unreadable");
        let json = model.with_extension("json");
        std::fs::write(&json, "{ \"parameterSets\": { \"a\": {}, }").unwrap();
        let mut customizer = Customizer::default();
        customizer.open(Some(&model));
        assert!(customizer.unreadable.is_some());
        assert!(!customizer.update_sets(|sets| sets.insert("b", vec![])));
        assert_eq!(std::fs::read_to_string(&json).unwrap(), "{ \"parameterSets\": { \"a\": {}, }");
        // Once the file is fixed, saving works again.
        std::fs::write(&json, "{ \"parameterSets\": { \"a\": {} } }").unwrap();
        assert!(customizer.update_sets(|sets| sets.insert("b", vec![])));
        assert!(customizer.unreadable.is_none());
        let names: Vec<String> = customizer.sets.sets.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, ["a", "b"]);
        let _ = std::fs::remove_dir_all(model.parent().unwrap());
    }

    #[test]
    fn saving_keeps_presets_added_meanwhile() {
        let model = temp_model("meanwhile");
        let json = model.with_extension("json");
        let _ = std::fs::remove_file(&json);
        let mut customizer = Customizer::default();
        customizer.open(Some(&model));
        assert!(customizer.update_sets(|sets| sets.insert("mine", vec![("n".into(), "1".into())])));
        // Another program adds a preset to the file.
        let mut other = ParameterSets::parse(&std::fs::read_to_string(&json).unwrap()).unwrap();
        other.insert("theirs", vec![]);
        std::fs::write(&json, other.to_json()).unwrap();
        assert!(customizer.update_sets(|sets| sets.remove("mine")));
        let names: Vec<String> = ParameterSets::parse(&std::fs::read_to_string(&json).unwrap())
            .unwrap()
            .sets
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        // Deleting the last of ours keeps the file and theirs.
        assert_eq!(names, ["theirs"]);
        let _ = std::fs::remove_dir_all(model.parent().unwrap());
    }

    #[test]
    fn opening_another_file_forgets_its_parameters() {
        let mut customizer = Customizer::default();
        customizer.set_parameters(vec![parameter("n", Literal::Number(1.0))]);
        customizer.set("n", Literal::Number(3.0));
        customizer.open(None);
        assert!(!customizer.has_parameters() && customizer.values().is_empty());
    }

    #[test]
    fn step_decimals() {
        assert_eq!(decimals(1.0), 0);
        assert_eq!(decimals(0.25), 2);
        assert_eq!(decimals(0.1), 1);
    }
}
