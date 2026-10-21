//! Receipt-bound visible question packages. Frozen answer objects stay opaque
//! to readers; the effective approval overlay supplies the scoring denominator.

use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::Path,
    sync::Arc,
};
mod files;
use files::{Files, exact_keys, valid_hash};

const INVENTORY: &str = "evaluator-only/approved-artifacts/question-inventory.json";
const RULES: &str = "evaluator-only/approved-artifacts/scoring-rules.json";
const RECEIPT: &str = "approval/human-approval-original.json";
const INDEX: &str = "evaluator-only/unit-index.json";
const READER_MANIFEST: &str = "reader-inputs/reader-view-manifest.json";
const APPROVAL: &str = "human_approved_for_first_visible_benchmark_pass";
const READER_NAMES: [&str; 4] = [
    "prompt.json",
    "source.json",
    "method.json",
    "instructions.txt",
];
const GRADER_NAMES: [&str; 5] = [
    "prompt.json",
    "source.json",
    "method.json",
    "oracle.json",
    "grading-instructions.txt",
];

/// Out-of-band digests and the approved question-cutoff denominator.
#[derive(Clone, Debug)]
pub struct PackagePin {
    manifest: String,
    inventory: String,
    rules: String,
    units: usize,
}
impl PackagePin {
    /// Checks an evaluator's independently supplied package identity.
    ///
    /// # Errors
    /// Rejects malformed digests or an empty denominator. Hex digests may carry
    /// the conventional `sha256:` prefix on this command/API boundary.
    pub fn new(
        manifest: &str,
        inventory: &str,
        rules: &str,
        units: usize,
    ) -> Result<Self, PackageError> {
        let parse = |value: &str| {
            let value = value.strip_prefix("sha256:").unwrap_or(value);
            require(valid_hash(value), "expected_digest")?;
            Ok::<_, PackageError>(value.to_owned())
        };
        require(units > 0, "empty_approved_inventory")?;
        Ok(Self {
            manifest: parse(manifest)?,
            inventory: parse(inventory)?,
            rules: parse(rules)?,
            units,
        })
    }
}

/// Exact provenance of a verified package; these are byte hashes, not JSON hashes.
#[derive(Clone, Debug, Serialize)]
pub struct PackageIdentity {
    /// Package version selected by its owner.
    pub version: String,
    /// Out-of-band pinned package checksum manifest.
    pub manifest_sha256: String,
    /// Unchanged human approval receipt.
    pub receipt_sha256: String,
    /// Unchanged source question/answer inventory.
    pub inventory_sha256: String,
    /// Unchanged categorical rubric.
    pub rules_sha256: String,
}

