//! Inspection and administration of a binding's capture policy and selectors.

use crate::{CliError, invalid_input, parse_project, render_json};
use merl_core::compilation_policy::{ActorClass, IssueSourceKind, PolicyEdit, Selector};
use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, PolicyInputId,
    SourceBindingId,
};
use merl_policy::{BindingPolicyChange, change_binding_policy};
use merl_store::Store;
use serde_json::json;
use std::path::Path;

#[derive(Clone, Copy, Default)]
pub(super) struct Options<'a> {
    pub database: Option<&'a str>,
    pub project: Option<&'a str>,
    pub id: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub expected: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub coverage: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub actor_class: Option<&'a str>,
    pub provider_actor: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub project_override: bool,
    pub remove: bool,
}

impl Options<'_> {
    fn edit(self) -> Result<PolicyEdit, CliError> {
        let actor_class = self
            .actor_class
            .map(|value| match value {
                "human" => Ok(ActorClass::Human),
                "bot" => Ok(ActorClass::Bot),
                "routine_agent" => Ok(ActorClass::RoutineAgent),
                "unknown" => Ok(ActorClass::Unknown),
                _ => Err(invalid_input(
                    "--actor-class must be human, bot, routine_agent, or unknown",
                )),
            })
            .transpose()?;
        let kind = self
            .kind
            .map(|value| match value {
                "issue" => Ok(IssueSourceKind::Issue),
                "issue_comment" => Ok(IssueSourceKind::IssueComment),
                _ => Err(invalid_input("--kind must be issue or issue_comment")),
            })
            .transpose()?;
        if self.source_version.is_some() {
            return Err(invalid_input("--version is for capture inspection"));
        }
        if let Some(provider_actor) = self.provider_actor {
            if kind.is_some() || self.project_override || (actor_class.is_some() == self.remove) {
                return Err(invalid_input(
                    "classify with --provider-actor and either --actor-class or --remove",
                ));
            }
            return Ok(PolicyEdit::Classify {
                provider_actor: provider_actor.to_owned(),
                actor_class,
            });
        }
        if self.project_override {
            if kind.is_some() || actor_class.is_some() {
                return Err(invalid_input("--override cannot include a selector"));
            }
            return Ok(PolicyEdit::Override {
                remove: self.remove,
            });
        }
        if kind.is_some() || actor_class.is_some() {
            return Ok(PolicyEdit::Rule {
                selector: Selector { kind, actor_class },
                remove: self.remove,
            });
        }
        if self.remove {
            return Err(invalid_input(
                "--remove needs a selector, override, or provider actor",
            ));
        }
        Ok(PolicyEdit::Defaults)
    }
}

