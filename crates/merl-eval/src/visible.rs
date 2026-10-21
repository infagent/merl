//! Approved question-cutoff execution and categorical completeness accounting.

use std::collections::BTreeSet;

use merl_corpus::fixture::{Fixture, source_digest, validate};
use serde::Serialize;

use crate::grading::{CategoricalGrade, Grader, TrialGrade, Verdict};
use crate::package::{PackageIdentity, VerifiedPackage};
use crate::runner::{PreparationRecord, TrialFailure};
use crate::{
    BenchmarkConfig, BenchmarkError, BenchmarkMethod, BenchmarkRunner, EvaluationQuestion,
    MerlSurface, ModelAdapter, SummaryCallRecord, TrialReport,
};

/// Result for one shared preparation prefix; every question remains a scoring unit.
#[derive(Clone, Debug, Serialize)]
pub struct PrefixReport {
    /// Case whose evidence was disclosed.
    pub case: String,
    /// Inclusive source cutoff.
    pub cutoff: u64,
    /// Body availability and unresolved-order limits retained from the renderer.
    pub source_fidelity: Vec<crate::CutoffFidelity>,
    /// Independently scored answers; categorical results have no scalar substitute.
    pub trials: Vec<TrialReport<CategoricalGrade>>,
    /// Failed slots with known usage and retained answers when available.
    pub failures: Vec<TrialFailure>,
    /// Costs charged once per applicable method and paired trial, even on failure.
    pub preparation: Vec<PreparationRecord>,
    /// Physical shared summary calls, separate from per-method cost attribution.
    pub summary_calls: Vec<SummaryCallRecord>,
}

impl PrefixReport {
    /// Invalidates all purported grades after an authority changes during reads.
    /// Known usage and answer text remain available for audit without score credit.
    pub fn invalidate_authority(&mut self) {
        self.failures
            .extend(self.trials.drain(..).map(|t| TrialFailure {
                unit_id: t.question_id,
                method: t.method,
                trial_id: t.trial_id,
                code: "authority_changed",
                answer_calls: t.answer_calls,
                answer: Some(t.answer),
                citations: t.citations,
                usage_incomplete: false,
            }));
    }
}

/// Checks original fixture JSON values against every approved unit at this prefix.
/// Call before parsing can discard optional fields or their original representation.
///
/// # Errors
/// Rejects an absent prefix or any changed observation, source identity or fidelity.
pub fn verify_prefix(
    package: &VerifiedPackage,
    bytes: &[u8],
    cutoff: u64,
) -> Result<(), BenchmarkError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| BenchmarkError::InvalidConfig("fixture JSON"))?;
    let case = value["id"]
        .as_str()
        .ok_or(BenchmarkError::InvalidConfig("fixture identity"))?;
    let units = package
        .units()
        .iter()
        .filter(|u| u.case() == case && u.cutoff() == cutoff)
        .collect::<Vec<_>>();
    if units.is_empty() {
        return Err(BenchmarkError::InvalidConfig("unapproved case/cutoff"));
    }
    let length =
        usize::try_from(cutoff).map_err(|_| BenchmarkError::InvalidConfig("cutoff length"))?;
    let observations = value["observations"]
        .as_array()
        .and_then(|a| a.get(..length))
        .ok_or(BenchmarkError::InvalidConfig("fixture prefix"))?;
    for unit in units {
        if unit.source()["observations"].as_array().map(Vec::as_slice) != Some(observations)
            || unit.source()["source_identity"] != value["source"]
            || unit.source()["history_fidelity"] != value["capture"]["history_fidelity"]
        {
            return Err(BenchmarkError::InvalidConfig(
                "package prefix differs from original fixture",
            ));
        }
    }
    Ok(())
}

