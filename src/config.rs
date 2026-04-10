use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub auth: AuthConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub download: DownloadConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub password: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct OutputConfig {
    #[serde(default = "default_output_dir")]
    pub directory: String,
    #[serde(default = "default_true")]
    pub create_subdirs: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            directory: default_output_dir(),
            create_subdirs: true,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct DownloadConfig {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_true")]
    pub include_metadata: bool,
    #[serde(default)]
    pub include_comments: bool,
    #[serde(default = "default_file_naming")]
    pub file_naming: FileNaming,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 5,
            include_metadata: true,
            include_comments: false,
            file_naming: FileNaming::Id,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileNaming {
    #[default]
    Id,
    Title,
    #[serde(rename = "id-title")]
    IdTitle,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct SourcesConfig {
    #[serde(default)]
    pub friends: bool,
    #[serde(default)]
    pub follows: bool,
    #[serde(default)]
    pub saved: bool,
}

fn default_output_dir() -> String {
    "~/reddit-archive".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent() -> usize {
    5
}

fn default_file_naming() -> FileNaming {
    FileNaming::Id
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Config =
            toml::from_str(&contents).with_context(|| "Failed to parse config file")?;
        Ok(config)
    }

    pub fn resolve_output_dir(&self) -> PathBuf {
        let dir = &self.output.directory;
        if let Some(stripped) = dir.strip_prefix("~/") {
            if let Some(home) = dirs_home() {
                return home.join(stripped);
            }
        }
        PathBuf::from(dir)
    }
}

fn dirs_home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "reddit-dl")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}
