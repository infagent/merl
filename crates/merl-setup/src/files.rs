//! Small generated-file journal. Intent and publication identity survive retries.

use super::{Plan, SetupError, absolute, conflict, failure, input};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

// Primary skills, retained legacy entries, settings and interrupted intent
// are bounded independently; ordinary short skills fit well below this limit.
const MAX_FILE: u64 = 128 * 1024;
const MAX_RECEIPT: u64 = 1024 * 1024;
// Bounded collision slots preserve unrecognized files left before identity commit.
const STAGE_SLOTS: u32 = 32;

#[derive(Debug, Clone)]
struct Entry {
    text: String,
    owned: bool,
    pending: Option<Value>,
}
#[derive(Debug)]
struct Receipt {
    version: u8,
    legacy_skills: BTreeMap<String, PathBuf>,
    entries: BTreeMap<PathBuf, Entry>,
    directories: Vec<(PathBuf, Value)>,
    complete: bool,
}
impl Default for Receipt {
    fn default() -> Self {
        Self {
            version: 2,
            legacy_skills: BTreeMap::new(),
            entries: BTreeMap::new(),
            directories: Vec::new(),
            complete: false,
        }
    }
}

fn receipt_version(value: &Value, path: &Path) -> Result<u8, SetupError> {
    match value["schema"].as_str() {
        Some("merl.init-receipt/v1") => Ok(1),
        Some("merl.init-receipt/v2") => Ok(2),
        _ => Err(conflict(path)),
    }
}

fn known_transition(old: &Path, new: &Path) -> bool {
    old.file_name().is_some_and(|n| n == "SKILL.md")
        && old
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "merl-mail")
        && old
            .parent()
            .and_then(Path::parent)
            .is_some_and(|root| root.join("merl/SKILL.md") == new)
}

pub(super) fn legacy_paths(
    value: &Value,
    config: &Path,
) -> Result<BTreeMap<String, PathBuf>, SetupError> {
    if receipt_version(value, config)? == 1 {
        return Ok(BTreeMap::new());
    }
    let settings = super::Settings::parse(&value["settings"])?;
    let entries = value["legacy_skills"]
        .as_object()
        .ok_or_else(|| conflict(config))?;
    if entries.len() > 2 {
        return Err(conflict(config));
    }
    entries
        .iter()
        .map(|(host, value)| {
            let path = PathBuf::from(value.as_str().ok_or_else(|| conflict(config))?);
            absolute(&path)?;
            if !matches!(host.as_str(), "codex" | "claude")
                || settings
                    .skills
                    .get(host)
                    .is_none_or(|primary| !known_transition(&path, primary))
            {
                return Err(conflict(config));
            }
            Ok((host.clone(), path))
        })
        .collect()
}

fn retained(plan: &Plan<'_>, receipt: &Receipt) -> Result<BTreeMap<String, PathBuf>, SetupError> {
    let mut paths = receipt.legacy_skills.clone();
    let recorded = recorded_settings(&plan.config)?;
    for (host, primary) in &plan.settings.skills {
        if primary
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|n| n != "merl")
        {
            continue;
        }
        let root = primary
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| input("skill path needs a root"))?;
        let legacy = root.join("merl-mail/SKILL.md");
        check_path(&legacy)?;
        let exists = match fs::symlink_metadata(&legacy) {
            Ok(m) if m.is_file() => true,
            Ok(_) => return Err(conflict(&legacy)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        if exists
            || receipt.entries.contains_key(&legacy)
            || recorded
                .as_ref()
                .is_some_and(|s| s.skills.get(host) == Some(&legacy))
        {
            paths.insert(host.clone(), legacy);
        }
    }
    Ok(paths)
}