/// Executes every approved unit sharing one case and cutoff, with fresh summary state.
/// The trusted fixture is checked against the package, then truncated before rendering
/// or updating summaries. Readers never receive terminal provider facts or gold labels.
///
/// # Errors
/// Rejects mismatched fixtures, unapproved prefixes, invalid configuration or inputs.
pub fn run_prefix<M: ModelAdapter, S: MerlSurface, J: Grader<Grade = CategoricalGrade>>(
    package: &VerifiedPackage,
    fixture_bytes: &[u8],
    cutoff: u64,
    config: BenchmarkConfig,
    model: &mut M,
    merl: &mut S,
    grader: &mut J,
) -> Result<PrefixReport, BenchmarkError> {
    verify_prefix(package, fixture_bytes, cutoff)?;
    let fixture: Fixture = serde_json::from_slice(fixture_bytes)
        .map_err(|_| BenchmarkError::InvalidConfig("fixture JSON"))?;
    validate(&fixture).map_err(crate::InputError::from)?;
    let units = package
        .units()
        .iter()
        .filter(|u| u.case() == fixture.id && u.cutoff() == cutoff)
        .collect::<Vec<_>>();
    let first = units
        .first()
        .ok_or(BenchmarkError::InvalidConfig("unapproved case/cutoff"))?;
    let length =
        usize::try_from(cutoff).map_err(|_| BenchmarkError::InvalidConfig("cutoff length"))?;
    let boundary_unordered = crate::cutoff_splits_unordered_group(&fixture, cutoff);
    // Preserve the boundary's fidelity fact, then remove all later source bytes
    // before any renderer or stateful preparation runs.
    // Terminal provider facts remain internal validation metadata, never model input.
    let mut bounded = fixture.clone();
    bounded.observations.truncate(length);
    bounded.gold_states.clear();
    bounded.capture.provider_observation_count = length;
    bounded.capture.source_sha256 =
        source_digest(&bounded.provider_snapshot, &bounded.observations);
    let questions = units
        .iter()
        .map(|unit| EvaluationQuestion {
            id: unit.id().to_owned(),
            cutoff,
            capture_phase: false,
            text: unit.question().to_owned(),
        })
        .collect();
    let mut disclosed_model = VisibleModel { package, model };
    let mut result = BenchmarkRunner {
        model: &mut disclosed_model,
        merl,
        scorer: grader,
    }
    .run_suite_recording(&bounded, questions, config)?;
    if boundary_unordered {
        for source in &mut result.source_fidelity {
            source.fidelity.cutoff_splits_unordered_group = true;
            source.fidelity.exact_replay = false;
        }
    }
    Ok(PrefixReport {
        case: first.case().to_owned(),
        cutoff,
        source_fidelity: result.source_fidelity,
        trials: result.trials,
        failures: result.failures,
        preparation: result.preparation,
        summary_calls: result.summary_calls,
    })
}

/// One expected scoring slot that has not produced a result or explicit failure.
#[derive(Clone, Debug, Serialize)]
pub struct MissingUnit {
    /// Exact approved question-cutoff identity.
    pub unit_id: String,
    /// Comparison arm.
    pub method: BenchmarkMethod,
    /// Independent paired trial.
    pub trial_id: String,
}

/// Native categorical counts. Exclusions never contribute to any success count.
#[derive(Clone, Debug, Serialize)]
pub struct VerdictCounts {
    /// Comparison arm.
    pub method: BenchmarkMethod,
    /// Approved units multiplied by paired trials, including missing and failed slots.
    pub expected: usize,
    /// Semantically correct answers.
    pub correct: usize,
    /// Answers missing an approved meaning.
    pub incomplete: usize,
    /// Contradictory or unsupported answers.
    pub incorrect: usize,
    /// Explicit scope exclusions, retained without credit.
    pub not_scored: usize,
    /// Execution or grading failures.
    pub failed: usize,
    /// Expected slots absent from all submitted prefix reports.
    pub missing: usize,
}

/// Complete inventory accounting, with native verdicts and separate preparation costs.
#[derive(Debug, Serialize)]
pub struct PackageReport {
    /// New harness report contract; independent of the approved artifact schemas.
    pub schema: &'static str,
    /// Exact approved bytes used for this report.
    pub package: PackageIdentity,
    /// Frozen common model configuration.
    pub config: BenchmarkConfig,
    /// Original ordered scoring inventory.
    pub unit_ids: Vec<String>,
    /// Audit references; never scoring slots or success credits.
    pub exclusion_ids: Vec<String>,
    /// Shared preparation groups and their separately scored results.
    pub prefixes: Vec<PrefixReport>,
    /// Explicit slots absent from the submitted groups.
    pub missing: Vec<MissingUnit>,
    /// Native categories with no invented numeric weighting.
    pub counts: Vec<VerdictCounts>,
}

