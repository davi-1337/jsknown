use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub project: String,
    pub output_root: Option<PathBuf>,
    pub scope_patterns: Vec<String>,
    pub rate_per_second: u32,
    pub rate_per_minute: u32,
    pub fetch_concurrency: usize,
    pub max_body_bytes: usize,
    pub debug: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3333,
            project: "default".to_string(),
            output_root: None,
            scope_patterns: Vec::new(),
            rate_per_second: 2,
            rate_per_minute: 0,
            fetch_concurrency: 5,
            max_body_bytes: 25_000_000,
            debug: false,
        }
    }
}

impl Config {
    pub fn project_root(&self) -> PathBuf {
        self.output_root
            .clone()
            .unwrap_or_else(default_output_root)
            .join(&self.project)
    }
}

fn default_output_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jsknown")
}