/// Verified immutable package bytes. Debug never renders answers or source prose.
pub struct VerifiedPackage {
    identity: PackageIdentity,
    units: Vec<VerifiedUnit>,
    prefix_count: usize,
    exclusions: Vec<String>,
}
impl fmt::Debug for VerifiedPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedPackage")
            .field("identity", &self.identity)
            .field("unit_count", &self.units.len())
            .field("prefix_count", &self.prefix_count)
            .finish_non_exhaustive()
    }
}
impl VerifiedPackage {
    /// Verifies hashes, approval bindings, exact prompts, oracle copies and prefixes.
    ///
    /// File bytes are snapshotted before use. No source body or answer is included
    /// in validation diagnostics. Proposal-time pending flags are historical;
    /// receipt-bound scope determines the effective approved units.
    ///
    /// # Errors
    /// Rejects unpinned, altered, incomplete, unsafe or internally inconsistent packages.
    pub fn open(root: &Path, pin: &PackagePin) -> Result<Self, PackageError> {
        let files = Files::open(root, &pin.manifest)?;
        require(
            files.hash(INVENTORY)? == pin.inventory && files.hash(RULES)? == pin.rules,
            "approved_artifact_pin",
        )?;
        let Bindings {
            version,
            index,
            scope,
            inventory,
            rules,
            manifest,
        } = approval_bindings(&files)?;
        let expected = inventory_answers(&inventory)?;
        let entries = array(&index, "units")?;
        require(entries.len() == pin.units, "approved_unit_denominator")?;
        require(
            scope["historical_metadata_overlay"]["effective_approved_inventory_denominator"]
                .as_u64()
                == Some(pin.units as u64),
            "effective_approved_denominator",
        )?;
        require(
            array(&scope, "unit_ids")?
                == &entries
                    .iter()
                    .map(|u| u["unit_id"].clone())
                    .collect::<Vec<_>>(),
            "approved_unit_list",
        )?;
        require(entries.len() == expected.len(), "missing_approved_unit")?;
        require(
            array(&manifest, "units")?.len() == entries.len(),
            "reader_manifest_units",
        )?;
        let mut units = Vec::new();
        let mut seen = BTreeSet::new();
        let mut prefixes = BTreeMap::<(String, u64), Value>::new();
        let mut questions = BTreeSet::new();
        let mut cases = BTreeSet::new();
        for (entry, reader_entry) in entries.iter().zip(array(&manifest, "units")?) {
            let id = text(entry, "unit_id")?;
            let qid = text(entry, "question_id")?;
            let case = text(entry, "case")?;
            let cutoff = number(entry, "cutoff")?;
            require(
                !qid.is_empty()
                    && !qid.contains('@')
                    && cutoff > 0
                    && id == format!("{qid}@{cutoff}"),
                "unit_identity",
            )?;
            require(seen.insert(id.to_owned()), "duplicate_unit")?;
            let (expected_case, answer) = expected
                .get(id)
                .ok_or(PackageError::Integrity("unapproved_unit"))?;
            require(case == expected_case, "unit_case")?;
            let unit = read_unit(
                &files,
                &version,
                &inventory,
                &rules,
                entry,
                reader_entry,
                answer,
            )?;
            let key = (case.to_owned(), cutoff);
            if let Some(prior) = prefixes.insert(key, unit.source.clone()) {
                require(prior == unit.source, "shared_prefix_changed")?;
            }
            questions.insert(qid.to_owned());
            cases.insert(case.to_owned());
            units.push(unit);
        }
        require(
            expected.keys().all(|id| seen.contains(id)),
            "missing_approved_unit",
        )?;
        validate_counts(
            &version, &scope, &index, &units, &prefixes, &questions, &cases,
        )?;
        let exclusions = array(&inventory, "excluded_components")?
            .iter()
            .map(|value| text(value, "id").map(ToOwned::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            identity: PackageIdentity {
                version: text(&version, "package_version")?.to_owned(),
                manifest_sha256: pin.manifest.clone(),
                receipt_sha256: files.hash(RECEIPT)?,
                inventory_sha256: pin.inventory.clone(),
                rules_sha256: pin.rules.clone(),
            },
            units,
            prefix_count: prefixes.len(),
            exclusions,
        })
    }
    /// Approved units in exact index order; repeated question IDs remain separate.
    #[must_use]
    pub fn units(&self) -> &[VerifiedUnit] {
        &self.units
    }
    /// Distinct case/cutoff evidence prefixes, independent of scoring units.
    #[must_use]
    pub fn prefix_count(&self) -> usize {
        self.prefix_count
    }
    /// Byte-level provenance retained in every semantic report.
    #[must_use]
    pub fn identity(&self) -> &PackageIdentity {
        &self.identity
    }
    /// Audit-only exclusion IDs; these never add scoring units or score credit.
    #[must_use]
    pub fn exclusion_ids(&self) -> &[String] {
        &self.exclusions
    }
}

