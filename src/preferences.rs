use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct UserPreferences {
    pub ui_scale: f32,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            ui_scale: 1.5, // 150% по умолчанию для комфортного чтения на современных мониторах
        }
    }
}

impl UserPreferences {
    pub fn config_path() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/verizon_nle/preferences.txt")
        } else {
            PathBuf::from("verizon_preferences.txt")
        }
    }

    pub fn load() -> Self {
        let mut prefs = Self::default();
        let path = Self::config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == "ui_scale" {
                        if let Ok(val) = v.trim().parse::<f32>() {
                            prefs.ui_scale = val.clamp(0.5, 4.0);
                        }
                    }
                }
            }
        }
        prefs
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let content = format!("ui_scale={:.2}\n", self.ui_scale);
        let _ = fs::write(&path, content);
    }
}
