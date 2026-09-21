//! Process boundaries for model and scorer calls, and the public Merl CLI view.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use merl_cli::CliResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AnswerAction, AnswerRequest, AnswerResponse, EvaluationQuestion, MerlPrepared, MerlSurface,
    ModelAdapter, Score, Scorer, SummaryRequest, SummaryResponse, TokenUsage,
};

const WIRE_SCHEMA: &str = "merl.eval-adapter/v1";

/// One external model program that consumes a JSON request and emits a bounded JSON response.
#[derive(Debug)]
pub struct ProcessModelAdapter {
    program: PathBuf,
    max_request_bytes: usize,
    max_response_bytes: usize,
}

impl ProcessModelAdapter {
    /// Selects the executable and hard I/O bounds for one model call.
    #[must_use]
    pub fn new(
        program: impl Into<PathBuf>,
        max_request_bytes: usize,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            program: program.into(),
            max_request_bytes,
            max_response_bytes,
        }
    }

    fn call(&self, request: &Value) -> Result<WireResponse, String> {
        let response: WireResponse = run_json_process(
            &self.program,
            request,
            self.max_request_bytes,
            self.max_response_bytes,
            None,
        )?;
        if response.schema() != WIRE_SCHEMA {
            return Err("model adapter returned an unsupported schema".to_owned());
        }
        Ok(response)
    }
}

impl ModelAdapter for ProcessModelAdapter {
    fn summarize(&mut self, request: SummaryRequest<'_>) -> Result<SummaryResponse, String> {
        let response = self.call(&json!({
            "schema": WIRE_SCHEMA,
            "kind": "summarize",
            "config": model_call_config(request.config),
            "trial": request.trial,
            "previous_summary": request.previous,
            "source": {
                "sequence": request.source.sequence,
                "supersedes": request.source.supersedes,
                "provider_id": request.source.provider_id,
                "author_id": request.source.author_id,
                "created_at": request.source.created_at,
                "occurred_at": request.source.occurred_at,
                "body": request.source.body,
                "disclosed_at_capture": request.source.disclosed_at_capture
            }
        }))?;
        match response {
            WireResponse::Summary { text, usage, .. } => Ok(SummaryResponse { text, usage }),
            _ => Err("model adapter returned an answer to a summary request".to_owned()),
        }
    }

    fn answer(&mut self, request: AnswerRequest<'_>) -> Result<AnswerResponse, String> {
        let response = self.call(&answer_wire_request(&request))?;
        let (action, usage) = match response {
            WireResponse::Final {
                answer,
                citations,
                usage,
                ..
            } => (AnswerAction::Final { answer, citations }, usage),
            WireResponse::Search { query, usage, .. } => (AnswerAction::Search(query), usage),
            WireResponse::Expand {
                reference, usage, ..
            } => (AnswerAction::Expand(reference), usage),
            WireResponse::Summary { .. } => {
                return Err("model adapter returned a summary to an answer request".to_owned());
            }
        };
        Ok(AnswerResponse { action, usage })
    }
}

fn answer_wire_request(request: &AnswerRequest<'_>) -> Value {
    json!({
        "schema": WIRE_SCHEMA,
        "kind": "answer",
        "config": model_call_config(request.config),
        "trial": request.trial,
        "question_id": request.question_id,
        "question": request.question,
        "context": request.context,
        "can_search": request.can_search,
        "can_expand": request.can_expand
    })
}