/// One approved question at one cutoff, with immutable role-specific inputs.
pub struct VerifiedUnit {
    id: String,
    question_id: String,
    case: String,
    cutoff: u64,
    question: String,
    source: Value,
    reader: BTreeMap<String, Arc<[u8]>>,
    grader: BTreeMap<String, Arc<[u8]>>,
}
impl fmt::Debug for VerifiedUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedUnit")
            .field("unit_id", &self.id)
            .field("cutoff", &self.cutoff)
            .finish_non_exhaustive()
    }
}
impl VerifiedUnit {
    /// Exact `question_id@cutoff` identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Original question identity, without its cutoff suffix.
    #[must_use]
    pub fn question_id(&self) -> &str {
        &self.question_id
    }
    /// Case whose evidence prefix this unit uses.
    #[must_use]
    pub fn case(&self) -> &str {
        &self.case
    }
    /// Verbatim per-cutoff prompt, without the descriptive inventory title.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }
    /// Inclusive observation cutoff.
    #[must_use]
    pub fn cutoff(&self) -> u64 {
        self.cutoff
    }
    /// Original source-prefix JSON values for trusted method preparation.
    #[must_use]
    pub fn source(&self) -> &Value {
        &self.source
    }
    /// Exactly the four verified files for trusted preparation, not a model mount.
    #[must_use]
    pub fn reader_files(&self) -> BTreeMap<String, Vec<u8>> {
        copy_files(&self.reader)
    }
    pub(crate) fn reader_policy(&self) -> Result<(&str, &str), PackageError> {
        let decode = |bytes| {
            std::str::from_utf8(bytes)
                .map_err(|_| PackageError::Integrity("reader_policy_encoding"))
        };
        Ok((
            decode(&self.reader["instructions.txt"])?,
            decode(&self.reader["method.json"])?,
        ))
    }
    /// Exactly this unit's five grader files, excluding all other units and audits.
    #[must_use]
    pub fn grader_files(&self) -> BTreeMap<String, Vec<u8>> {
        copy_files(&self.grader)
    }
}
fn copy_files(files: &BTreeMap<String, Arc<[u8]>>) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .map(|(name, bytes)| (name.clone(), bytes.to_vec()))
        .collect()
}

