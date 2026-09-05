use anyhow::{bail, Context, Result};
use directories::BaseDirs;
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub preview_width: usize,
    pub watch: bool,
    pub recent_days: u32,
    pub max_record_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preview_width: 36,
            watch: true,
            recent_days: 7,
            max_record_bytes: 4 * 1024 * 1024,
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let dirs = BaseDirs::new().context("Cannot locate the platform configuration directory")?;
        Ok(dirs.config_dir().join("codex-nav/config.toml"))
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::metadata(path) {
            Ok(meta) if !meta.is_file() => bail!("Navigator configuration is not a regular file"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).context("Cannot inspect Navigator configuration"),
            _ => (),
        }
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).context("Cannot read Navigator configuration"),
        };
        let mut text = String::new();
        file.take(65_537)
            .read_to_string(&mut text)
            .context("Navigator configuration must be UTF-8 text")?;
        if text.len() > 65_536 {
            bail!("Navigator configuration exceeds 64 KiB");
        }
        // Do not echo TOML errors: they can include a complete input line.
        let config: Self = toml::from_str(&text)
            .map_err(|_| anyhow::anyhow!("Invalid Navigator configuration TOML"))?;
        if !(4..=500).contains(&config.preview_width) {
            bail!("preview_width must be between 4 and 500");
        }
        if !(1..=3660).contains(&config.recent_days) {
            bail!("recent_days must be between 1 and 3660");
        }
        if !(1024..=64 * 1024 * 1024).contains(&config.max_record_bytes) {
            bail!("max_record_bytes must be between 1024 and 67108864");
        }
        Ok(config)
    }
}
