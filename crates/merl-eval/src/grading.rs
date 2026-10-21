//! Distinct legacy numeric and approved question-meaning grading contracts.

use crate::{BenchmarkError, Score, Scorer};
use serde::{Deserialize, Serialize};

/// Approved categorical vocabulary. No numeric weighting is implied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// All required meanings and evidence guards are satisfied.
    Correct,
    /// A required meaning is missing.
    Incomplete,
    /// The answer contradicts evidence or makes unsupported in-scope claims.
    Incorrect,
    /// The grader declined an excluded scope; this is never success credit.
    NotScored,
}

/// Evaluator-owned semantic verdict for one exact question-cutoff unit.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CategoricalGrade {
    /// Exact unit the grader evaluated.
    pub unit_id: String,
    /// Approved categorical outcome.
    pub verdict: Verdict,
    /// Recorded justification; no object-key or enum equality is inferred.
    pub reason: String,
}

/// A grade's native contract, without silently mapping categorical results to numbers.
pub trait TrialGrade: Clone + std::fmt::Debug + Serialize {
    /// Checks this grade belongs to the submitted unit and satisfies its wire contract.
    ///
    /// # Errors
    /// Rejects a mismatched unit or malformed result.
    fn validate(&self, unit_id: &str) -> Result<(), BenchmarkError>;
    /// Legacy scalar grades, when the grade actually uses that contract.
    fn scalar(&self) -> Option<Score>;
}
impl TrialGrade for Score {
    fn validate(&self, _unit_id: &str) -> Result<(), BenchmarkError> {
        for grade in [self.correctness, self.provenance].into_iter().flatten() {
            if !(0.0..=1.0).contains(&grade) {
                return Err(BenchmarkError::InvalidScore);
            }
        }
        Ok(())
    }
    fn scalar(&self) -> Option<Score> {
        Some(*self)
    }
}
impl TrialGrade for CategoricalGrade {
    fn validate(&self, unit_id: &str) -> Result<(), BenchmarkError> {
        if self.unit_id != unit_id || self.reason.trim().is_empty() {
            return Err(BenchmarkError::Scoring(
                "categorical grade requires the exact unit and a reason".to_owned(),
            ));
        }
        Ok(())
    }
    fn scalar(&self) -> Option<Score> {
        None
    }
}

/// Evaluator boundary shared by legacy and semantic runs, retaining native grades.
pub trait Grader {
    /// Native result type; categorical grades remain categorical.
    type Grade: TrialGrade;
    /// Grades a plain semantic answer and optional citations.
    ///
    /// # Errors
    /// Returns a transport, missing-oracle or invalid-grade failure.
    fn grade(
        &mut self,
        unit_id: &str,
        answer: &str,
        citations: &[String],
    ) -> Result<Self::Grade, String>;
}
impl<T: Scorer> Grader for T {
    type Grade = Score;
    fn grade(
        &mut self,
        unit_id: &str,
        answer: &str,
        citations: &[String],
    ) -> Result<Score, String> {
        self.score(unit_id, answer, citations)
    }
}

/// Isolated evaluator process with only the selected unit's verified static files.
#[derive(Debug)]
pub struct CategoricalProcessGrader<'a> {
    package: &'a crate::package::VerifiedPackage,
    program: crate::isolation::IsolatedProgram,
    calls: Vec<crate::isolation::ProcessCallRecord>,
}
impl<'a> CategoricalProcessGrader<'a> {
    /// Selects a verified package and pinned grader executable.
    #[must_use]
    pub fn new(
        package: &'a crate::package::VerifiedPackage,
        program: crate::isolation::IsolatedProgram,
    ) -> Self {
        Self {
            package,
            program,
            calls: Vec::new(),
        }
    }
    /// Exact invocation hashes, including runtime response and unit rubric files.
    #[must_use]
    pub fn calls(&self) -> &[crate::isolation::ProcessCallRecord] {
        &self.calls
    }
}
impl Grader for CategoricalProcessGrader<'_> {
    type Grade = CategoricalGrade;
    fn grade(
        &mut self,
        unit_id: &str,
        answer: &str,
        citations: &[String],
    ) -> Result<CategoricalGrade, String> {
        let unit = self
            .package
            .units()
            .iter()
            .find(|unit| unit.id() == unit_id)
            .ok_or_else(|| "grader unit is not approved".to_owned())?;
        let request = serde_json::json!({ "schema": "merl.eval-categorical-request/v1", "unit_id": unit_id,
            "answer": answer, "citations": citations });
        let mut inputs = unit.grader_files();
        inputs.insert(
            "response.json".into(),
            serde_json::to_vec(&request).map_err(|_| "grader input JSON")?,
        );
        let response = self
            .program
            .recorded_call(&inputs, &request, &mut self.calls)
            .map_err(|error| error.to_string())?;
        let grade: CategoricalGrade =
            serde_json::from_value(response).map_err(|_| "invalid categorical response")?;
        grade.validate(unit_id).map_err(|error| error.to_string())?;
        Ok(grade)
    }
}
