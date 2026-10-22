use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    pub ui_scale: f32,
    pub snapping_enabled: bool,
    pub linked_selection: bool,
    pub loop_enabled: bool,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            ui_scale: 1.5, // 150% по умолчанию для комфортного чтения на 2K/4K экранах
            snapping_enabled: true,
            linked_selection: true,
            loop_enabled: false,
        }
    }
}

impl UserPreferences {
    pub fn config_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/verizon_nle")
        } else {
            PathBuf::from(".config/verizon_nle")
        }
    }

    pub fn preferences_path() -> PathBuf {
        Self::config_dir().join("preferences.json")
    }

    pub fn dock_layout_path() -> PathBuf {
        Self::config_dir().join("dock_layout.json")
    }

    pub fn load() -> Self {
        let path = Self::preferences_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(prefs) = serde_json::from_str::<UserPreferences>(&content) {
                return prefs;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let dir = Self::config_dir();
        let _ = fs::create_dir_all(&dir);
        let path = Self::preferences_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}