impl PackageReport {
    /// Checks the submitted groups and fills every missing scoring slot explicitly.
    ///
    /// # Errors
    /// Rejects duplicate, unapproved or mismatched units, trials, prompts or grades.
    pub fn assemble(
        package: &VerifiedPackage,
        config: BenchmarkConfig,
        prefixes: Vec<PrefixReport>,
    ) -> Result<Self, BenchmarkError> {
        if config.trials.is_empty() {
            return Err(BenchmarkError::InvalidConfig("paired trials are required"));
        }
        let mut trial_ids = BTreeSet::new();
        for trial in &config.trials {
            if trial.id.is_empty() || !trial_ids.insert(trial.id.as_str()) {
                return Err(BenchmarkError::InvalidConfig(
                    "paired trial identities must be unique",
                ));
            }
        }
        let mut seen = BTreeSet::new();
        let mut groups = BTreeSet::new();
        for prefix in &prefixes {
            if !groups.insert((prefix.case.as_str(), prefix.cutoff)) {
                return Err(BenchmarkError::InvalidConfig(
                    "duplicate preparation prefix",
                ));
            }
            for result in &prefix.trials {
                result.score.validate(&result.question_id)?;
                record_slot(
                    package,
                    &trial_ids,
                    &mut seen,
                    prefix,
                    &result.question_id,
                    &result.trial_id,
                    result.method,
                )?;
            }
            for result in &prefix.failures {
                record_slot(
                    package,
                    &trial_ids,
                    &mut seen,
                    prefix,
                    &result.unit_id,
                    &result.trial_id,
                    result.method,
                )?;
            }
        }
        let mut missing = Vec::new();
        for unit in package.units() {
            for trial in &config.trials {
                for method in BenchmarkMethod::ALL {
                    if !seen.contains(&(
                        unit.id().to_owned(),
                        trial.id.clone(),
                        method_index(method),
                    )) {
                        missing.push(MissingUnit {
                            unit_id: unit.id().to_owned(),
                            trial_id: trial.id.clone(),
                            method,
                        });
                    }
                }
            }
        }
        let counts = BenchmarkMethod::ALL
            .into_iter()
            .map(|method| {
                let grades = prefixes
                    .iter()
                    .flat_map(|p| &p.trials)
                    .filter(|t| t.method == method)
                    .map(|t| t.score.verdict)
                    .collect::<Vec<_>>();
                VerdictCounts {
                    method,
                    expected: package.units().len() * config.trials.len(),
                    correct: grades.iter().filter(|&&v| v == Verdict::Correct).count(),
                    incomplete: grades.iter().filter(|&&v| v == Verdict::Incomplete).count(),
                    incorrect: grades.iter().filter(|&&v| v == Verdict::Incorrect).count(),
                    not_scored: grades.iter().filter(|&&v| v == Verdict::NotScored).count(),
                    failed: prefixes
                        .iter()
                        .flat_map(|p| &p.failures)
                        .filter(|f| f.method == method)
                        .count(),
                    missing: missing.iter().filter(|m| m.method == method).count(),
                }
            })
            .collect();
        Ok(Self {
            schema: "merl.eval-visible-report/v1",
            package: package.identity().clone(),
            config,
            unit_ids: package.units().iter().map(|u| u.id().to_owned()).collect(),
            exclusion_ids: package.exclusion_ids().to_vec(),
            prefixes,
            missing,
            counts,
        })
    }
}

fn method_index(method: BenchmarkMethod) -> usize {
    BenchmarkMethod::ALL
        .iter()
        .position(|&m| m == method)
        .expect("method is in the frozen comparison")
}

fn record_slot(
    package: &VerifiedPackage,
    trials: &BTreeSet<&str>,
    seen: &mut BTreeSet<(String, String, usize)>,
    prefix: &PrefixReport,
    unit_id: &str,
    trial_id: &str,
    method: BenchmarkMethod,
) -> Result<(), BenchmarkError> {
    if !trials.contains(trial_id)
        || !package
            .units()
            .iter()
            .any(|u| u.id() == unit_id && u.case() == prefix.case && u.cutoff() == prefix.cutoff)
    {
        return Err(BenchmarkError::InvalidConfig("unapproved scoring slot"));
    }
    if !seen.insert((
        unit_id.to_owned(),
        trial_id.to_owned(),
        method_index(method),
    )) {
        return Err(BenchmarkError::InvalidConfig("duplicate scoring slot"));
    }
    Ok(())
}

// Keep the existing model wire schema and the exact question. Reader instructions
// and the approved general method are disclosed context, never raw source access.
struct VisibleModel<'a, M> {
    package: &'a VerifiedPackage,
    model: &'a mut M,
}
impl<M: ModelAdapter> ModelAdapter for VisibleModel<'_, M> {
    fn failed_usage(&self) -> crate::TokenUsage {
        self.model.failed_usage()
    }
    fn summarize(
        &mut self,
        request: crate::SummaryRequest<'_>,
    ) -> Result<crate::SummaryResponse, String> {
        self.model.summarize(request)
    }
    fn answer(
        &mut self,
        request: crate::AnswerRequest<'_>,
    ) -> Result<crate::AnswerResponse, String> {
        let unit = self
            .package
            .units()
            .iter()
            .find(|u| u.id() == request.question_id)
            .ok_or("unapproved reader unit")?;
        let (instructions, method) = unit.reader_policy().map_err(|_| "invalid reader policy")?;
        let context = format!(
            "[reader instructions]\n{instructions}\n[approved reader method]\n{method}\n[method-disclosed evidence]\n{}",
            request.context
        );
        self.model.answer(crate::AnswerRequest {
            context: &context,
            ..request
        })
    }
}
