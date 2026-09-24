//! Resolve calendar expressions using captured evidence, without reading a clock.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use time::{Duration, OffsetDateTime, UtcOffset};

// Match the store's structural identifier bound so these references fit its records.
const MAX_REFERENCE_BYTES: usize = 128;

/// Number of planning fields one assertion can address, with one value per role.
pub const TEMPORAL_ROLE_COUNT: usize = 3;

/// A bounded structural reference; source prose belongs behind a payload reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct Reference(String);

impl TryFrom<String> for Reference {
    type Error = InvalidTemporal;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > MAX_REFERENCE_BYTES
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-/.:+".contains(&b))
        {
            return Err(InvalidTemporal);
        }
        Ok(Self(value))
    }
}
impl From<Reference> for String {
    fn from(value: Reference) -> Self {
        value.0
    }
}
impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Invalid timestamp, offset, reference, or source span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTemporal;
impl fmt::Display for InvalidTemporal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid temporal evidence")
    }
}
impl std::error::Error for InvalidTemporal {}

/// Upstream evidence for the local calendar at the time of authorship.
///
/// A provider's UTC transport timestamp alone does not establish the author's offset.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "AuthorTimeFields", into = "AuthorTimeFields")]
pub struct AuthorTime(AuthorTimeFields);
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AuthorTimeFields {
    utc_millis: i64,
    offset_seconds: Option<i32>,
    timezone: Option<Reference>,
}
impl TryFrom<AuthorTimeFields> for AuthorTime {
    type Error = InvalidTemporal;
    fn try_from(value: AuthorTimeFields) -> Result<Self, Self::Error> {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(value.utc_millis) * 1_000_000)
            .map_err(|_| InvalidTemporal)?;
        if let Some(offset) = value.offset_seconds {
            UtcOffset::from_whole_seconds(offset).map_err(|_| InvalidTemporal)?;
        }
        Ok(Self(value))
    }
}
impl From<AuthorTime> for AuthorTimeFields {
    fn from(value: AuthorTime) -> Self {
        value.0
    }
}
impl AuthorTime {
    /// Records an authored instant and the offset supplied by its source.
    ///
    /// The optional zone name preserves provenance. Calendar arithmetic uses the
    /// recorded offset, so future timezone-database changes cannot alter replay.
    /// # Errors
    /// Rejects unrepresentable timestamps, offsets outside a day, and invalid zone names.
    pub fn new(
        utc_millis: i64,
        offset_seconds: Option<i32>,
        timezone: Option<String>,
    ) -> Result<Self, InvalidTemporal> {
        AuthorTimeFields {
            utc_millis,
            offset_seconds,
            timezone: timezone.map(Reference::try_from).transpose()?,
        }
        .try_into()
    }
    /// Returns the authored instant for comparison with a captured source version.
    #[must_use]
    pub fn utc_millis(&self) -> i64 {
        self.0.utc_millis
    }
}

/// Half-open UTF-8 byte range in the assertion's captured source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    /// Inclusive start byte.
    pub start: usize,
    /// Exclusive end byte.
    pub end: usize,
}

/// A future condition, without a claim that the event has occurred.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventPredicate {
    /// Wait for the named provider pull request to merge.
    ProviderMerged {
        /// Provider-qualified PR identity, such as `github:example/parser/pull/229`.
        subject: Reference,
    },
    /// Wait for an accepted semantic object's resolution status to become resolved.
    ObjectResolved {
        /// Project-local semantic object identity.
        subject: Reference,
    },
}

/// The planning field to which an expression applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRole {
    /// Revisit planning on this date or after this event.
    ReviewAt,
    /// Execution must wait for this event.
    StartAfter,
    /// Requester's required date, without an owner commitment.
    NeededBy,
}

/// Compiler interpretation before deterministic source-time normalization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expression {
    /// Shift the author's local calendar date by a whole number of days.
    RelativeDate {
        /// Zero means today; one means tomorrow. No time of day is implied.
        days: i16,
    },
    /// Preserve a condition over an external or accepted semantic object.
    AfterEvent {
        /// Condition to reconsider; normalization does not evaluate it.
        predicate: EventPredicate,
    },
}

