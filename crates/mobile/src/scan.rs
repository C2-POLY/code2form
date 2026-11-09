//! A scan set straight for the app, right after its reconstruction: the
//! same as the studio's Mesh › Set Straight, so both give the same file.

use std::path::Path;

/// What setting a scan straight did, for the app to say.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LevelReport {
    /// The floating parts taken off.
    pub islands: u32,
    /// How far it was tipped to stand on its base, and turned about Z, in
    /// degrees.
    pub tilt: f64,
    pub turn: f64,
    /// Its size, in mm, along X, Y and Z.
    pub size: Vec<f64>,
}

#[derive(Debug, uniffi::Error)]
pub enum ScanError {
    Failed { message: String },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ScanError {}

/// Sets the scan `file` straight, in place: floating bits off, standing
/// on its base, its footprint along X and Y, centred on z = 0.
#[uniffi::export]
pub fn level_scan(file: String) -> Result<LevelReport, ScanError> {
    let path = Path::new(&file);
    let leveled = c2f_geom::level_file(path, path).map_err(|message| ScanError::Failed { message })?;
    Ok(LevelReport { islands: leveled.islands as u32, tilt: leveled.tilt, turn: leveled.turn, size: leveled.size.to_vec() })
}
