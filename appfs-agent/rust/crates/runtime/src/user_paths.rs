use std::env;
use std::path::PathBuf;

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[must_use]
pub fn user_home_dir() -> Option<PathBuf> {
    if let Some(home) = non_empty_env_path("HOME") {
        return Some(home);
    }

    #[cfg(windows)]
    {
        if let Some(profile) = non_empty_env_path("USERPROFILE") {
            return Some(profile);
        }

        let home_drive = env::var_os("HOMEDRIVE").filter(|value| !value.is_empty());
        let home_path = env::var_os("HOMEPATH").filter(|value| !value.is_empty());
        if let (Some(home_drive), Some(home_path)) = (home_drive, home_path) {
            return Some(PathBuf::from(format!(
                "{}{}",
                home_drive.to_string_lossy(),
                home_path.to_string_lossy()
            )));
        }
    }

    None
}

#[must_use]
pub fn claw_config_home() -> Option<PathBuf> {
    non_empty_env_path("CLAW_CONFIG_HOME").or_else(|| user_home_dir().map(|home| home.join(".claw")))
}

#[cfg(test)]
mod tests {
    use super::{claw_config_home, user_home_dir};

    #[test]
    fn user_home_dir_prefers_home_when_present() {
        let _guard = crate::test_env_lock();
        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let original_homedrive = std::env::var_os("HOMEDRIVE");
        let original_homepath = std::env::var_os("HOMEPATH");

        std::env::set_var("HOME", "/tmp/test-home");
        std::env::set_var("USERPROFILE", "C:\\Users\\Fallback");
        std::env::remove_var("HOMEDRIVE");
        std::env::remove_var("HOMEPATH");

        assert_eq!(user_home_dir(), Some(std::path::PathBuf::from("/tmp/test-home")));

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match original_homedrive {
            Some(value) => std::env::set_var("HOMEDRIVE", value),
            None => std::env::remove_var("HOMEDRIVE"),
        }
        match original_homepath {
            Some(value) => std::env::set_var("HOMEPATH", value),
            None => std::env::remove_var("HOMEPATH"),
        }
    }

    #[test]
    fn claw_config_home_uses_explicit_override() {
        let _guard = crate::test_env_lock();
        let original_config_home = std::env::var_os("CLAW_CONFIG_HOME");
        let original_home = std::env::var_os("HOME");

        std::env::set_var("CLAW_CONFIG_HOME", "/tmp/custom-claw");
        std::env::set_var("HOME", "/tmp/test-home");

        assert_eq!(
            claw_config_home(),
            Some(std::path::PathBuf::from("/tmp/custom-claw"))
        );

        match original_config_home {
            Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
            None => std::env::remove_var("CLAW_CONFIG_HOME"),
        }
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn user_home_dir_falls_back_to_userprofile_on_windows() {
        let _guard = crate::test_env_lock();
        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        let original_homedrive = std::env::var_os("HOMEDRIVE");
        let original_homepath = std::env::var_os("HOMEPATH");

        std::env::remove_var("HOME");
        std::env::set_var("USERPROFILE", r"C:\Users\Tester");
        std::env::remove_var("HOMEDRIVE");
        std::env::remove_var("HOMEPATH");

        assert_eq!(
            user_home_dir(),
            Some(std::path::PathBuf::from(r"C:\Users\Tester"))
        );

        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match original_homedrive {
            Some(value) => std::env::set_var("HOMEDRIVE", value),
            None => std::env::remove_var("HOMEDRIVE"),
        }
        match original_homepath {
            Some(value) => std::env::set_var("HOMEPATH", value),
            None => std::env::remove_var("HOMEPATH"),
        }
    }
}
