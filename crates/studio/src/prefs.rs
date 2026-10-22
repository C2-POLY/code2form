//! Settings kept per user between launches, each in a small text file of
//! the settings folder, as Open Recent keeps its list.

use std::path::PathBuf;

/// How much wire to add to each wire's routed length, in millimeters: two
/// stripped ends of 5 mm. `c2f circuit wires` adds the same by default.
pub const DEFAULT_WIRE_MARGIN: f64 = 10.0;

/// The margin added to every wire's length, kept between launches.
#[derive(Debug)]
pub struct WireMargin {
    pub mm: f64,
    /// Where it is kept; `None` keeps it for this run only.
    store: Option<PathBuf>,
}

impl WireMargin {
    /// Reads it from `store`; missing or unreadable, it is the default.
    pub fn load(store: Option<PathBuf>) -> Self {
        let read = store.as_deref().and_then(|s| std::fs::read_to_string(s).ok());
        let mm = read.and_then(|text| text.trim().parse::<f64>().ok()).filter(|mm| mm.is_finite() && *mm >= 0.0);
        WireMargin { mm: mm.unwrap_or(DEFAULT_WIRE_MARGIN), store }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(store) = &self.store else { return Ok(()) };
        if let Some(dir) = store.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(store, format!("{}\n", self.mm))
    }
}

/// The margin's file in the settings folder, but in screenshot runs,
/// which neither read nor change it.
pub fn wire_margin_store() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(crate::recent::settings_dir()?.join("wire-margin.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_margin_survives_a_save_and_load() {
        let store = std::env::temp_dir().join(format!("code2form-margin-{}/wire-margin.txt", std::process::id()));
        let _ = std::fs::remove_file(&store);
        assert_eq!(WireMargin::load(Some(store.clone())).mm, DEFAULT_WIRE_MARGIN);
        let mut margin = WireMargin::load(Some(store.clone()));
        margin.mm = 25.5;
        margin.save().unwrap();
        assert_eq!(WireMargin::load(Some(store.clone())).mm, 25.5);
        std::fs::write(&store, "-3").unwrap();
        assert_eq!(WireMargin::load(Some(store.clone())).mm, DEFAULT_WIRE_MARGIN);
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }
}