pub(super) fn read(path: &Path) -> Result<Option<String>, SetupError> {
    read_limit(path, MAX_FILE)
}
fn read_limit(path: &Path, limit: u64) -> Result<Option<String>, SetupError> {
    check_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => return Err(conflict(path)),
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !f.metadata()?.is_file() {
        return Err(conflict(path));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(conflict(path));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| conflict(path))
}
pub(super) fn check_path(path: &Path) -> Result<(), SetupError> {
    absolute(path)?;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_type().is_symlink() => {
                // macOS exposes its system temporary roots through these aliases.
                if cfg!(target_os = "macos") && matches!(ancestor.to_str(), Some("/var" | "/tmp")) {
                    continue;
                }
                return Err(conflict(ancestor));
            }
            Ok(m) if ancestor != path && !m.is_dir() => return Err(conflict(ancestor)),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn receipt_path(plan: &Plan<'_>) -> PathBuf {
    sibling(&plan.config, "merl-init.json")
}
pub(super) fn is_complete(config: &Path) -> Result<bool, SetupError> {
    let path = sibling(config, "merl-init.json");
    let Some(text) = read_limit(&path, MAX_RECEIPT)? else {
        return Ok(read(config)?.is_some());
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| conflict(&path))?;
    receipt_version(&value, &path)?;
    if value["complete"] != true {
        return Ok(false);
    }
    for entry in value["files"].as_array().ok_or_else(|| conflict(&path))? {
        let file = Path::new(entry["path"].as_str().ok_or_else(|| conflict(&path))?);
        if read(file)?.as_deref() != entry["text"].as_str() || !entry["pending"].is_null() {
            return Ok(false);
        }
    }
    Ok(read(config)?.is_some())
}
pub(super) fn recorded_settings(config: &Path) -> Result<Option<super::Settings>, SetupError> {
    let path = sibling(config, "merl-init.json");
    read_limit(&path, MAX_RECEIPT)?
        .map(|s| {
            let value: Value = serde_json::from_str(&s).map_err(|_| conflict(&path))?;
            receipt_version(&value, &path)?;
            super::Settings::parse(&value["settings"])
        })
        .transpose()
}
pub(super) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(
        ".{}.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        suffix
    ))
}

pub(super) fn receipt_value(config: &Path) -> Result<Option<Value>, SetupError> {
    let path = sibling(config, "merl-init.json");
    read_limit(&path, MAX_RECEIPT)?
        .map(|text| {
            let value: Value = serde_json::from_str(&text).map_err(|_| conflict(&path))?;
            receipt_version(&value, &path)?;
            Ok(value)
        })
        .transpose()
}

pub(super) fn changes(
    plan: &Plan<'_>,
    hosts: &[String],
) -> Result<Vec<super::FileChange>, SetupError> {
    use super::{Change, FileChange, Ownership};
    validate(plan)?;
    let receipt = load(plan)?;
    let legacy = retained(plan, &receipt)?;
    let mut wanted = desired(plan);
    if plan.removing {
        wanted.retain(|path, _| {
            path == &plan.config
                || plan.settings.skills.iter().any(|(host, target)| {
                    target == path && (hosts.is_empty() || hosts.contains(host))
                })
        });
        if !hosts.is_empty() {
            let mut remaining = plan.settings.clone();
            remaining.skills.retain(|host, _| !hosts.contains(host));
            wanted.insert(plan.config.clone(), remaining.bytes());
        }
    }
    let ownership = |path: &Path| {
        receipt
            .entries
            .get(path)
            .map_or(Ownership::Unmanaged, |entry| {
                if entry.pending.is_some() {
                    Ownership::Pending
                } else if entry.owned {
                    Ownership::Owned
                } else {
                    Ownership::Borrowed
                }
            })
    };
    let mut changes = Vec::new();
    for (path, text) in wanted {
        let actual = read(&path)?;
        let owned = receipt.entries.get(&path).is_some_and(|entry| entry.owned);
        let removing = plan.removing && (path != plan.config || hosts.is_empty());
        let action = if removing {
            if owned {
                Change::Remove
            } else {
                Change::Retain
            }
        } else if actual.is_none() {
            Change::Create
        } else if actual.as_deref() != Some(&text) && !owned {
            return Err(conflict(&path));
        } else if actual.as_deref() != Some(&text) {
            Change::Update
        } else if owned {
            Change::Unchanged
        } else {
            Change::Borrowed
        };
        let mut recorded_ownership = ownership(&path);
        if action == Change::Borrowed && recorded_ownership == Ownership::Unmanaged {
            recorded_ownership = Ownership::Borrowed;
        }
        let host = plan
            .settings
            .skills
            .iter()
            .find(|(_, target)| *target == &path)
            .map(|(host, _)| host.clone());
        changes.push(FileChange {
            ownership: recorded_ownership,
            path,
            action,
            host,
            retained_legacy: false,
        });
    }
    for (host, path) in legacy {
        if plan.removing && !hosts.is_empty() && !hosts.contains(&host) {
            continue;
        }
        let owner = ownership(&path);
        let action = if plan.removing && owner == Ownership::Owned {
            Change::Remove
        } else {
            Change::Retain
        };
        changes.push(FileChange {
            path,
            action,
            host: Some(host),
            retained_legacy: true,
            ownership: owner,
        });
    }
    Ok(changes)
}

