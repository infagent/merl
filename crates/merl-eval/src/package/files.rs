//! Bounded immutable package snapshots; no later filesystem reads during staging.

use super::{PackageError, require};
use crate::isolation::hex_digest;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    sync::Arc,
};

// Match the provider payload ceiling. The aggregate ceiling bounds retained
// duplicates and audit artifacts without trusting a package's declared sizes.
const FILE_BYTES: usize = 16 * 1024 * 1024;
const PACKAGE_BYTES: usize = 512 * 1024 * 1024;
const FILE_COUNT: usize = 4096;
const PATH_DEPTH: usize = 16;

pub(super) struct Files(BTreeMap<String, Arc<[u8]>>);
impl Files {
    pub(super) fn open(root: &Path, expected_manifest: &str) -> Result<Self, PackageError> {
        let root = fs::canonicalize(root)?;
        require(!root.starts_with("/usr"), "package_under_worker_runtime")?;
        let manifest = read(&root.join("package.sha256"))?;
        require(
            hex_digest(&manifest) == expected_manifest,
            "package_manifest_hash",
        )?;
        let expected = manifest_entries(&manifest)?;
        let mut files = BTreeMap::new();
        let mut pending = vec![(root.clone(), 0)];
        let mut total = 0_usize;
        let mut directories = 0_usize;
        while let Some((directory, depth)) = pending.pop() {
            directories += 1;
            require(directories <= FILE_COUNT, "package_directory_count")?;
            require(depth < PATH_DEPTH, "package_path_depth")?;
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                let metadata = fs::symlink_metadata(&path)?;
                require(!metadata.file_type().is_symlink(), "package_symlink")?;
                if metadata.is_dir() {
                    pending.push((path, depth + 1));
                    continue;
                }
                require(metadata.is_file(), "package_non_regular_file")?;
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|_| PackageError::Integrity("package_path"))?
                    .to_str()
                    .ok_or(PackageError::Integrity("package_path_encoding"))?
                    .to_owned();
                safe_path(&relative)?;
                if relative == "package.sha256" {
                    continue;
                }
                require(files.len() < FILE_COUNT, "package_file_count")?;
                let digest = expected
                    .get(&relative)
                    .ok_or(PackageError::Integrity("unlisted_package_file"))?;
                let bytes = read(&path)?;
                total = total
                    .checked_add(bytes.len())
                    .ok_or(PackageError::Integrity("package_byte_limit"))?;
                require(total <= PACKAGE_BYTES, "package_byte_limit")?;
                require(hex_digest(&bytes) == *digest, "package_file_hash")?;
                files.insert(relative, Arc::from(bytes));
            }
        }
        require(files.len() == expected.len(), "missing_package_file")?;
        Ok(Self(files))
    }
    pub(super) fn bytes(&self, path: &str) -> Result<&[u8], PackageError> {
        safe_path(path)?;
        self.0
            .get(path)
            .map(AsRef::as_ref)
            .ok_or(PackageError::Integrity("unlisted_package_reference"))
    }
    pub(super) fn shared(&self, path: &str) -> Result<Arc<[u8]>, PackageError> {
        self.bytes(path)?;
        self.0
            .get(path)
            .cloned()
            .ok_or(PackageError::Integrity("unlisted_package_reference"))
    }
    pub(super) fn json(&self, path: &str) -> Result<Value, PackageError> {
        serde_json::from_slice(self.bytes(path)?)
            .map_err(|_| PackageError::Integrity("package_json"))
    }
    pub(super) fn hash(&self, path: &str) -> Result<String, PackageError> {
        Ok(hex_digest(self.bytes(path)?))
    }
    pub(super) fn check_hash(&self, path: &str, expected: &Value) -> Result<(), PackageError> {
        require(
            expected.as_str() == Some(self.hash(path)?.as_str()),
            "package_reference_hash",
        )
    }
    pub(super) fn verify_manifest(&self, path: &str) -> Result<(), PackageError> {
        let parent = Path::new(path)
            .parent()
            .and_then(Path::to_str)
            .ok_or(PackageError::Integrity("manifest_parent"))?;
        for (relative, hash) in manifest_entries(self.bytes(path)?)? {
            require(
                self.hash(&format!("{parent}/{relative}"))? == hash,
                "proposal_manifest_hash",
            )?;
        }
        Ok(())
    }
}
fn read(path: &Path) -> Result<Vec<u8>, PackageError> {
    require(
        fs::symlink_metadata(path)?.is_file(),
        "package_non_regular_file",
    )?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)?;
    require(bytes.len() <= FILE_BYTES, "package_file_byte_limit")?;
    Ok(bytes)
}
fn manifest_entries(bytes: &[u8]) -> Result<BTreeMap<String, String>, PackageError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| PackageError::Integrity("manifest_encoding"))?;
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        require(entries.len() < FILE_COUNT, "manifest_file_count")?;
        let (hash, path) = line
            .split_once("  ")
            .ok_or(PackageError::Integrity("manifest_line"))?;
        require(valid_hash(hash), "manifest_digest")?;
        safe_path(path)?;
        require(
            entries.insert(path.to_owned(), hash.to_owned()).is_none(),
            "duplicate_manifest_path",
        )?;
    }
    require(!entries.is_empty(), "empty_manifest")?;
    Ok(entries)
}
pub(super) fn safe_path(path: &str) -> Result<(), PackageError> {
    require(
        !path.is_empty()
            && !path.contains('\\')
            && !path.chars().any(char::is_control)
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "unsafe_package_path",
    )
}
pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(super) fn exact_keys(value: &Value, keys: &[&str]) -> Result<(), PackageError> {
    let object = value
        .as_object()
        .ok_or(PackageError::Integrity("expected_object"))?;
    require(
        object.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == keys.iter().copied().collect(),
        "unexpected_disclosed_fields",
    )
}
