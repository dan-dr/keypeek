use crate::protocols::{ConnectionIdentity, ConnectionSpec};
use directories::ProjectDirs;
use ini::Ini;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const ALL_LAYERS_MASK: u32 = u32::MAX;

fn default_visible_layers() -> u32 {
    ALL_LAYERS_MASK
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ConnectionPriority {
    #[default]
    LastConnected,
    Manual,
}

impl fmt::Display for ConnectionPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConnectionPriority::LastConnected => "last_connected",
            ConnectionPriority::Manual => "manual",
        })
    }
}

impl FromStr for ConnectionPriority {
    type Err = ParseSettingsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "last_connected" => Ok(ConnectionPriority::LastConnected),
            "manual" => Ok(ConnectionPriority::Manual),
            _ => Err(ParseSettingsError),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SavedConnection {
    #[serde(default = "default_connection_enabled")]
    pub enabled: bool,
    pub display_name: String,
    pub identity: ConnectionIdentity,
    pub spec: ConnectionSpec,
    pub layout_name: Option<String>,
    #[serde(default = "default_visible_layers")]
    pub visible_layers: u32,
    #[serde(default)]
    pub last_connected_at: u64,
}

fn default_connection_enabled() -> bool {
    true
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProtocolType {
    #[default]
    Via,
    Vial,
    Zmk,
}

impl fmt::Display for ProtocolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ProtocolType::Via => "via",
                ProtocolType::Vial => "vial",
                ProtocolType::Zmk => "zmk",
            }
        )
    }
}

#[derive(Debug)]
pub struct ParseSettingsError;

impl FromStr for ProtocolType {
    type Err = ParseSettingsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "via" => Ok(ProtocolType::Via),
            "vial" => Ok(ProtocolType::Vial),
            "zmk" => Ok(ProtocolType::Zmk),
            _ => Err(ParseSettingsError),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Bottom,
    Top,
}

impl fmt::Display for WindowPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                WindowPosition::TopLeft => "Top Left",
                WindowPosition::TopRight => "Top Right",
                WindowPosition::BottomLeft => "Bottom Left",
                WindowPosition::BottomRight => "Bottom Right",
                WindowPosition::Bottom => "Bottom",
                WindowPosition::Top => "Top",
            }
        )
    }
}

impl FromStr for WindowPosition {
    type Err = ParseSettingsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "Top Left" => Ok(WindowPosition::TopLeft),
            "Top Right" => Ok(WindowPosition::TopRight),
            "Bottom Left" => Ok(WindowPosition::BottomLeft),
            "Bottom Right" => Ok(WindowPosition::BottomRight),
            "Bottom" => Ok(WindowPosition::Bottom),
            "Top" => Ok(WindowPosition::Top),
            _ => Err(ParseSettingsError),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ThemeColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ThemeColor {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

impl fmt::Display for ThemeColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.r, self.g, self.b, self.a)
    }
}

