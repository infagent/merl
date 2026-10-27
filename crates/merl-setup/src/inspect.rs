//! Bounded local setup observations, without locks or runtime-store access.

use super::{Bundle, Settings, SetupError, absolute, conflict, files, load};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Observed setup completeness, independent of dispatcher health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    /// Neither settings nor a setup receipt exists.
    NotConfigured,
    /// Settings, directory and expected local skill files agree.
    Configured,
    /// Recorded setup is unfinished or some selected artifact needs attention.
    Incomplete,
}
/// An observed generated file's content state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Expected content is present.
    Installed,
    /// Recorded content is intact but differs from the current bundled skill.
    Outdated,
    /// The selected file is absent.
    Missing,
    /// Publication or removal remains recorded as unfinished.
    Pending,
    /// Content differs from recorded expected bytes; preserved.
    Modified,
    /// Unmanaged bytes differ from the current bundle; no modification history is known.
    Different,
    /// A symlink, invalid file type or bounded-read failure prevents inspection.
    Conflict,
}
/// Receipt ownership, independent of whether bytes match the current bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// The receipt records ownership.
    Owned,
    /// Identical pre-existing bytes were explicitly left unowned.
    Borrowed,
    /// No ownership record exists.
    Unmanaged,
    /// Publication has pending intent but no completed ownership acknowledgment.
    Pending,
}
/// A directory type observation; no contents are opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryState {
    /// The selected directory exists.
    Exists,
    /// The selected directory does not exist.
    Missing,
    /// Its type, ancestry or access prevents safe inspection.
    Conflict,
}
impl DirectoryState {
    /// Checks a directory path without creating it or opening its contents.
    ///
    /// # Errors
    /// Returns `INIT_INPUT` for an invalid absolute path. Unsafe ancestry/type
    /// or inaccessible metadata is reported as a conflict observation.
    pub fn inspect(path: &Path) -> Result<Self, SetupError> {
        absolute(path)?;
        if files::check_path(path).is_err() {
            return Ok(Self::Conflict);
        }
        Ok(match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => Self::Exists,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            _ => Self::Conflict,
        })
    }
}
/// One selected host skill, exposing metadata rather than its contents.
#[derive(Debug)]
pub struct SkillState {
    /// Selected local host.
    pub host: String,
    /// Exact selected file path.
    pub path: PathBuf,
    /// Observed content state.
    pub state: FileState,
    /// Recorded ownership, not inferred from matching bytes alone.
    pub ownership: Ownership,
    /// Whether current bytes equal this binary's bundled skill.
    pub matches_bundle: bool,
    /// Retained compatibility path, never a canonical publication target.
    pub retained_legacy: bool,
}
/// Setup facts from one selected settings file and its bounded receipt.
#[derive(Debug)]
pub struct Inspection {
    /// Current saved choices, or interrupted choices if no settings were published.
    pub settings: Option<Settings>,
    /// True only when an actual settings file exists; receipt choices are not defaults.
    pub config_present: bool,
    /// Configuration file content state.
    pub config_state: FileState,
    /// Overall setup completeness.
    pub state: SetupState,
    /// True when receipt publication remains unfinished.
    pub pending: bool,
    /// Selected local skills and their observations.
    pub skills: Vec<SkillState>,
    /// Type of the selected mailbox directory, if any.
    pub mailbox: Option<DirectoryState>,
}

