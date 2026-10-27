//! User-level settings and ownership of generated setup files.
//!
//! Setup never opens a mailbox or invokes a host. Same-user filesystem access is
//! trusted, as for mail, but symlink destinations and changed content conflict.

mod files;
mod inspect;

pub use inspect::{
    DirectoryState, FileState, Inspection, Ownership, SetupState, SkillState, inspect,
};

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Component, Path, PathBuf},
};

/// A bounded setup or settings failure.
#[derive(Debug)]
pub struct SetupError {
    /// Stable machine-readable identity.
    pub code: &'static str,
    /// Actionable explanation; no host configuration content is included.
    pub message: String,
}
impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for SetupError {}
impl From<std::io::Error> for SetupError {
    fn from(e: std::io::Error) -> Self {
        failure("INIT_IO", &e.to_string())
    }
}
fn failure(code: &'static str, message: &str) -> SetupError {
    SetupError {
        code,
        message: message.into(),
    }
}
fn input(message: &str) -> SetupError {
    failure("INIT_INPUT", message)
}
fn conflict(path: &Path) -> SetupError {
    failure(
        "INIT_CONFLICT",
        &format!(
            "Preserved conflicting setup path: {}. Keep local edits and resolve this path before retrying; run merl help init for recovery guidance.",
            path.display()
        ),
    )
}

/// Known user locations. No project directories are searched.
#[derive(Clone, Debug)]
pub struct Locations {
    /// User settings file, also anchoring its setup receipt and lock.
    pub config: PathBuf,
    /// Durable mailbox suggestion; not used until setup is accepted.
    pub mail: PathBuf,
    /// Codex personal skill root.
    pub codex_skills: PathBuf,
    /// Claude personal skill root.
    pub claude_skills: PathBuf,
}
impl Locations {
    /// Resolves platform user locations without reading or creating files.
    ///
    /// # Errors
    /// Returns `INIT_INPUT` for absent or non-absolute user locations.
    pub fn from_environment() -> Result<Self, SetupError> {
        let profile = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| input("HOME must identify an absolute user directory"))?;
        absolute(&profile)?;
        let config_root = if cfg!(target_os = "macos") {
            profile.join("Library/Application Support")
        } else {
            environment_path("XDG_CONFIG_HOME", profile.join(".config"))?
        };
        let data_root = if cfg!(target_os = "macos") {
            profile.join("Library/Application Support")
        } else {
            environment_path("XDG_DATA_HOME", profile.join(".local/share"))?
        };
        let claude = environment_path("CLAUDE_CONFIG_DIR", profile.join(".claude"))?;
        Ok(Self {
            config: config_root.join("merl/config.json"),
            mail: data_root.join("merl/mail"),
            codex_skills: profile.join(".agents/skills"),
            claude_skills: claude.join("skills"),
        })
    }
}
/// Returns the known platform user settings path without reading it.
///
/// # Errors
/// Returns `INIT_INPUT` if its user location is absent or relative.
pub fn default_config() -> Result<PathBuf, SetupError> {
    let profile = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| input("HOME must name an absolute user directory"))?;
    absolute(&profile)?;
    Ok(if cfg!(target_os = "macos") {
        profile.join("Library/Application Support/merl/config.json")
    } else {
        environment_path("XDG_CONFIG_HOME", profile.join(".config"))?.join("merl/config.json")
    })
}

fn environment_path(name: &str, fallback: PathBuf) -> Result<PathBuf, SetupError> {
    let p = std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map_or(fallback, PathBuf::from);
    absolute(&p)?;
    Ok(p)
}
pub(crate) fn absolute(path: &Path) -> Result<(), SetupError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path.to_str().is_none()
    {
        return Err(input(
            "setup paths must be absolute UTF-8 paths without dot components",
        ));
    }
    Ok(())
}