/// Typed integrity failures, without source text, oracle values, or response excerpts.
#[derive(Debug)]
pub enum PackageError {
    /// A structural, hash, approval or evidence invariant failed; the code is stable.
    Integrity(&'static str),
    /// The local package could not be read.
    Io(std::io::Error),
}
impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integrity(code) => write!(f, "visible package integrity failure: {code}"),
            Self::Io(_) => f.write_str("visible package I/O failure"),
        }
    }
}
impl Error for PackageError {}
impl From<std::io::Error> for PackageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn require(test: bool, code: &'static str) -> Result<(), PackageError> {
    if test {
        Ok(())
    } else {
        Err(PackageError::Integrity(code))
    }
}
fn schema(value: &Value, expected: &'static str) -> Result<(), PackageError> {
    require(value["schema"] == expected, "unsupported_schema")
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, PackageError> {
    value[key]
        .as_str()
        .ok_or(PackageError::Integrity("expected_string"))
}
fn number(value: &Value, key: &str) -> Result<u64, PackageError> {
    value[key]
        .as_u64()
        .ok_or(PackageError::Integrity("expected_unsigned_integer"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, PackageError> {
    value[key]
        .as_array()
        .ok_or(PackageError::Integrity("expected_array"))
}
fn json_bytes(bytes: &[u8]) -> Result<Value, PackageError> {
    serde_json::from_slice(bytes).map_err(|_| PackageError::Integrity("package_json"))
}
fn inventory_answers(
    inventory: &Value,
) -> Result<BTreeMap<String, (String, &Value)>, PackageError> {
    let mut answers = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for question in array(inventory, "questions")? {
        let id = text(question, "id")?;
        require(ids.insert(id), "duplicate_question")?;
        let mut cutoffs = BTreeSet::new();
        for answer in array(question, "answers")? {
            let cutoff = number(answer, "cutoff")?;
            require(cutoffs.insert(cutoff), "duplicate_question_cutoff")?;
            require(
                answers
                    .insert(
                        format!("{id}@{cutoff}"),
                        (text(question, "case")?.to_owned(), answer),
                    )
                    .is_none(),
                "duplicate_inventory_unit",
            )?;
        }
        let declared = array(question, "cutoffs")?
            .iter()
            .map(|c| c.as_u64().ok_or(PackageError::Integrity("cutoff")))
            .collect::<Result<BTreeSet<_>, _>>()?;
        require(cutoffs == declared, "inventory_cutoff_list")?;
    }
    Ok(answers)
}
fn allowlist(
    files: &Files,
    unit: &Value,
    field: &str,
    destination: &str,
    expected: &[&str],
) -> Result<BTreeMap<String, Arc<[u8]>>, PackageError> {
    let mut result = BTreeMap::new();
    for entry in array(unit, field)? {
        let name = text(entry, destination)?;
        require(expected.contains(&name), "allowlist_destination")?;
        let path = text(entry, "package_path")?;
        files.check_hash(path, &entry["sha256"])?;
        require(
            result
                .insert(name.to_owned(), files.shared(path)?)
                .is_none(),
            "duplicate_allowlist_destination",
        )?;
    }
    require(result.len() == expected.len(), "incomplete_allowlist")?;
    Ok(result)
}
fn validate_prefix(
    source: &Value,
    case: &str,
    cutoff: u64,
    answer: &Value,
) -> Result<(), PackageError> {
    schema(source, "merl.visible-source-prefix/v1")?;
    exact_keys(
        source,
        &[
            "schema",
            "case",
            "source_observation_cutoff",
            "source_identity",
            "history_fidelity",
            "observations",
        ],
    )?;
    require(
        source["case"] == case && source["source_observation_cutoff"] == cutoff,
        "source_prefix_identity",
    )?;
    let observations = array(source, "observations")?;
    require(observations.len() as u64 == cutoff, "source_prefix_length")?;
    for (index, observation) in observations.iter().enumerate() {
        require(
            observation["sequence"].as_u64() == Some((index as u64) + 1),
            "future_or_reordered_observation",
        )?;
        if let Some(body) = observation["body"].as_str() {
            require(
                observation["body_sha256"]
                    == format!("sha256:{}", crate::isolation::hex_digest(body.as_bytes())),
                "source_body_hash",
            )?;
        } else {
            require(
                observation["body"].is_null() && observation["body_sha256"].is_null(),
                "missing_body_hash",
            )?;
        }
        if let Some(prior) = observation["supersedes"].as_u64() {
            require(prior > 0 && prior <= index as u64, "future_supersession")?;
        }
    }
    for evidence in array(answer, "evidence")? {
        let sequence = number(evidence, "observation")?;
        require(sequence > 0 && sequence <= cutoff, "future_oracle_evidence")?;
        let index =
            usize::try_from(sequence - 1).map_err(|_| PackageError::Integrity("evidence_index"))?;
        let source = &observations[index];
        require(
            evidence["case"] == case && evidence["version_id"] == source["version_id"],
            "oracle_evidence_identity",
        )?;
        if evidence.get("body_sha256").is_some() {
            require(
                evidence["body_sha256"] == source["body_sha256"],
                "oracle_evidence_hash",
            )?;
        }
    }
    Ok(())
}

struct Bindings {
    version: Value,
    index: Value,
    scope: Value,
    inventory: Value,
    rules: Value,
    manifest: Value,
}
fn approval_bindings(files: &Files) -> Result<Bindings, PackageError> {
    let version = files.json("version.json")?;
    schema(&version, "merl.visible-benchmark-package/v1")?;
    for (field, path) in [
        ("receipt_sha256", RECEIPT),
        ("approved_inventory_sha256", INVENTORY),
        ("approved_rules_sha256", RULES),
        ("unit_index_sha256", INDEX),
        ("reader_view_manifest_sha256", READER_MANIFEST),
    ] {
        files.check_hash(path, &version[field])?;
    }
    let receipt = files.json(RECEIPT)?;
    schema(&receipt, "merl.planner-approval-receipt/v1")?;
    require(receipt["approval"] == APPROVAL, "receipt_approval")?;
    let artifacts = receipt["artifact_sha256"]
        .as_object()
        .ok_or(PackageError::Integrity("receipt_artifacts"))?;
    for (name, hash) in artifacts {
        files.check_hash(&format!("evaluator-only/approved-artifacts/{name}"), hash)?;
    }
    for name in [
        "question-inventory.json",
        "scoring-rules.json",
        "proposal-v2.sha256",
    ] {
        require(artifacts.contains_key(name), "missing_receipt_artifact")?;
    }
    files.verify_manifest("evaluator-only/approved-artifacts/proposal-v2.sha256")?;
    let index = files.json(INDEX)?;
    schema(&index, "merl.visible-approved-unit-index/v1")?;
    let scope_path = text(&index, "approved_scope_file")?;
    files.check_hash(scope_path, &index["approved_scope_sha256"])?;
    let scope = files.json(scope_path)?;
    schema(&scope, "merl.visible-approved-scope/v1")?;
    files.check_hash(RECEIPT, &scope["receipt_sha256"])?;
    require(
        scope["approval"] == APPROVAL && scope["further_blanket_approval_required"] == false,
        "effective_approval",
    )?;
    let inventory = files.json(INVENTORY)?;
    schema(&inventory, "merl.visible-question-inventory/proposal-v2")?;
    let rules = files.json(RULES)?;
    schema(&rules, "merl.visible-scoring-rules/proposal-v2")?;
    require(
        inventory["version"] == rules["version"]
            && rules["version"] == receipt["version"]
            && scope["approved_proposal_version"] == receipt["version"]
            && version["approved_inventory_version"] == receipt["version"],
        "approved_version",
    )?;
    require(
        inventory["matching_rules"] == rules["matching_rules"]
            && inventory["scoring_rules"] == rules["scoring_rules"],
        "rubric_copy",
    )?;
    let manifest = files.json(READER_MANIFEST)?;
    schema(&manifest, "merl.visible-reader-view-manifest/v1")?;
    require(
        version["package_version"] == index["version"]
            && index["version"] == scope["version"]
            && scope["version"] == manifest["version"],
        "package_version",
    )?;

    Ok(Bindings {
        version,
        index,
        scope,
        inventory,
        rules,
        manifest,
    })
}

fn read_unit(
    files: &Files,
    version: &Value,
    inventory: &Value,
    rules: &Value,
    entry: &Value,
    reader_entry: &Value,
    answer: &Value,
) -> Result<VerifiedUnit, PackageError> {
    let id = text(entry, "unit_id")?;
    let qid = text(entry, "question_id")?;
    let case = text(entry, "case")?;
    let cutoff = number(entry, "cutoff")?;
    for field in [
        "unit_id",
        "question_id",
        "case",
        "cutoff",
        "reader_allowlist",
    ] {
        require(
            entry[field] == reader_entry[field],
            "reader_manifest_mapping",
        )?;
    }
    let reader = allowlist(
        files,
        entry,
        "reader_allowlist",
        "reader_destination",
        &READER_NAMES,
    )?;
    let grader = allowlist(
        files,
        entry,
        "grader_static_allowlist",
        "grader_destination",
        &GRADER_NAMES,
    )?;
    for item in array(entry, "reader_allowlist")? {
        require(
            text(item, "package_path")?.starts_with("reader-inputs/"),
            "reader_scope",
        )?;
    }
    for bytes in [
        &reader["instructions.txt"],
        &grader["grading-instructions.txt"],
    ] {
        std::str::from_utf8(bytes).map_err(|_| PackageError::Integrity("instructions_encoding"))?;
    }
    for name in ["prompt.json", "source.json", "method.json"] {
        require(reader[name] == grader[name], "grader_reader_input_mismatch")?;
    }
    let oracle_path = text(entry, "evaluator_oracle")?;
    files.check_hash(oracle_path, &entry["evaluator_oracle_sha256"])?;
    require(
        grader["oracle.json"].as_ref() == files.bytes(oracle_path)?,
        "grader_oracle_mapping",
    )?;
    let oracle = files.json(oracle_path)?;
    schema(&oracle, "merl.visible-approved-oracle-unit/v1")?;
    files.check_hash(RECEIPT, &oracle["approval_receipt_sha256"])?;
    files.check_hash(INVENTORY, &oracle["source_inventory_sha256"])?;
    require(
        oracle["approved_answer"] == *answer,
        "approved_answer_changed",
    )?;
    require(
        oracle["unit_id"] == id
            && oracle["question_id"] == qid
            && oracle["case"] == case
            && oracle["source_observation_cutoff"] == cutoff
            && oracle["approval"] == APPROVAL
            && oracle["version"] == version["package_version"],
        "oracle_identity",
    )?;
    let prompt = validate_prompt(entry, &reader["prompt.json"], answer)?;
    validate_method(files, &reader["method.json"], version, inventory, rules)?;
    let source = json_bytes(&reader["source.json"])?;
    validate_prefix(&source, case, cutoff, answer)?;
    Ok(VerifiedUnit {
        id: id.to_owned(),
        question_id: qid.to_owned(),
        case: case.to_owned(),
        cutoff,
        question: text(&prompt, "question")?.to_owned(),
        source,
        reader,
        grader,
    })
}

fn validate_prompt(entry: &Value, bytes: &[u8], answer: &Value) -> Result<Value, PackageError> {
    let id = text(entry, "unit_id")?;
    let qid = text(entry, "question_id")?;
    let case = text(entry, "case")?;
    let cutoff = number(entry, "cutoff")?;
    let prompt = json_bytes(bytes)?;
    schema(&prompt, "merl.visible-reader-question/v1")?;
    exact_keys(
        &prompt,
        &[
            "schema",
            "unit_id",
            "question_id",
            "case",
            "source_observation_cutoff",
            "question",
            "source_file",
            "method_file",
            "instructions_file",
        ],
    )?;
    require(
        prompt["unit_id"] == id
            && prompt["question_id"] == qid
            && prompt["case"] == case
            && prompt["source_observation_cutoff"] == cutoff,
        "prompt_identity",
    )?;
    require(
        prompt["source_file"] == "source.json"
            && prompt["method_file"] == "method.json"
            && prompt["instructions_file"] == "instructions.txt",
        "prompt_input_mapping",
    )?;
    require(
        prompt["question"].as_str().is_some() && prompt["question"] == answer["question_at_cutoff"],
        "exact_cutoff_prompt",
    )?;
    Ok(prompt)
}
fn validate_method(
    files: &Files,
    bytes: &[u8],
    version: &Value,
    inventory: &Value,
    rules: &Value,
) -> Result<(), PackageError> {
    let method = json_bytes(bytes)?;
    schema(&method, "merl.visible-reader-method/v1")?;
    require(
        method["question_policy"] == inventory["question_policy"]
            && method["matching_rules"] == rules["matching_rules"]
            && method["scoring_rules"] == rules["scoring_rules"]
            && method["method_approval"] == APPROVAL
            && method["version"] == version["package_version"],
        "reader_method_copy",
    )?;
    files.check_hash(RULES, &method["provenance"]["approved_rules_sha256"])?;
    files.check_hash(
        INVENTORY,
        &method["provenance"]["approved_inventory_sha256"],
    )?;
    Ok(())
}

fn validate_counts(
    version: &Value,
    scope: &Value,
    index: &Value,
    units: &[VerifiedUnit],
    prefixes: &BTreeMap<(String, u64), Value>,
    questions: &BTreeSet<String>,
    cases: &BTreeSet<String>,
) -> Result<(), PackageError> {
    for ((case, cutoff), source) in prefixes {
        for ((other_case, other_cutoff), other) in prefixes {
            if case == other_case && cutoff < other_cutoff {
                let length = usize::try_from(*cutoff)
                    .map_err(|_| PackageError::Integrity("prefix_length"))?;
                require(
                    array(source, "observations")?.as_slice()
                        == &array(other, "observations")?[..length],
                    "prefix_observation_changed",
                )?;
                require(
                    source["source_identity"] == other["source_identity"]
                        && source["history_fidelity"] == other["history_fidelity"],
                    "prefix_metadata_changed",
                )?;
            }
        }
    }
    let supplied_questions = array(scope, "question_ids")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(PackageError::Integrity("question_id"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        *questions == supplied_questions && array(scope, "question_ids")?.len() == questions.len(),
        "approved_question_list",
    )?;
    for (value, fields) in [
        (
            &version,
            [
                "unit_count",
                "question_count",
                "source_prefix_count",
                "case_count",
            ],
        ),
        (
            &scope,
            [
                "approved_question_cutoff_units",
                "approved_questions",
                "approved_distinct_case_cutoffs",
                "approved_case_count",
            ],
        ),
    ] {
        for (field, count) in
            fields
                .into_iter()
                .zip([units.len(), questions.len(), prefixes.len(), cases.len()])
        {
            require(
                number(value, field)? == count as u64,
                "declared_inventory_counts",
            )?;
        }
    }
    for (field, count) in [
        ("unit_count", units.len()),
        ("question_count", questions.len()),
        ("source_prefix_count", prefixes.len()),
    ] {
        require(number(index, field)? == count as u64, "unit_index_counts")?;
    }
    Ok(())
}