/// Inspects local setup without mutating files or observing host/session health.
///
/// # Errors
/// Returns `INIT_CONFIG` for invalid selected settings, or a receipt error for
/// invalid ownership metadata. Individual skill conflicts remain observations.
pub fn inspect(config: &Path, bundle: Bundle<'_>) -> Result<Inspection, SetupError> {
    let saved = load(config)?;
    let config_present = saved.is_some();
    let receipt = files::receipt_value(config)?;
    let mut recorded = BTreeMap::new();
    let mut pending = false;
    let mut recorded_settings = None;
    if let Some(value) = &receipt {
        pending = value["complete"] != true;
        recorded_settings = Some(Settings::parse(&value["settings"])?);
        for entry in value["files"].as_array().ok_or_else(|| conflict(config))? {
            let path = PathBuf::from(entry["path"].as_str().ok_or_else(|| conflict(config))?);
            absolute(&path)?;
            if !entry["text"].is_string()
                || !entry["owned"].is_boolean()
                || recorded.insert(path, entry).is_some()
            {
                return Err(conflict(config));
            }
            pending |= !entry["pending"].is_null();
        }
    }
    let legacy = receipt
        .as_ref()
        .map(|v| files::legacy_paths(v, config))
        .transpose()?
        .unwrap_or_default();
    let skill_settings = if pending {
        recorded_settings.clone().or_else(|| saved.clone())
    } else {
        saved.clone().or_else(|| recorded_settings.clone())
    };
    let settings = saved.or(recorded_settings);
    let config_text = files::read(config)?;
    let config_state = if let Some(text) = &config_text {
        if recorded
            .get(config)
            .is_some_and(|entry| !entry["pending"].is_null())
        {
            FileState::Pending
        } else if recorded
            .get(config)
            .is_some_and(|entry| entry["text"].as_str() != Some(text))
        {
            FileState::Modified
        } else {
            FileState::Installed
        }
    } else {
        FileState::Missing
    };
    let mut skills = Vec::new();
    let mut mailbox = None;
    let mut complete = config_present && !pending && config_state == FileState::Installed;
    if let Some(settings) = &settings {
        mailbox = Some(DirectoryState::inspect(&settings.mail_home)?);
        complete &= mailbox == Some(DirectoryState::Exists);
    }
    if let Some(skill_settings) = &skill_settings {
        for (host, path) in &skill_settings.skills {
            let skill = observe_skill(host, path, recorded.get(path).copied(), bundle, false);
            complete &= matches!(skill.state, FileState::Installed | FileState::Outdated);
            skills.push(skill);
        }
    }
    for (host, path) in legacy {
        let skill = observe_skill(&host, &path, recorded.get(&path).copied(), bundle, true);
        complete &= matches!(skill.state, FileState::Installed | FileState::Outdated)
            || (skill.ownership == Ownership::Unmanaged
                && matches!(skill.state, FileState::Different | FileState::Missing));
        skills.push(skill);
    }

    let state = if settings.is_none() {
        SetupState::NotConfigured
    } else if complete {
        SetupState::Configured
    } else {
        SetupState::Incomplete
    };
    Ok(Inspection {
        settings,
        config_present,
        config_state,
        state,
        pending,
        skills,
        mailbox,
    })
}

fn observe_skill(
    host: &str,
    path: &Path,
    entry: Option<&serde_json::Value>,
    bundle: Bundle<'_>,
    retained_legacy: bool,
) -> SkillState {
    let ownership = entry.map_or(Ownership::Unmanaged, |entry| {
        if !entry["pending"].is_null() {
            Ownership::Pending
        } else if entry["owned"] == true {
            Ownership::Owned
        } else {
            Ownership::Borrowed
        }
    });
    let text = if path.file_name().is_some_and(|name| name == "SKILL.md") {
        files::read(path)
    } else {
        Err(conflict(path))
    };
    let matches_bundle = text.as_ref().ok().and_then(|text| text.as_deref()) == Some(bundle.skill);
    let state = match text {
        Ok(_) if entry.is_some_and(|entry| !entry["pending"].is_null()) => FileState::Pending,
        Ok(None) => FileState::Missing,
        Ok(Some(text)) => {
            if let Some(entry) = entry {
                if entry["text"].as_str() == Some(&text) {
                    if matches_bundle || retained_legacy {
                        FileState::Installed
                    } else {
                        FileState::Outdated
                    }
                } else {
                    FileState::Modified
                }
            } else if matches_bundle {
                FileState::Installed
            } else {
                FileState::Different
            }
        }
        Err(_) => FileState::Conflict,
    };
    SkillState {
        host: host.into(),
        path: path.into(),
        state,
        ownership,
        matches_bundle,
        retained_legacy,
    }
}