/// Saved opt-in choices. Skill paths name files, not host configuration files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Dedicated absolute mailbox home.
    pub mail_home: PathBuf,
    /// Selected hosts and their generated SKILL.md destinations.
    pub skills: BTreeMap<String, PathBuf>,
    /// Whether to show foreground dispatcher instructions; never starts it.
    pub foreground: bool,
}
impl Settings {
    fn value(&self) -> Value {
        json!({"schema":"merl.config/v1", "mail_home":self.mail_home,"skills":self.skills,"dispatcher":if self.foreground {"foreground"} else {"none"}})
    }
    fn bytes(&self) -> String {
        format!("{}\n", self.value())
    }
    fn parse(value: &Value) -> Result<Self, SetupError> {
        let bad = || {
            failure(
                "INIT_CONFIG",
                "Invalid Merl settings; use an explicit --home to bypass settings or repair the selected config",
            )
        };
        if value["schema"] != "merl.config/v1" {
            return Err(bad());
        }
        let mail_home = PathBuf::from(value["mail_home"].as_str().ok_or_else(bad)?);
        absolute(&mail_home).map_err(|_| bad())?;
        let mut skills = BTreeMap::new();
        for (host, path) in value["skills"].as_object().ok_or_else(bad)? {
            if !matches!(host.as_str(), "codex" | "claude") {
                return Err(bad());
            }
            let path = PathBuf::from(path.as_str().ok_or_else(bad)?);
            absolute(&path).map_err(|_| bad())?;
            skills.insert(host.clone(), path);
        }
        let foreground = match value["dispatcher"].as_str() {
            Some("foreground") => true,
            Some("none") => false,
            _ => return Err(bad()),
        };
        Ok(Self {
            mail_home,
            skills,
            foreground,
        })
    }
}

/// Reads one known settings file, without directory discovery or creation.
///
/// # Errors
/// Returns `INIT_CONFIG` for malformed, oversized or inaccessible settings.
pub fn load(config: &Path) -> Result<Option<Settings>, SetupError> {
    absolute(config)?;
    files::read(config)
        .map_err(|_| failure("INIT_CONFIG", "Cannot read selected Merl settings"))?
        .map(|text| {
            let value = serde_json::from_str(&text)
                .map_err(|_| failure("INIT_CONFIG", "Invalid JSON in selected Merl settings"))?;
            Settings::parse(&value)
        })
        .transpose()
}

/// Reads completed settings or the choices in an interrupted setup receipt.
///
/// Mail commands deliberately use only [`load`]; incomplete setup is not a
/// mailbox default. This function is for showing, resuming or removing setup.
///
/// # Errors
/// Returns a settings or receipt error rather than guessing partial choices.
pub fn setup_settings(config: &Path) -> Result<Option<Settings>, SetupError> {
    let recorded = files::recorded_settings(config)?;
    if !files::is_complete(config)? && recorded.is_some() {
        return Ok(recorded);
    }
    load(config)
}

/// Reports whether the last setup attempt reached its durable completion marker.
///
/// # Errors
/// Returns a receipt error rather than claiming success after partial setup.
pub fn setup_is_complete(config: &Path) -> Result<bool, SetupError> {
    files::is_complete(config)
}

/// Resolves a mailbox, bypassing all environment/settings reads for explicit home.
///
/// # Errors
/// Returns `INIT_CONFIG` for invalid settings, or `INIT_INPUT` when unconfigured.
pub fn resolve_home(home: Option<&str>, config: Option<&str>) -> Result<PathBuf, SetupError> {
    if let Some(home) = home {
        let p = PathBuf::from(home);
        absolute(&p)?;
        return Ok(p);
    }
    let path = if let Some(config) = config {
        PathBuf::from(config)
    } else {
        default_config()?
    };
    load(&path)?.map(|s| s.mail_home).ok_or_else(|| {
        input("Supply --home <absolute-path> or run merl init to save a mailbox default")
    })
}

/// Version-bound generated skill used for installation and safe upgrades.
#[derive(Clone, Copy, Debug)]
pub struct Bundle<'a> {
    /// Application version used in the receipt.
    pub version: &'a str,
    /// Exact bundled skill bytes, as UTF-8.
    pub skill: &'a str,
}

