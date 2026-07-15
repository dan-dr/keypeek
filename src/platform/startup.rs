#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupStatus {
    Unavailable,
    Disabled,
    Enabled,
    RequiresApproval,
}

impl StartupStatus {
    pub fn is_available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

#[cfg(any(target_os = "linux", test))]
fn escape_desktop_exec_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str(r"\\\\"),
            '"' => {
                escaped.push_str(r"\\");
                escaped.push('"');
            }
            '`' => escaped.push_str(r"\\`"),
            '$' => escaped.push_str(r"\\$"),
            '%' => escaped.push_str("%%"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::StartupStatus;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    fn app_bundle() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?;
        executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .map(Path::to_path_buf)
    }

    fn is_signed(bundle: &Path) -> bool {
        let verified = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(bundle)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !verified {
            return false;
        }

        Command::new("/usr/bin/codesign")
            .args(["--display", "--verbose=2"])
            .arg(bundle)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stderr).ok())
            .is_some_and(|details| details.lines().any(|line| line.starts_with("Authority=")))
    }

    fn service_management_is_available() -> bool {
        Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|version| version.split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 13)
    }

    pub fn status() -> StartupStatus {
        if !service_management_is_available() {
            return StartupStatus::Unavailable;
        }
        let Some(bundle) = app_bundle() else {
            return StartupStatus::Unavailable;
        };
        if !is_signed(&bundle) {
            return StartupStatus::Unavailable;
        }

        let service = unsafe { SMAppService::mainAppService() };
        match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => StartupStatus::Enabled,
            SMAppServiceStatus::RequiresApproval => StartupStatus::RequiresApproval,
            _ => StartupStatus::Disabled,
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<StartupStatus, String> {
        if !status().is_available() {
            return Err("Start on login requires a signed KeyPeek app bundle".to_string());
        }

        let service = unsafe { SMAppService::mainAppService() };
        let result = if enabled {
            unsafe { service.registerAndReturnError() }
        } else {
            unsafe { service.unregisterAndReturnError() }
        };
        result.map_err(|error| error.to_string())?;
        Ok(status())
    }
}

#[cfg(target_os = "windows")]
mod implementation {
    use super::StartupStatus;
    use std::io;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "KeyPeek";

    fn executable() -> Option<std::path::PathBuf> {
        let executable = std::env::current_exe().ok()?;
        if executable.starts_with(std::env::temp_dir())
            || executable
                .components()
                .any(|component| component.as_os_str() == "target")
        {
            return None;
        }
        Some(executable)
    }

    fn command() -> Option<String> {
        Some(format!("\"{}\"", executable()?.display()))
    }

    pub fn status() -> StartupStatus {
        let Some(command) = command() else {
            return StartupStatus::Unavailable;
        };
        let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ)
        else {
            return StartupStatus::Disabled;
        };
        let Ok(value): Result<String, _> = key.get_value(VALUE_NAME) else {
            return StartupStatus::Disabled;
        };
        if command == value {
            StartupStatus::Enabled
        } else {
            StartupStatus::Disabled
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<StartupStatus, String> {
        let command = command()
            .ok_or_else(|| "Install KeyPeek before enabling Start on login".to_string())?;
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root
            .create_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
            .map_err(|error| error.to_string())?;
        let existing: Result<String, _> = key.get_value(VALUE_NAME);
        match &existing {
            Ok(existing) if existing != &command => {
                return Err("A different KeyPeek startup command already exists".to_string());
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(error.to_string());
            }
            _ => {}
        }
        if enabled {
            key.set_value(VALUE_NAME, &command)
                .map_err(|error| error.to_string())?;
        } else if existing.is_ok() {
            key.delete_value(VALUE_NAME)
                .map_err(|error| error.to_string())?;
        } else if let Err(error) = key.delete_value(VALUE_NAME) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error.to_string());
            }
        }
        Ok(status())
    }
}

#[cfg(target_os = "linux")]
mod implementation {
    use super::{escape_desktop_exec_arg, StartupStatus};
    use directories::BaseDirs;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn desktop_file() -> Option<PathBuf> {
        BaseDirs::new().map(|dirs| {
            dirs.config_dir()
                .join("autostart")
                .join("dev.srwi.KeyPeek.desktop")
        })
    }

    fn executable() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?;
        if executable.starts_with(std::env::temp_dir())
            || executable
                .components()
                .any(|component| component.as_os_str() == "target")
        {
            return None;
        }
        Some(executable)
    }

    fn desktop_contents(executable: &Path) -> String {
        let escaped = escape_desktop_exec_arg(&executable.to_string_lossy());
        format!(
            "[Desktop Entry]\nType=Application\nName=KeyPeek\nExec=\"{escaped}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        )
    }

    pub fn status() -> StartupStatus {
        let (Some(path), Some(executable)) = (desktop_file(), executable()) else {
            return StartupStatus::Unavailable;
        };
        match fs::read_to_string(path) {
            Ok(contents) if contents == desktop_contents(&executable) => StartupStatus::Enabled,
            _ => StartupStatus::Disabled,
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<StartupStatus, String> {
        let path =
            desktop_file().ok_or_else(|| "Could not find the config directory".to_string())?;
        let executable = executable()
            .ok_or_else(|| "Install KeyPeek before enabling Start on login".to_string())?;
        let expected_contents = desktop_contents(&executable);
        if enabled {
            match fs::read_to_string(&path) {
                Ok(contents) if contents != expected_contents => {
                    return Err("A different KeyPeek autostart entry already exists".to_string());
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.to_string());
                }
                _ => {}
            }
            let parent = path
                .parent()
                .ok_or_else(|| "Invalid autostart path".to_string())?;
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            fs::write(path, expected_contents).map_err(|error| error.to_string())?;
        } else {
            match fs::read_to_string(&path) {
                Ok(contents) if contents == expected_contents => {
                    fs::remove_file(path).map_err(|error| error.to_string())?;
                }
                Ok(_) => {
                    return Err("A different KeyPeek autostart entry already exists".to_string());
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.to_string());
                }
                _ => {}
            }
        }
        Ok(status())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod implementation {
    use super::StartupStatus;

    pub fn status() -> StartupStatus {
        StartupStatus::Unavailable
    }

    pub fn set_enabled(_enabled: bool) -> Result<StartupStatus, String> {
        Err("Start on login is not supported on this platform".to_string())
    }
}

pub use implementation::{set_enabled, status};

#[cfg(test)]
mod tests {
    use super::escape_desktop_exec_arg;

    #[test]
    fn desktop_exec_escapes_field_codes_and_quoted_characters() {
        assert_eq!(escape_desktop_exec_arg("100%"), "100%%");
        assert_eq!(escape_desktop_exec_arg("$`"), r"\\$\\`");
        assert_eq!(escape_desktop_exec_arg("\\"), r"\\\\");
        assert_eq!(escape_desktop_exec_arg("\"").as_bytes(), b"\\\\\"");
    }
}