fn load(plan: &Plan<'_>) -> Result<Receipt, SetupError> {
    let p = receipt_path(plan);
    let Some(text) = read_limit(&p, MAX_RECEIPT)? else {
        return Ok(Receipt::default());
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| conflict(&p))?;
    receipt_version(&value, &p)?;
    let mut receipt = Receipt {
        version: receipt_version(&value, &p)?,
        legacy_skills: legacy_paths(&value, &p)?,
        complete: value["complete"] == true,
        ..Receipt::default()
    };
    for item in value["files"].as_array().ok_or_else(|| conflict(&p))? {
        let path = PathBuf::from(item["path"].as_str().ok_or_else(|| conflict(&p))?);
        absolute(&path)?;
        let text = item["text"]
            .as_str()
            .ok_or_else(|| conflict(&p))?
            .to_owned();
        let owned = item["owned"].as_bool().ok_or_else(|| conflict(&p))?;
        let pending = item.get("pending").filter(|v| !v.is_null()).cloned();
        if receipt
            .entries
            .insert(
                path,
                Entry {
                    text,
                    owned,
                    pending,
                },
            )
            .is_some()
        {
            return Err(conflict(&p));
        }
    }
    for item in value["directories"]
        .as_array()
        .ok_or_else(|| conflict(&p))?
    {
        let path = PathBuf::from(item["path"].as_str().ok_or_else(|| conflict(&p))?);
        absolute(&path)?;
        if !item["identity"].is_array() {
            return Err(conflict(&p));
        }
        receipt.directories.push((path, item["identity"].clone()));
    }
    // A completed publication is recognizable by inode, not matching text alone.
    let paths = receipt.entries.keys().cloned().collect::<Vec<_>>();
    for path in paths {
        let entry = receipt.entries.get_mut(&path).expect("loaded entry");
        if let Some(pending) = &entry.pending {
            let current = read(&path)?;
            if pending["remove"] == true {
                if current.is_none() {
                    receipt.entries.remove(&path);
                }
            } else if current.as_deref() == pending["text"].as_str()
                && identity(&path)?.as_ref() == pending.get("identity")
                && !pending["identity"].is_null()
            {
                entry.text = pending["text"].as_str().ok_or_else(|| conflict(&p))?.into();
                entry.owned = true;
            }
        }
    }
    Ok(receipt)
}
fn save(plan: &Plan<'_>, receipt: &Receipt) -> Result<(), SetupError> {
    let files = receipt
        .entries
        .iter()
        .map(|(path, e)| json!({"path":path,"text":e.text,"owned":e.owned,"pending":e.pending}))
        .collect::<Vec<_>>();
    let mut value = json!({"schema":if receipt.version == 1 {"merl.init-receipt/v1"}else{"merl.init-receipt/v2"},"version":plan.bundle.version,"complete":receipt.complete,"settings":plan.settings.value(),"files":files,"directories":receipt.directories.iter().map(|(path,id)|json!({"path":path,"identity":id})).collect::<Vec<_>>()});
    if receipt.version == 2 {
        value["legacy_skills"] = json!(receipt.legacy_skills);
    }
    let text = format!("{value}\n");
    if text.len() as u64 > MAX_RECEIPT {
        return Err(input("setup receipt exceeds its bounded capacity"));
    }
    let p = receipt_path(plan);
    let (stage, mut f) = stage_file(&p)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&stage, &p)?;
    sync_parent(&p)
}
fn desired(plan: &Plan<'_>) -> BTreeMap<PathBuf, String> {
    let mut desired = BTreeMap::from([(plan.config.clone(), plan.settings.bytes())]);
    for path in plan.settings.skills.values() {
        desired.insert(path.clone(), plan.bundle.skill.into());
    }
    desired
}
fn comparison_path(path: &Path) -> Result<PathBuf, SetupError> {
    let ancestor = path
        .ancestors()
        .find(|p| p.exists())
        .ok_or_else(|| input("path has no existing filesystem root"))?;
    Ok(ancestor.canonicalize()?.join(
        path.strip_prefix(ancestor)
            .map_err(|_| input("invalid path ancestry"))?,
    ))
}