impl FromStr for ThemeColor {
    type Err = ParseSettingsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split(',').map(str::trim);
        let (Some(r), Some(g), Some(b), Some(a)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ParseSettingsError);
        };

        if parts.next().is_some() {
            return Err(ParseSettingsError);
        }

        Ok(Self {
            r: r.parse().map_err(|_| ParseSettingsError)?,
            g: g.parse().map_err(|_| ParseSettingsError)?,
            b: b.parse().map_err(|_| ParseSettingsError)?,
            a: a.parse().map_err(|_| ParseSettingsError)?,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThemeSettings {
    pub layer_colors: [ThemeColor; 7],
    pub font_color: ThemeColor,
}

impl ThemeSettings {
    pub fn layer_color(&self, layer: u8) -> ThemeColor {
        if let Some(color) = self.layer_colors.get(layer as usize) {
            *color
        } else {
            self.layer_colors[6]
        }
    }
}

impl Default for ThemeSettings {
    fn default() -> Self {
        const ALPHA: u8 = 239;
        Self {
            layer_colors: [
                ThemeColor::new(83, 83, 83, ALPHA),
                ThemeColor::new(80, 140, 115, ALPHA),
                ThemeColor::new(100, 115, 150, ALPHA),
                ThemeColor::new(140, 110, 150, ALPHA),
                ThemeColor::new(95, 121, 127, ALPHA),
                ThemeColor::new(147, 137, 110, ALPHA),
                ThemeColor::new(127, 127, 127, ALPHA),
            ],
            font_color: ThemeColor::new(255, 255, 255, 255),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub size: i32,
    pub font_size_multiplier: f32,
    pub auto_fit_before_ellipsis: bool,
    pub position: WindowPosition,
    pub timeout: i64,
    pub margin: u32,
    pub theme: ThemeSettings,
    pub auto_connect: bool,
    pub connection_priority: ConnectionPriority,
    pub saved_connections: Vec<SavedConnection>,
    unparsed_saved_connections: Vec<serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            size: 60,
            font_size_multiplier: 1.0,
            auto_fit_before_ellipsis: false,
            position: WindowPosition::BottomRight,
            timeout: 2000,
            margin: 10,
            theme: ThemeSettings::default(),
            auto_connect: false,
            connection_priority: ConnectionPriority::LastConnected,
            saved_connections: Vec::new(),
            unparsed_saved_connections: Vec::new(),
        }
    }
}

impl Settings {
    pub fn config_file_path() -> Option<PathBuf> {
        Self::project_dirs().map(|dirs| dirs.config_dir().join("settings.ini"))
    }

    fn project_dirs() -> Option<ProjectDirs> {
        ProjectDirs::from("dev", "srwi", "KeyPeek")
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_file_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine the KeyPeek config directory",
            )
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.save_to_file(&path)
    }

    pub fn load() -> Option<Self> {
        Self::config_file_path()
            .and_then(Self::load_from_file)
            .or_else(|| Self::load_from_file("settings.ini"))
    }

    pub fn save_to_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let mut conf = Ini::new();
        let mut section = conf.with_section(Some("settings"));
        section.set("size", self.size.to_string());
        section.set(
            "font_size_multiplier",
            self.font_size_multiplier.to_string(),
        );
        section.set(
            "auto_fit_before_ellipsis",
            self.auto_fit_before_ellipsis.to_string(),
        );
        section.set("position", self.position.to_string());
        section.set("timeout", self.timeout.to_string());
        section.set("margin", self.margin.to_string());
        for (index, color) in self.theme.layer_colors.iter().enumerate() {
            section.set(format!("layer_color_{index}"), color.to_string());
        }
        section.set("font_color", self.theme.font_color.to_string());
        let mut saved_connections = self
            .saved_connections
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        saved_connections.extend(self.unparsed_saved_connections.clone());
        let saved_connections = serde_json::to_string(&saved_connections)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut section = conf.with_section(Some("connections"));
        section.set("auto_connect", self.auto_connect.to_string());
        section.set("priority", self.connection_priority.to_string());
        section.set("saved", saved_connections);
        conf.write_to_file(path)
    }

    pub fn load_from_file(path: impl AsRef<Path>) -> Option<Self> {
        let conf = Ini::load_from_file(path).ok()?;
        let section = conf.section(Some("settings"))?;
        let mut s = Settings::default();
        if let Some(val) = section.get("size") {
            s.size = val.parse().unwrap_or(s.size);
        }
        if let Some(val) = section.get("font_size_multiplier") {
            let parsed = val.parse::<f32>().unwrap_or(s.font_size_multiplier);
            s.font_size_multiplier = parsed.clamp(0.1, 2.0);
        }
        if let Some(val) = section.get("auto_fit_before_ellipsis") {
            s.auto_fit_before_ellipsis = val.parse().unwrap_or(s.auto_fit_before_ellipsis);
        }
        if let Some(val) = section.get("position") {
            if let Ok(parsed) = val.parse() {
                s.position = parsed;
            }
        }
        if let Some(val) = section.get("timeout") {
            let parsed = val.parse::<i64>().unwrap_or(s.timeout);
            s.timeout = if parsed < 0 {
                -1
            } else {
                parsed.clamp(0, 14_999)
            };
        }
        if let Some(val) = section.get("margin") {
            s.margin = val.parse().unwrap_or(s.margin);
        }
        for index in 0..s.theme.layer_colors.len() {
            if let Some(val) = section.get(format!("layer_color_{index}")) {
                if let Ok(parsed) = val.parse() {
                    s.theme.layer_colors[index] = parsed;
                }
            }
        }
        if let Some(val) = section.get("font_color") {
            if let Ok(parsed) = val.parse() {
                s.theme.font_color = parsed;
            }
        }
        if let Some(section) = conf.section(Some("connections")) {
            if let Some(val) = section.get("auto_connect") {
                s.auto_connect = val.parse().unwrap_or(s.auto_connect);
            }
            if let Some(val) = section.get("priority") {
                s.connection_priority = val.parse().unwrap_or(s.connection_priority);
            }
            if let Some(val) = section.get("saved") {
                if let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(val) {
                    for entry in entries {
                        match serde_json::from_value(entry.clone()) {
                            Ok(connection) => s.saved_connections.push(connection),
                            Err(_) => s.unparsed_saved_connections.push(entry),
                        }
                    }
                }
            }
        }
        Some(s)
    }

    pub fn upsert_saved_connection(&mut self, connection: SavedConnection) -> usize {
        let existing_index = self
            .saved_connections
            .iter()
            .position(|saved| saved.identity == connection.identity);

        let mut index = if let Some(index) = existing_index {
            let enabled = self.saved_connections[index].enabled;
            let visible_layers = self.saved_connections[index].visible_layers;
            self.saved_connections[index] = SavedConnection {
                enabled,
                visible_layers,
                ..connection
            };
            index
        } else {
            self.saved_connections.push(connection);
            self.saved_connections.len() - 1
        };

        if self.connection_priority == ConnectionPriority::LastConnected && index != 0 {
            let connection = self.saved_connections.remove(index);
            self.saved_connections.insert(0, connection);
            index = 0;
        }

        index
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionPriority, SavedConnection, Settings};
    use crate::protocols::{ConnectionIdentity, ConnectionSpec};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn saved_connection(name: &str, path: &str) -> SavedConnection {
        SavedConnection {
            enabled: true,
            display_name: name.to_string(),
            identity: ConnectionIdentity::Via {
                vid: 0x1234,
                pid: 0x5678,
                json_path: path.to_string(),
            },
            spec: ConnectionSpec::Via {
                json_path: path.to_string(),
            },
            layout_name: Some("LAYOUT".to_string()),
            visible_layers: super::ALL_LAYERS_MASK,
            last_connected_at: 1,
        }
    }

    #[test]
    fn settings_round_trip_saved_connections() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("keypeek-settings-{unique}.ini"));
        let mut settings = Settings {
            auto_connect: true,
            connection_priority: ConnectionPriority::Manual,
            ..Settings::default()
        };
        settings
            .saved_connections
            .push(saved_connection("Sofle", "/tmp/sofle.json"));

        settings.save_to_file(&path).unwrap();
        let loaded = Settings::load_from_file(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(loaded, settings);
    }

    #[test]
    fn malformed_saved_entry_does_not_discard_valid_connections() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("keypeek-settings-invalid-{unique}.ini"));
        let valid = serde_json::to_value(saved_connection("Sofle", "/tmp/sofle.json")).unwrap();
        let unknown = serde_json::json!({"future_connection": true});
        std::fs::write(
            &path,
            format!(
                "[settings]\nsize=60\n[connections]\nsaved={}\n",
                serde_json::to_string(&vec![valid, unknown.clone()]).unwrap()
            ),
        )
        .unwrap();

        let loaded = Settings::load_from_file(&path).unwrap();
        assert_eq!(loaded.saved_connections.len(), 1);
        let draft = loaded.clone();
        draft.save_to_file(&path).unwrap();
        let reloaded = Settings::load_from_file(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(reloaded.saved_connections.len(), 1);
        assert_eq!(reloaded.unparsed_saved_connections, vec![unknown]);
    }

    #[test]
    fn malformed_saved_json_does_not_discard_other_settings() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("keypeek-settings-broken-{unique}.ini"));
        std::fs::write(&path, "[settings]\nsize=87\n[connections]\nsaved=[broken\n").unwrap();

        let loaded = Settings::load_from_file(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(loaded.size, 87);
        assert!(loaded.saved_connections.is_empty());
    }

    #[test]
    fn upsert_deduplicates_and_preserves_enabled_state() {
        let mut settings = Settings::default();
        settings
            .saved_connections
            .push(saved_connection("Old name", "/tmp/sofle.json"));
        settings.saved_connections[0].enabled = false;
        settings.saved_connections[0].visible_layers = 0b0101;
        let mut updated = saved_connection("Sofle", "/tmp/sofle.json");
        updated.last_connected_at = 2;

        settings.upsert_saved_connection(updated);

        assert_eq!(settings.saved_connections.len(), 1);
        assert_eq!(settings.saved_connections[0].display_name, "Sofle");
        assert!(!settings.saved_connections[0].enabled);
        assert_eq!(settings.saved_connections[0].visible_layers, 0b0101);
        assert_eq!(settings.saved_connections[0].last_connected_at, 2);
    }

    #[test]
    fn saved_connection_without_layer_visibility_defaults_to_all_layers() {
        let mut value = serde_json::to_value(saved_connection("Sofle", "/tmp/sofle.json")).unwrap();
        value.as_object_mut().unwrap().remove("visible_layers");

        let loaded: SavedConnection = serde_json::from_value(value).unwrap();

        assert_eq!(loaded.visible_layers, super::ALL_LAYERS_MASK);
    }

    #[test]
    fn last_connected_priority_moves_updated_connection_to_front() {
        let mut settings = Settings {
            saved_connections: vec![
                saved_connection("Sofle", "/tmp/sofle.json"),
                saved_connection("Charybdis", "/tmp/charybdis.json"),
            ],
            ..Settings::default()
        };

        settings.upsert_saved_connection(saved_connection("Charybdis", "/tmp/charybdis.json"));

        assert_eq!(settings.saved_connections[0].display_name, "Charybdis");
    }

    #[test]
    fn manual_priority_keeps_existing_order() {
        let mut settings = Settings {
            connection_priority: ConnectionPriority::Manual,
            saved_connections: vec![
                saved_connection("Sofle", "/tmp/sofle.json"),
                saved_connection("Charybdis", "/tmp/charybdis.json"),
            ],
            ..Settings::default()
        };

        settings.upsert_saved_connection(saved_connection("Charybdis", "/tmp/charybdis.json"));

        assert_eq!(settings.saved_connections[0].display_name, "Sofle");
    }
}