fn model_call_config(config: &crate::BenchmarkConfig) -> Value {
    json!({
        "model": config.model,
        "model_version": config.model_version,
        "effort": config.effort,
        "system_prompt": config.system_prompt,
        "task_prompt": config.task_prompt,
        "temperature": config.temperature
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireResponse {
    Summary {
        schema: String,
        usage: TokenUsage,
        text: String,
    },
    Final {
        schema: String,
        usage: TokenUsage,
        answer: String,
        citations: Vec<String>,
    },
    Search {
        schema: String,
        usage: TokenUsage,
        query: String,
    },
    Expand {
        schema: String,
        usage: TokenUsage,
        reference: String,
    },
}

impl WireResponse {
    fn schema(&self) -> &str {
        match self {
            Self::Summary { schema, .. }
            | Self::Final { schema, .. }
            | Self::Search { schema, .. }
            | Self::Expand { schema, .. } => schema,
        }
    }
}

/// Evaluator-owned scorer process. Gold labels never enter the model request.
#[derive(Debug)]
pub struct ProcessScorer {
    program: PathBuf,
    max_response_bytes: usize,
    /// Evaluator-owned scoring bundle, exposed only to the scorer process.
    pub scoring_spec: Option<PathBuf>,
}

impl ProcessScorer {
    /// Selects the scorer executable and maximum JSON response size.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>, max_response_bytes: usize) -> Self {
        Self {
            program: program.into(),
            max_response_bytes,
            scoring_spec: None,
        }
    }
}

impl Scorer for ProcessScorer {
    fn score(
        &mut self,
        question_id: &str,
        answer: &str,
        citations: &[String],
    ) -> Result<Score, String> {
        let response: WireScore = run_json_process(
            &self.program,
            &json!({
                "schema": "merl.eval-score-request/v1",
                "question_id": question_id,
                "answer": answer,
                "citations": citations
            }),
            1_048_576,
            self.max_response_bytes,
            self.scoring_spec.as_deref(),
        )?;
        if response.schema != "merl.eval-score/v1" {
            return Err("scorer returned an unsupported schema".to_owned());
        }
        Ok(response.score)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireScore {
    schema: String,
    score: Score,
}

fn run_json_process<T: for<'de> Deserialize<'de>>(
    program: &Path,
    request: &Value,
    max_request_bytes: usize,
    max_response_bytes: usize,
    scoring_spec: Option<&Path>,
) -> Result<T, String> {
    let bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    if bytes.len() > max_request_bytes {
        return Err("adapter request exceeds its byte budget".to_owned());
    }
    let mut command = Command::new(program);
    if let Some(path) = scoring_spec {
        command.env("MERL_EVAL_SCORING_SPEC", path);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start adapter: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&bytes)
            .map_err(|error| format!("adapter input failed: {error}"))?;
    }
    let mut response = Vec::new();
    let limit = u64::try_from(max_response_bytes).map_err(|error| error.to_string())?;
    child
        .stdout
        .take()
        .ok_or("adapter stdout unavailable")?
        .take(limit.saturating_add(1))
        .read_to_end(&mut response)
        .map_err(|error| format!("adapter output failed: {error}"))?;
    if response.len() > max_response_bytes {
        let _ = child.kill();
        let _ = child.wait();
        return Err("adapter response exceeds its byte budget".to_owned());
    }
    let status = child
        .wait()
        .map_err(|error| format!("adapter wait failed: {error}"))?;
    if !status.success() {
        return Err(format!("adapter exited with {status}"));
    }
    serde_json::from_slice(&response).map_err(|error| format!("invalid adapter JSON: {error}"))
}

/// Reads a prepared Issue through Merl's public CLI and expands requested refs.
#[derive(Debug)]
pub struct MerlTrialArtifact {
    /// Closed authority database prepared for one independent trial.
    pub database: PathBuf,
    /// Measured compilation, correction, and retry usage for this trial.
    pub preparation_usage: TokenUsage,
}

/// Reads prepared Issues through Merl's public CLI, one authority per trial.
#[derive(Debug)]
pub struct CliMerlSurface {
    /// Project identity within that database.
    pub project: String,
    /// Provider Issue object identity.
    pub issue: String,
    /// Stable Issue conversation scope.
    pub scope: String,
    /// Reader role used for this benchmark case.
    pub role: String,
    /// Independently prepared authority and measured cost for each paired trial.
    pub trials: Vec<MerlTrialArtifact>,
    /// Index of the next authority to read.
    pub next_trial: usize,
    active_database: Option<PathBuf>,
}

impl CliMerlSurface {
    /// Creates a reader over independently prepared candidate trials.
    #[must_use]
    pub fn new(
        project: String,
        issue: String,
        scope: String,
        role: String,
        trials: Vec<MerlTrialArtifact>,
    ) -> Self {
        Self {
            project,
            issue,
            scope,
            role,
            trials,
            next_trial: 0,
            active_database: None,
        }
    }

    fn command(&self, database: &Path, args: &[&str]) -> Result<Value, String> {
        let mut arguments = args.iter().map(ToString::to_string).collect::<Vec<_>>();
        arguments.extend([
            "--project".to_owned(),
            self.project.clone(),
            "--database".to_owned(),
            database.display().to_string(),
            "--json".to_owned(),
        ]);
        let output = match merl_cli::run(&arguments) {
            CliResponse::Success(output) => output,
            CliResponse::HumanError(error) | CliResponse::JsonError(error) => {
                return Err(error.to_string());
            }
        };
        serde_json::from_str(&output).map_err(|error| format!("invalid Merl CLI JSON: {error}"))
    }
}

impl MerlSurface for CliMerlSurface {
    fn prepare(&mut self, question: &EvaluationQuestion) -> Result<MerlPrepared, String> {
        let trial = self
            .trials
            .get(self.next_trial)
            .ok_or("Merl trial artifact is missing")?;
        let view = self.command(
            &trial.database,
            &[
                "issue",
                "view",
                "--issue",
                &self.issue,
                "--scope",
                &self.scope,
                "--role",
                &self.role,
            ],
        )?;
        let coverage = &view["coverage"];
        let source_cutoff = coverage["observation_head"]
            .as_u64()
            .ok_or("Merl view omitted source head")?;
        let complete = coverage["processed_through"].as_u64() == Some(question.cutoff)
            && [
                "required_gaps",
                "required_pending",
                "required_failed",
                "required_purged",
            ]
            .iter()
            .all(|field| coverage[*field].as_u64() == Some(0));
        let prepared = MerlPrepared {
            source_cutoff,
            view: serde_json::to_string(&view).map_err(|error| error.to_string())?,
            preparation_usage: trial.preparation_usage,
            required_coverage_complete: complete,
            causal: true,
        };
        self.active_database = Some(trial.database.clone());
        self.next_trial += 1;
        Ok(prepared)
    }

    fn expand(&mut self, reference: &str) -> Result<String, String> {
        let database = self
            .active_database
            .as_deref()
            .ok_or("prepare a Merl trial before expansion")?;
        let detail = if let Some(source) = reference.strip_prefix("source:") {
            self.command(database, &["source", "show", "--version", source])?
        } else {
            self.command(database, &["show", reference, "--source"])?
        };
        serde_json::to_string(&detail).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AnswerRequest, WireResponse, answer_wire_request};
    use crate::{BenchmarkConfig, BenchmarkMethod, RandomnessControl, TrialIdentity};

    #[test]
    fn model_answer_wire_does_not_reveal_the_benchmark_arm() {
        let trial = TrialIdentity {
            id: "pair-a".to_owned(),
            randomness: RandomnessControl::Seed(7),
        };
        let config = BenchmarkConfig {
            model: "model".to_owned(),
            model_version: "v1".to_owned(),
            effort: "medium".to_owned(),
            system_prompt: "Use the context.".to_owned(),
            task_prompt: "Answer the question.".to_owned(),
            trials: vec![trial.clone()],
            recent_window: 2,
            max_tool_rounds: 1,
            search_results: 2,
            temperature: Some(0.0),
        };
        let wire = answer_wire_request(&AnswerRequest {
            config: &config,
            trial: &trial,
            method: BenchmarkMethod::Merl,
            question_id: "Q1",
            question: "What is current?",
            context: "D1 active",
            can_search: false,
            can_expand: true,
        });
        assert!(wire.get("method").is_none());
        assert_eq!(wire["trial"]["id"], "pair-a");
        assert_eq!(wire["trial"]["randomness"]["seed"], 7);
        assert_eq!(wire["can_expand"], true);
    }

    #[test]
    fn model_response_rejects_unexpected_prose_fields() {
        let valid = json!({
            "schema": "merl.eval-adapter/v1",
            "kind": "final",
            "answer": "Sweep gain.",
            "citations": ["O4"],
            "usage": { "input": 120, "output": 18 }
        });
        let parsed: WireResponse = serde_json::from_value(valid.clone()).expect("valid action");
        assert!(matches!(parsed, WireResponse::Final { .. }));

        let mut with_essay = valid;
        with_essay["rationale"] = json!("I reasoned step by step...");
        assert!(serde_json::from_value::<WireResponse>(with_essay).is_err());
    }
}