fn validate_transition(plan: &Plan<'_>, receipt: &Receipt) -> Result<(), SetupError> {
    if !plan.removing
        && receipt.version == 1
        && (!receipt.complete || receipt.entries.values().any(|e| e.pending.is_some()))
    {
        let quoted = format!(
            "'{}'",
            plan.config.display().to_string().replace('\'', "'\\''")
        );
        return Err(input(&format!(
            "Legacy setup is incomplete; no upgrade was written. Use merl init --config {quoted} --uninstall --yes, then retry init. Alternatively finish setup with the original compatible initializer."
        )));
    }
    let recorded = recorded_settings(&plan.config)?;
    if plan.settings.skills.iter().any(|(host, path)| {
        recorded.as_ref().is_some_and(|old| {
            old.skills.get(host).is_some_and(|old_path| {
                old_path != path && (plan.removing || !known_transition(old_path, path))
            })
        })
    }) {
        return Err(input(
            "uninstall a host before moving its recorded skill destination",
        ));
    }
    Ok(())
}

pub(super) fn validate(plan: &Plan<'_>) -> Result<(), SetupError> {
    absolute(&plan.settings.mail_home)?;
    check_path(&plan.settings.mail_home)?;
    if plan.settings.mail_home.exists() && !plan.settings.mail_home.is_dir() {
        return Err(conflict(&plan.settings.mail_home));
    }
    let receipt = load(plan)?;
    validate_transition(plan, &receipt)?;
    let legacy = retained(plan, &receipt)?;
    let mut paths = vec![
        plan.config.clone(),
        receipt_path(plan),
        sibling(&plan.config, "merl-init.lock"),
    ];
    for (host, path) in &plan.settings.skills {
        if !matches!(host.as_str(), "codex" | "claude")
            || path.file_name().is_none_or(|n| n != "SKILL.md")
        {
            return Err(input("supported hosts install a SKILL.md file"));
        }
        paths.push(path.clone());
    }
    paths.extend(legacy.values().cloned());
    let mail = comparison_path(&plan.settings.mail_home)?;
    for (index, path) in paths.iter().enumerate() {
        check_path(path)?;
        let resolved = comparison_path(path)?;
        // Setup files cannot contain or sit beneath the mailbox directory.
        // A shared ancestor is safe: directory cleanup separately excludes mail.
        if resolved.starts_with(&mail) || mail.starts_with(&resolved) {
            return Err(input("setup files and mailbox home must not overlap"));
        }
        for other in &paths[index + 1..] {
            let other = comparison_path(other)?;
            if resolved.starts_with(&other) || other.starts_with(&resolved) {
                return Err(input("managed setup destinations overlap"));
            }
        }
    }
    // Retained entries also participate in overlap and content checks.
    for path in receipt.entries.keys() {
        let resolved = comparison_path(path)?;
        if resolved.starts_with(&mail) || mail.starts_with(&resolved) {
            return Err(input("recorded setup path overlaps mailbox home"));
        }
        for target in &paths {
            let other = comparison_path(target)?;
            if path != target && (resolved.starts_with(&other) || other.starts_with(&resolved)) {
                return Err(input("recorded setup destinations overlap"));
            }
        }
    }
    for (path, entry) in &receipt.entries {
        let actual = read(path)?;
        if let Some(pending) = &entry.pending {
            validate_stage(path, pending)?;
            // An absent new file is retryable; a foreign identical publication
            // remains borrowed rather than inheriting interrupted ownership.
            if actual.is_none() && entry.text.is_empty() {
                continue;
            }
            if !entry.owned
                && entry.text.is_empty()
                && actual.as_deref() == pending["text"].as_str()
            {
                continue;
            }
        }
        if actual.as_deref() != Some(&entry.text) {
            return Err(conflict(path));
        }
    }
    for (path, text) in desired(plan).into_iter().filter(|_| !plan.removing) {
        if text.len() as u64 > MAX_FILE {
            return Err(input("generated setup file exceeds its bounded capacity"));
        }
        if let Some(actual) = read(&path)? {
            let owned = receipt.entries.get(&path).is_some_and(|e| e.owned);
            if actual != text && !owned {
                return Err(conflict(&path));
            }
        }
    }
    Ok(())
}
fn create_directories(path: &Path, receipt: &mut Receipt) -> Result<(), SetupError> {
    check_path(path)?;
    let mut missing = Vec::new();
    let mut p = path;
    while !p.exists() {
        missing.push(p.to_path_buf());
        p = p.parent().ok_or_else(|| input("invalid directory"))?;
    }
    for p in missing.into_iter().rev() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&p)?;
        receipt.directories.push((
            p.clone(),
            identity(&p)?.expect("created directory identity"),
        ));
    }
    Ok(())
}
fn lock(plan: &Plan<'_>, receipt: &mut Receipt) -> Result<File, SetupError> {
    create_directories(
        plan.config
            .parent()
            .ok_or_else(|| input("config needs a parent"))?,
        receipt,
    )?;
    let path = sibling(&plan.config, "merl-init.lock");
    check_path(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.try_lock()
        .map_err(|_| failure("INIT_BUSY", "Another initializer holds this settings lock"))?;
    Ok(file)
}
fn stage_file(path: &Path) -> Result<(PathBuf, File), SetupError> {
    for n in 0..STAGE_SLOTS {
        let stage = sibling(path, &format!("merl-stage-{n}"));
        check_path(&stage)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&stage) {
            Ok(f) => return Ok((stage, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    Err(failure(
        "INIT_CONFLICT",
        "All setup staging slots are occupied; preserve and inspect the reported directory before retrying",
    ))
}
fn identity(path: &Path) -> Result<Option<Value>, SetupError> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(json!([m.dev(), m.ino()])))
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        Err(input(
            "setup publication requires a Unix filesystem identity",
        ))
    }
}
fn sync_parent(path: &Path) -> Result<(), SetupError> {
    File::open(path.parent().ok_or_else(|| input("path needs a parent"))?)?.sync_all()?;
    Ok(())
}

fn validate_stage(path: &Path, pending: &Value) -> Result<(), SetupError> {
    if let Some(stage) = pending["stage"].as_str().map(PathBuf::from) {
        if !(0..STAGE_SLOTS).any(|n| stage == sibling(path, &format!("merl-stage-{n}"))) {
            return Err(conflict(&stage));
        }
        check_path(&stage)?;
    }
    Ok(())
}
fn cleanup_stage(path: &Path, pending: &Value) -> Result<(), SetupError> {
    validate_stage(path, pending)?;
    let Some(stage) = pending["stage"].as_str().map(PathBuf::from) else {
        return Ok(());
    };
    check_path(&stage)?;
    if identity(&stage)?.as_ref() == pending.get("identity")
        && read(&stage)?.as_deref() == pending["text"].as_str()
    {
        fs::remove_file(&stage)?;
        sync_parent(&stage)?;
    }
    Ok(())
}

fn publish(
    plan: &Plan<'_>,
    receipt: &mut Receipt,
    path: &Path,
    text: &str,
    published: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<bool, SetupError> {
    if let Some(pending) = receipt
        .entries
        .get(path)
        .and_then(|entry| entry.pending.as_ref())
    {
        cleanup_stage(path, pending)?;
    }
    if let Some(actual) = read(path)?
        && actual == text
    {
        let entry = receipt.entries.entry(path.into()).or_insert(Entry {
            text: text.into(),
            owned: false,
            pending: None,
        });
        // A same-content foreign file following interrupted creation is borrowed.
        if entry.text.is_empty() {
            entry.text = text.into();
            entry.owned = false;
        }
        entry.pending = None;
        save(plan, receipt)?;
        return Ok(false);
    }
    let old = read(path)?;
    if let Some(old) = &old
        && !receipt
            .entries
            .get(path)
            .is_some_and(|e| e.owned && &e.text == old)
    {
        return Err(conflict(path));
    }
    // Record intent before creating directories or generated file bytes.
    let entry = receipt.entries.entry(path.into()).or_insert(Entry {
        text: String::new(),
        owned: false,
        pending: None,
    });
    entry.pending = Some(json!({"text":text}));
    save(plan, receipt)?;
    create_directories(
        path.parent().ok_or_else(|| input("file needs a parent"))?,
        receipt,
    )?;
    save(plan, receipt)?;
    let (stage, mut f) = stage_file(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    receipt
        .entries
        .get_mut(path)
        .expect("recorded intent")
        .pending = Some(json!({"text":text,"identity":identity(&stage)?,"stage":stage}));
    save(plan, receipt)?;
    // Revalidate immediately before publication. Never replace a borrowed file.
    if read(path)? != old {
        return Err(conflict(path));
    }
    if old.is_none() {
        fs::hard_link(&stage, path)?;
        fs::remove_file(&stage)?;
    } else {
        fs::rename(&stage, path)?;
    }
    sync_parent(path)?;
    published(path)?;
    receipt.entries.insert(
        path.into(),
        Entry {
            text: text.into(),
            owned: true,
            pending: None,
        },
    );
    save(plan, receipt)?;
    Ok(true)
}
pub(super) fn apply(
    plan: &Plan<'_>,
    published: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<bool, SetupError> {
    validate(plan)?;
    let mut initial = Receipt::default();
    let _guard = lock(plan, &mut initial)?;
    validate(plan)?;
    let mut receipt = load(plan)?;
    receipt.legacy_skills = retained(plan, &receipt)?;
    receipt.version = 2;
    receipt.directories.extend(initial.directories);
    receipt.complete = false;
    save(plan, &receipt)?;
    create_directories(&plan.settings.mail_home, &mut Receipt::default())?;
    let mut changed = false;
    // Skills first: configured defaults are published only after skill setup succeeds.
    let mut files = desired(plan);
    let config = files.remove(&plan.config).expect("config generated");
    for (path, text) in files
        .into_iter()
        .chain(std::iter::once((plan.config.clone(), config)))
    {
        if publish(plan, &mut receipt, &path, &text, published)? {
            changed = true;
        }
    }
    receipt.complete = true;
    save(plan, &receipt)?;
    Ok(changed)
}
pub(super) fn uninstall(
    plan: &Plan<'_>,
    hosts: &[String],
    published: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<bool, SetupError> {
    validate(plan)?;
    if read_limit(&receipt_path(plan), MAX_RECEIPT)?.is_none() {
        return Ok(false);
    }
    let mut initial = Receipt::default();
    let _guard = lock(plan, &mut initial)?;
    validate(plan)?;
    let mut receipt = load(plan)?;
    receipt.complete = false;
    let all = hosts.is_empty();
    let mut selected = Vec::new();
    for (host, path) in &plan.settings.skills {
        if all || hosts.contains(host) {
            selected.push(path.clone());
            if let Some(legacy) = receipt.legacy_skills.get(host) {
                selected.push(legacy.clone());
            }
        }
    }
    if all {
        selected.push(plan.config.clone());
    }
    let mut remaining = plan.settings.clone();
    remaining.skills.retain(|h, _| !hosts.contains(h));
    if !all
        && !receipt
            .entries
            .get(&plan.config)
            .is_some_and(|entry| entry.owned)
        && read(&plan.config)?.as_deref() != Some(&remaining.bytes())
    {
        return Err(conflict(&plan.config));
    }
    let mut changed = false;
    for path in selected {
        if let Some(entry) = receipt.entries.get_mut(&path) {
            if entry.owned {
                if read(&path)?.as_deref() != Some(&entry.text) {
                    return Err(conflict(&path));
                }
                entry.pending = Some(json!({"remove":true}));
                save(plan, &receipt)?;
                fs::remove_file(&path)?;
                sync_parent(&path)?;
                changed = true;
            }
            receipt.entries.remove(&path);
            save(plan, &receipt)?;
            published(&path)?;
        }
    }
    if !all {
        changed |= publish(
            plan,
            &mut receipt,
            &plan.config,
            &remaining.bytes(),
            published,
        )?;
        receipt.complete = true;
        receipt
            .legacy_skills
            .retain(|host, _| !hosts.contains(host));
        let remaining_plan = Plan {
            config: plan.config.clone(),
            settings: remaining,
            bundle: plan.bundle,
            removing: true,
        };
        save(&remaining_plan, &receipt)?;
    }
    if all {
        receipt.legacy_skills.clear();
    }
    if all && receipt.entries.is_empty() {
        fs::remove_file(receipt_path(plan))?;
    }
    let mail = comparison_path(&plan.settings.mail_home)?;
    for (dir, id) in receipt.directories.iter().rev() {
        let resolved = comparison_path(dir)?;
        if !resolved.starts_with(&mail) && !mail.starts_with(&resolved) {
            check_path(dir)?;
            if identity(dir)?.as_ref() != Some(id) {
                continue;
            }
            match fs::remove_dir(dir) {
                Ok(()) => (),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(changed)
}