/// A complete, preflighted setup plan. Constructing it never mutates files.
#[derive(Debug)]
pub struct Plan<'a> {
    config: PathBuf,
    settings: Settings,
    bundle: Bundle<'a>,
    removing: bool,
}

/// One generated file's proposed effect; no mutation has happened yet.
#[derive(Debug)]
pub struct FileChange {
    /// Absolute managed destination.
    pub path: PathBuf,
    /// Proposed publication or retention.
    pub action: Change,
    /// Selected host, or `None` for settings.
    pub host: Option<String>,
    /// True when retaining the known legacy skill beside the primary skill.
    pub retained_legacy: bool,
    /// Existing ownership, independent of the proposed action.
    pub ownership: Ownership,
}
/// A validated setup plan's file effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Create an absent generated file.
    Create,
    /// Replace unchanged owned generated bytes.
    Update,
    /// Keep identical owned bytes.
    Unchanged,
    /// Use identical pre-existing bytes without adopting ownership.
    Borrowed,
    /// Remove unchanged owned bytes.
    Remove,
    /// Keep a file without publishing new bytes.
    Retain,
}
impl<'a> Plan<'a> {
    /// Describes proposed file effects, optionally selecting uninstall hosts.
    ///
    /// # Errors
    /// Returns the same preflight errors as construction if files changed since.
    pub fn changes(&self, hosts: &[String]) -> Result<Vec<FileChange>, SetupError> {
        files::changes(self, hosts)
    }
    /// Validates all generated destinations and existing content before mutation.
    ///
    /// # Errors
    /// Returns `INIT_INPUT` for overlapping/invalid paths, `INIT_CONFLICT` for
    /// borrowed or locally changed content, or `INIT_IO` for inaccessible files.
    pub fn new(
        config: PathBuf,
        settings: Settings,
        bundle: Bundle<'a>,
    ) -> Result<Self, SetupError> {
        let plan = Self {
            config,
            settings,
            bundle,
            removing: false,
        };
        files::validate(&plan)?;
        Ok(plan)
    }
    /// Validates removal without requiring borrowed skills to match this version.
    ///
    /// # Errors
    /// Returns the same path/content errors as [`Self::new`]. Local edits to
    /// previously recorded files still conflict and are always preserved.
    pub fn removal(
        config: PathBuf,
        settings: Settings,
        bundle: Bundle<'a>,
    ) -> Result<Self, SetupError> {
        let plan = Self {
            config,
            settings,
            bundle,
            removing: true,
        };
        files::validate(&plan)?;
        Ok(plan)
    }
    /// Applies the plan, reporting each durable file publication to the caller.
    ///
    /// The callback supports progress and cancellation. Its error stops setup;
    /// retrying this plan resumes recorded intent without adopting borrowed files.
    /// No mailbox contents or host executables are accessed.
    ///
    /// # Errors
    /// Returns a conflict if preflight changed, `INIT_BUSY` for concurrent setup,
    /// or `INIT_IO` for failure/cancellation. Earlier publications remain recorded.
    pub fn apply(
        &self,
        published: &mut impl FnMut(&Path) -> std::io::Result<()>,
    ) -> Result<bool, SetupError> {
        if self.removing {
            return Err(input("a removal plan cannot install files"));
        }
        files::apply(self, published).map_err(|mut error| {
            if error.code == "INIT_IO" {
                error.message = format!(
                    "Setup incomplete; retry the same init options. {}",
                    error.message
                );
            }
            error
        })
    }
    /// Removes unchanged owned files for selected hosts, or all setup when empty.
    ///
    /// Borrowed files, mailbox directories and data are always retained. No
    /// binary is removed and no dispatcher process is stopped.
    ///
    /// # Errors
    /// Returns a conflict on edits/symlinks, `INIT_BUSY` or `INIT_IO` on failure.
    pub fn uninstall(
        &self,
        hosts: &[String],
        published: &mut impl FnMut(&Path) -> std::io::Result<()>,
    ) -> Result<bool, SetupError> {
        files::uninstall(self, hosts, published)
    }
}