/// A typed temporal expression with an exact source span.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemporalExpression {
    /// Planning field named by this expression.
    pub role: TemporalRole,
    /// Inclusive expression byte offset in the assertion source.
    pub span_start: usize,
    /// Exclusive expression byte offset in the assertion source.
    pub span_end: usize,
    /// Interpretation; the compiler cannot supply its own clock basis.
    pub expression: Expression,
}

/// A normalization failure that retains the source instead of guessing a date.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedReason {
    /// The capture has no authored-time evidence.
    MissingAuthorTime,
    /// The authored instant has no recorded local offset.
    MissingTimezone,
    /// Calendar arithmetic would exceed the supported date range.
    DateOutOfRange,
}

/// A source-grounded date or an unevaluated event condition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TemporalValue {
    /// A local calendar date, without a time of day or UTC deadline.
    Date {
        /// ISO calendar date in YYYY-MM-DD form.
        date: String,
    },
    /// A condition whose satisfaction depends on provider or semantic state.
    Predicate {
        /// Event to wait for.
        predicate: EventPredicate,
    },
    /// Evidence cannot determine a date.
    Unresolved {
        /// Stable reason for leaving this value unresolved.
        reason: UnresolvedReason,
    },
}

/// Immutable result, including the expression and the exact captured clock basis.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemporalResult {
    /// Immutable source version containing the expression and reason spans.
    pub source: Reference,
    /// Interpretation and source range supplied by the compiler.
    pub original: TemporalExpression,
    /// Recorded source time, absent when the capture lacks it.
    pub basis: Option<AuthorTime>,
    /// Deterministic value or explicit unresolved outcome.
    pub value: TemporalValue,
}
impl TemporalExpression {
    /// Checks the source range and the reference shape required by this planning field.
    ///
    /// # Errors
    /// Rejects empty ranges, calendar dates in event-only fields, and unqualified PR identities.
    pub fn validate(&self) -> Result<(), InvalidTemporal> {
        if self.span_start >= self.span_end
            || self.role == TemporalRole::StartAfter
                && matches!(self.expression, Expression::RelativeDate { .. })
        {
            return Err(InvalidTemporal);
        }
        if let Expression::AfterEvent {
            predicate: EventPredicate::ProviderMerged { subject },
        } = &self.expression
        {
            let valid = subject
                .0
                .strip_prefix("github:")
                .and_then(|reference| reference.rsplit_once("/pull/"))
                .is_some_and(|(repository, number)| {
                    !repository.is_empty() && number.parse::<u64>().is_ok_and(|n| n > 0)
                });
            if !valid {
                return Err(InvalidTemporal);
            }
        }
        Ok(())
    }

    /// Resolves a calendar date or retains an event predicate using supplied evidence.
    #[must_use]
    pub fn normalize(&self, source: Reference, basis: Option<&AuthorTime>) -> TemporalResult {
        let value = match &self.expression {
            Expression::AfterEvent { predicate } => TemporalValue::Predicate {
                predicate: predicate.clone(),
            },
            Expression::RelativeDate { days } => normalize_date(*days, basis),
        };
        TemporalResult {
            source,
            original: self.clone(),
            basis: basis.cloned(),
            value,
        }
    }
}
fn normalize_date(days: i16, basis: Option<&AuthorTime>) -> TemporalValue {
    let Some(basis) = basis else {
        return TemporalValue::Unresolved {
            reason: UnresolvedReason::MissingAuthorTime,
        };
    };
    let Some(offset) = basis.0.offset_seconds else {
        return TemporalValue::Unresolved {
            reason: UnresolvedReason::MissingTimezone,
        };
    };
    let date =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(basis.0.utc_millis) * 1_000_000)
            .ok()
            .zip(UtcOffset::from_whole_seconds(offset).ok())
            .and_then(|(instant, offset)| instant.checked_to_offset(offset))
            .and_then(|local| local.date().checked_add(Duration::days(i64::from(days))));
    match date {
        Some(date) => TemporalValue::Date {
            date: date.to_string(),
        },
        None => TemporalValue::Unresolved {
            reason: UnresolvedReason::DateOutOfRange,
        },
    }
}

/// A proposed task deferral; review must authorize it before it becomes task state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deferral {
    /// Whether the source says the owner has accepted responsibility.
    pub accepted: bool,
    /// Exact source span explaining the delay.
    pub reason: Span,
}
