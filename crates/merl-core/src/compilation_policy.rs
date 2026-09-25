//! Bounded Issue policy selection from provider metadata and administrator choices.

use crate::{CompilationMode, CoverageRequirement, InvalidIdentifier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Author classification for compilation cost and coverage, independent of authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorClass {
    /// GitHub identifies the author as a User.
    Human,
    /// GitHub identifies the author as a Bot.
    Bot,
    /// An administrator identifies this provider account as routine agent traffic.
    RoutineAgent,
    /// The provider omitted the author or supplied an unrecognized type.
    #[default]
    Unknown,
}

impl ActorClass {
    /// Classifies GitHub's author type without consulting its login or prose.
    #[must_use]
    pub fn from_github_type(value: Option<&str>) -> Self {
        match value {
            Some("User") => Self::Human,
            Some("Bot") => Self::Bot,
            _ => Self::Unknown,
        }
    }
}

/// Prose surfaces supported by the first-release Issue workflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceKind {
    /// The Issue description.
    Issue,
    /// A comment on the Issue.
    IssueComment,
}

/// Independent timing and completeness choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyValues {
    /// Timing of compiler work.
    pub mode: CompilationMode,
    /// Requirement for a semantic-completeness claim.
    pub coverage: CoverageRequirement,
}

/// Exact structural selectors; an absent dimension matches any value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Selector {
    /// Issue surface; other source kinds cannot match these rules.
    pub kind: Option<IssueSourceKind>,
    /// Author class after project configuration takes effect.
    pub actor_class: Option<ActorClass>,
}

/// One rule with a unique selector within its binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Exact match dimensions.
    pub selector: Selector,
    /// Both policy fields selected by this rule.
    pub policy: PolicyValues,
}

/// Which authority input supplied the author class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// A provider User or Bot type.
    Provider,
    /// A binding-scoped administrator mapping of a stable provider ID.
    ProjectConfiguration,
    /// Missing or unrecognized provider metadata.
    Unknown,
}

/// The winning level of the fixed precedence order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionRule {
    /// Binding defaults after no selector matched.
    BindingDefault,
    /// A class-only selector.
    ActorClass,
    /// A kind-only selector, which takes precedence over class-only rules.
    Kind,
    /// An exact kind and class selector.
    KindActorClass,
    /// An administrator override for this project's binding.
    ProjectOverride,
    /// A provider deletion carries no prose to compile.
    ObservedDeletion,
}

/// Capture-time explanation retained even after rules or account metadata change.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicySelection {
    /// Author class used for this observation.
    pub actor_class: ActorClass,
    /// Trusted source of that classification.
    pub classification: Classification,
    /// Winning rule level.
    pub rule: SelectionRule,
    /// Matching selector, absent for defaults, overrides, and deletions.
    pub selector: Option<Selector>,
}

/// One administrative edit to a binding's policy configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum PolicyEdit {
    /// Replace defaults while retaining selectors and classifications.
    #[default]
    Defaults,
    /// Replace the exact selector, or remove it to restore fallback.
    Rule {
        /// Match dimensions; at least one must be present.
        selector: Selector,
        /// Remove this selector instead of assigning values.
        remove: bool,
    },
    /// Replace or remove the binding-scoped project override.
    Override {
        /// Remove the override and resume selector resolution.
        remove: bool,
    },
    /// Map a stable provider author ID, or remove its existing mapping.
    Classify {
        /// Provider ID, never a display login.
        provider_actor: String,
        /// New class, or no class to resume provider classification.
        actor_class: Option<ActorClass>,
    },
}

/// Immutable selector snapshot attached to an accepted binding policy version.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selectors {
    rules: Vec<Rule>,
    actors: BTreeMap<String, ActorClass>,
    project_override: Option<PolicyValues>,
}

impl Selectors {
    /// Applies one edit; the finite selector space permits at most fourteen rules.
    ///
    /// # Errors
    /// Rejects empty selectors and invalid provider IDs. At most 128 account mappings
    /// fit in one binding so an administrator request cannot grow capture work unbounded.
    pub fn apply(
        &mut self,
        edit: &PolicyEdit,
        policy: PolicyValues,
    ) -> Result<(), InvalidIdentifier> {
        match edit {
            PolicyEdit::Defaults => {}
            PolicyEdit::Rule { selector, remove } => {
                if selector.kind.is_none() && selector.actor_class.is_none() {
                    return Err(InvalidIdentifier);
                }
                self.rules.retain(|rule| rule.selector != *selector);
                if !remove {
                    self.rules.push(Rule {
                        selector: *selector,
                        policy,
                    });
                }
            }
            PolicyEdit::Override { remove } => self.project_override = (!remove).then_some(policy),
            PolicyEdit::Classify {
                provider_actor,
                actor_class,
            } => {
                // GitHub node IDs are opaque printable values. Match the source store's
                // 512-byte identity limit; neither logins nor payload text enter this map.
                if provider_actor.is_empty()
                    || provider_actor.len() > 512
                    || !provider_actor.bytes().all(|b| b.is_ascii_graphic())
                {
                    return Err(InvalidIdentifier);
                }
                if let Some(class) = actor_class {
                    // First-release configuration allows 128 exceptional accounts per
                    // binding; adding an account cannot grow the snapshot without limit.
                    const MAX_ACTOR_MAPPINGS: usize = 128;
                    if self.actors.len() >= MAX_ACTOR_MAPPINGS
                        && !self.actors.contains_key(provider_actor)
                    {
                        return Err(InvalidIdentifier);
                    }
                    self.actors.insert(provider_actor.clone(), *class);
                } else {
                    self.actors.remove(provider_actor);
                }
            }
        }
        Ok(())
    }

    /// Resolves a complete pair using override, exact, kind, class, then defaults.
    #[must_use]
    pub fn resolve(
        &self,
        defaults: PolicyValues,
        kind: Option<IssueSourceKind>,
        provider_actor: Option<&str>,
        provider_class: ActorClass,
    ) -> (PolicyValues, PolicySelection) {
        let configured = provider_actor.and_then(|id| self.actors.get(id)).copied();
        let actor_class = configured.unwrap_or(provider_class);
        let classification = if configured.is_some() {
            Classification::ProjectConfiguration
        } else if provider_class == ActorClass::Unknown {
            Classification::Unknown
        } else {
            Classification::Provider
        };
        let mut selection = PolicySelection {
            actor_class,
            classification,
            rule: SelectionRule::BindingDefault,
            selector: None,
        };
        // Rules govern Issue prose. Ingestion handles deletion as a deterministic event.
        if kind.is_none() {
            return (defaults, selection);
        }
        if let Some(policy) = self.project_override {
            selection.rule = SelectionRule::ProjectOverride;
            return (policy, selection);
        }
        for (selector, level) in [
            (
                Selector {
                    kind,
                    actor_class: Some(actor_class),
                },
                SelectionRule::KindActorClass,
            ),
            (
                Selector {
                    kind,
                    actor_class: None,
                },
                SelectionRule::Kind,
            ),
            (
                Selector {
                    kind: None,
                    actor_class: Some(actor_class),
                },
                SelectionRule::ActorClass,
            ),
        ] {
            if let Some(rule) = self.rules.iter().find(|rule| rule.selector == selector) {
                selection.rule = level;
                selection.selector = Some(selector);
                return (rule.policy, selection);
            }
        }
        (defaults, selection)
    }
}
