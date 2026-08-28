use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

const APPLICATION_DIRECTORY: &str = "ai-agent-control-center";
const DEFAULT_DATA_DIRECTORIES: &str = "/usr/local/share:/usr/share";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxPaths {
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    runtime_directory: PathBuf,
    data_directories: Vec<PathBuf>,
    runtime_uses_cache_fallback: bool,
}

impl LinuxPaths {
    pub fn discover() -> Result<Self, String> {
        Self::from_values(
            env::var_os("HOME"),
            env::var_os("XDG_DATA_HOME"),
            env::var_os("XDG_CONFIG_HOME"),
            env::var_os("XDG_CACHE_HOME"),
            env::var_os("XDG_RUNTIME_DIR"),
            env::var_os("XDG_DATA_DIRS"),
        )
    }

    fn from_values(
        home: Option<OsString>,
        data_home: Option<OsString>,
        config_home: Option<OsString>,
        cache_home: Option<OsString>,
        runtime_directory: Option<OsString>,
        data_directories: Option<OsString>,
    ) -> Result<Self, String> {
        let home = absolute_path(home.as_deref()).ok_or_else(|| {
            "The absolute home directory is unavailable for XDG resolution.".to_string()
        })?;
        let data_home = absolute_path(data_home.as_deref())
            .unwrap_or_else(|| home.join(".local").join("share"));
        let config_home =
            absolute_path(config_home.as_deref()).unwrap_or_else(|| home.join(".config"));
        let cache_home =
            absolute_path(cache_home.as_deref()).unwrap_or_else(|| home.join(".cache"));

        let (runtime_directory, runtime_uses_cache_fallback) =
            if let Some(root) = absolute_path(runtime_directory.as_deref()) {
                (root.join(APPLICATION_DIRECTORY), false)
            } else {
                (cache_home.join(APPLICATION_DIRECTORY).join("runtime"), true)
            };

        let system_data_directories = match data_directories.as_deref() {
            None => OsString::from(DEFAULT_DATA_DIRECTORIES),
            Some(value) if value.is_empty() => OsString::from(DEFAULT_DATA_DIRECTORIES),
            Some(value) => value.to_os_string(),
        };
        let mut resolved_data_directories = vec![data_home.clone()];
        let mut seen = HashSet::from([data_home.clone()]);
        for directory in
            env::split_paths(&system_data_directories).filter(|path| path.is_absolute())
        {
            if seen.insert(directory.clone()) {
                resolved_data_directories.push(directory);
            }
        }

        Ok(Self {
            home,
            data_home,
            config_home,
            cache_home,
            runtime_directory,
            data_directories: resolved_data_directories,
            runtime_uses_cache_fallback,
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    #[cfg(test)]
    pub fn data_home(&self) -> &Path {
        &self.data_home
    }

    pub fn config_home(&self) -> &Path {
        &self.config_home
    }

    #[cfg(test)]
    pub fn cache_home(&self) -> &Path {
        &self.cache_home
    }

    pub fn data_directories(&self) -> &[PathBuf] {
        &self.data_directories
    }

    pub fn application_data_directory(&self) -> PathBuf {
        self.data_home.join(APPLICATION_DIRECTORY)
    }

    pub fn application_config_directory(&self) -> PathBuf {
        self.config_home.join(APPLICATION_DIRECTORY)
    }

    pub fn application_cache_directory(&self) -> PathBuf {
        self.cache_home.join(APPLICATION_DIRECTORY)
    }

    #[cfg(test)]
    pub fn runtime_directory(&self) -> &Path {
        &self.runtime_directory
    }

    #[cfg(test)]
    pub fn runtime_uses_cache_fallback(&self) -> bool {
        self.runtime_uses_cache_fallback
    }

    pub fn voice_data_directory(&self) -> PathBuf {
        self.application_data_directory().join("voice-runtime")
    }

    pub fn voice_config_directory(&self) -> PathBuf {
        self.application_config_directory().join("voice-runtime")
    }

    pub fn voice_cache_directory(&self) -> PathBuf {
        self.application_cache_directory().join("voice-runtime")
    }

    pub fn voice_runtime_directory(&self) -> PathBuf {
        self.runtime_directory.join("voice")
    }

    pub fn kwin_runtime_directory(&self) -> PathBuf {
        self.runtime_directory.join("kwin")
    }
}

fn absolute_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_0016_xdg_paths_honor_absolute_overrides_and_order() {
        let paths = LinuxPaths::from_values(
            Some(OsString::from("/home/example")),
            Some(OsString::from("/data/user")),
            Some(OsString::from("/config/user")),
            Some(OsString::from("/cache/user")),
            Some(OsString::from("/run/user/1000")),
            Some(OsString::from(
                "/data/user:/opt/share:relative:/usr/share:/opt/share",
            )),
        )
        .unwrap();

        assert_eq!(paths.home(), Path::new("/home/example"));
        assert_eq!(
            paths.voice_data_directory(),
            Path::new("/data/user/ai-agent-control-center/voice-runtime")
        );
        assert_eq!(
            paths.voice_config_directory(),
            Path::new("/config/user/ai-agent-control-center/voice-runtime")
        );
        assert_eq!(
            paths.voice_cache_directory(),
            Path::new("/cache/user/ai-agent-control-center/voice-runtime")
        );
        assert_eq!(
            paths.voice_runtime_directory(),
            Path::new("/run/user/1000/ai-agent-control-center/voice")
        );
        assert_eq!(
            paths.data_directories(),
            &[
                PathBuf::from("/data/user"),
                PathBuf::from("/opt/share"),
                PathBuf::from("/usr/share"),
            ]
        );
        assert!(!paths.runtime_uses_cache_fallback());
    }

    #[test]
    fn task_0016_xdg_paths_reject_relative_values_and_bound_runtime_fallback() {
        let paths = LinuxPaths::from_values(
            Some(OsString::from("/home/example")),
            Some(OsString::from("relative-data")),
            Some(OsString::from("relative-config")),
            Some(OsString::from("/cache/user")),
            Some(OsString::from("relative-runtime")),
            Some(OsString::from("relative-system")),
        )
        .unwrap();

        assert_eq!(paths.data_home(), Path::new("/home/example/.local/share"));
        assert_eq!(paths.config_home(), Path::new("/home/example/.config"));
        assert_eq!(paths.cache_home(), Path::new("/cache/user"));
        assert_eq!(
            paths.runtime_directory(),
            Path::new("/cache/user/ai-agent-control-center/runtime")
        );
        assert_eq!(
            paths.data_directories(),
            &[PathBuf::from("/home/example/.local/share")]
        );
        assert!(paths.runtime_uses_cache_fallback());
    }
}