pub(super) fn execute(
    binding: &str,
    set: bool,
    options: Options<'_>,
    json_output: bool,
    now: i64,
) -> Result<String, CliError> {
    let project = parse_project(required(options.project, "--project")?)?;
    let binding = SourceBindingId::try_from(binding).map_err(|e| invalid_input(&e.to_string()))?;
    let mut store = Store::open(Path::new(required(options.database, "--database")?))?;
    if !set {
        return inspect(
            &store,
            &project,
            &binding,
            options.source_version,
            json_output,
        );
    }
    let edit = options.edit()?;
    let values_required = matches!(
        edit,
        PolicyEdit::Defaults
            | PolicyEdit::Rule { remove: false, .. }
            | PolicyEdit::Override { remove: false }
    );
    if values_required && (options.mode.is_none() || options.coverage.is_none()) {
        return Err(invalid_input(
            "--mode and --coverage are required for a policy value change",
        ));
    }
    if !values_required && (options.mode.is_some() || options.coverage.is_some()) {
        return Err(invalid_input(
            "classification and removal do not accept --mode or --coverage",
        ));
    }
    let change = BindingPolicyChange {
        id: PolicyInputId::try_from(required(options.id, "--id")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        actor: ActorId::try_from(required(options.actor, "--actor")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        binding,
        expected_version: CapturePolicyVersion::try_from(required(
            options.expected,
            "--expected-version",
        )?)
        .map_err(|e| invalid_input(&e.to_string()))?,
        mode: match options.mode.unwrap_or("capture_only") {
            "capture_only" => CompilationMode::CaptureOnly,
            "on_demand" => CompilationMode::OnDemand,
            "eager" => CompilationMode::Eager,
            _ => {
                return Err(invalid_input(
                    "--mode must be capture_only, on_demand, or eager",
                ));
            }
        },
        coverage: match options.coverage.unwrap_or("optional") {
            "optional" => CoverageRequirement::Optional,
            "required" => CoverageRequirement::Required,
            _ => return Err(invalid_input("--coverage must be optional or required")),
        },
        reason: required(options.reason, "--reason")?.to_owned(),
        edit,
    };
    let record = change_binding_policy(&mut store, &project, &change, now)?;
    let outcome = record.inputs[0].disposition.as_str();
    if json_output {
        render_json(
            &json!({"schema":"merl.binding-policy-change/v1","action":"source.compilation-policy.set","project":project.as_str(),"binding":change.binding.as_str(),"request":change.id.as_str(),"actor":change.actor.as_str(),"outcome":outcome,"reason_code":record.inputs[0].reason.as_str(),"revision":record.committed_revision.map(merl_core::ProjectRevision::get),"evaluation":record.id.as_str(),"authorization_policy":record.version.as_str()}),
        )
    } else {
        Ok(format!(
            "{outcome}: binding {} policy change by {}; {}; accepted revision {}.\n",
            change.binding,
            change.actor,
            record.inputs[0].reason,
            record
                .committed_revision
                .map_or_else(|| "none".to_owned(), |r| r.get().to_string())
        ))
    }
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, CliError> {
    value.ok_or_else(|| invalid_input(&format!("{name} is required")))
}

pub(super) fn help(set: bool, json_output: bool) -> Result<String, CliError> {
    let (command, usage, summary) = if set {
        (
            "source compilation-policy set",
            "merl source compilation-policy set <binding> --project <id> --database <path> --id <request> --actor <administrator> --expected-version <version> [--mode <capture_only|on_demand|eager> --coverage <optional|required>] --reason <text> [--kind <issue|issue_comment>] [--actor-class <human|bot|routine_agent|unknown>] [--override] [--remove] [--provider-actor <provider-id>] [--json]",
            "Set defaults or an exact selector with --mode and --coverage; --override takes precedence over selectors. --provider-actor with --actor-class classifies a stable provider account without mode or coverage. --remove restores fallback for a selector, override, or account mapping. Earlier captures keep their policy; use source require or source compile for historical work.",
        )
    } else {
        (
            "source compilation-policy",
            "merl source compilation-policy <binding> --project <id> --database <path> [--version <source-version>] [--json]",
            "Inspect defaults, selectors, account mappings, and accepted provenance. --version explains an immutable capture. Precedence: project override, kind and class, kind, class, defaults. Unknown authors use the unknown class.",
        )
    };
    let trust = set.then_some(crate::security::ACTOR_CLAIM);
    if json_output {
        render_json(
            &json!({"trust_boundary":trust,"schema":"merl.help/v1","command":command,"usage":usage,"summary":summary,"related":["source compilation-policy set","issue capture","source require","source compile"],"outcomes":["accepted","rejected","conflict"],"errors":["INVALID_INPUT","BINDING_NOT_FOUND","POLICY_INPUT_CONFLICT","POLICY_CONFLICT","STORAGE_ERROR"]}),
        )
    } else {
        Ok(format!(
            "{usage}\n\n{summary}\n{}\n",
            trust.unwrap_or_default()
        ))
    }
}

fn inspect(
    store: &Store,
    project: &merl_core::ProjectId,
    binding: &SourceBindingId,
    source_version: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    let state = store.binding_policy(project, binding)?;
    if let Some(version) = source_version {
        let version = merl_core::SourceVersionId::try_from(version)
            .map_err(|e| invalid_input(&e.to_string()))?;
        let source = store
            .source_version(project, &version)?
            .ok_or_else(|| invalid_input("source version does not exist"))?;
        if source.binding != *binding {
            return Err(invalid_input("source version belongs to another binding"));
        }
        let selection = store.source_policy_selection(project, &version)?;
        let result = json!({"schema":"merl.binding-policy-selection/v1","project":project.as_str(),"binding":binding.as_str(),"source_version":version.as_str(),"kind":source.kind.as_str(),"policy":{"mode":source.compilation_mode.as_str(),"coverage":source.coverage_requirement.as_str(),"version":source.policy_version.as_str()},"selection":selection});
        return if json_output {
            render_json(&result)
        } else {
            Ok(format!(
                "Source {version}: {}, {}, policy {}.\nSelection: {}\n",
                source.compilation_mode.as_str(),
                source.coverage_requirement.as_str(),
                source.policy_version,
                result["selection"]
            ))
        };
    }
    let change = state.change.as_ref().map(|object| {
            let evaluation = store.object_policy_evaluation(project, &object.id)?.ok_or(merl_store::StoreError::CorruptHistory)?;
            let evaluation = store.policy_evaluation(project, &evaluation)?.ok_or(merl_store::StoreError::CorruptHistory)?;
            Ok::<_, merl_store::StoreError>(json!({"object":object.id.as_str(),"revision":object.project_revision.get(),"actor":evaluation.actor.as_str(),"evaluation":evaluation.id.as_str(),"authorization_policy":evaluation.version.as_str(),"reason":object.payload.as_ref().map(merl_core::PayloadId::as_str)}))
        }).transpose()?;
    if json_output {
        render_json(
            &json!({"schema":"merl.binding-policy/v1","action":"source.compilation-policy","project":project.as_str(),"binding":binding.as_str(),"policy":{"mode":state.policy.mode.as_str(),"coverage":state.policy.coverage.as_str(),"version":state.policy.version.as_str()},"change":change,"selectors":state.selectors}),
        )
    } else {
        let provenance = change.map_or_else(
            || "Established by initial capture.".to_owned(),
            |change| {
                format!(
                    "Changed by {} at revision {}; evaluation {}; reason {}.",
                    change["actor"].as_str().unwrap_or(""),
                    change["revision"],
                    change["evaluation"].as_str().unwrap_or(""),
                    change["reason"].as_str().unwrap_or("")
                )
            },
        );
        Ok(format!(
            "Binding {binding}: {}, {}, version {}.\n{provenance}\nSelectors: {}\n",
            state.policy.mode.as_str(),
            state.policy.coverage.as_str(),
            state.policy.version,
            serde_json::to_string(&state.selectors).map_err(|e| invalid_input(&e.to_string()))?
        ))
    }
}
