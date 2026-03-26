use serde::{Deserialize, Serialize};

/// Project configuration loaded from .project.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub name: String,
    pub asm_path: String,
    #[serde(default)]
    pub debug_path: Option<String>,
    #[serde(default)]
    pub rom_path: Option<String>,
    #[serde(default)]
    pub fdd_path: Option<String>,
    #[serde(default)]
    pub fdd_content_path: Option<String>,
    #[serde(default)]
    pub fdd_template_path: Option<String>,
    #[serde(default)]
    pub rom_align: Option<usize>,
    #[serde(default)]
    pub dependent_projects_dir: Option<String>,
    #[serde(default)]
    pub cpu: Option<String>,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

impl ProjectConfig {
    pub fn cpu_mode(&self) -> CpuMode {
        match self.cpu.as_deref() {
            Some("z80") => CpuMode::Z80,
            _ => CpuMode::I8080,
        }
    }
}

/// Target CPU mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuMode {
    I8080,
    Z80,
}
