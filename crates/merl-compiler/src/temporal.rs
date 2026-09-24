//! Validate expression spans before resolving against immutable source metadata.

use crate::{Assertion, CompileError, source_span_valid};
use merl_core::temporal::{AuthorTime, Deferral, TemporalResult, TemporalRole};
use serde_json::Value;

pub(super) fn normalize(
    assertion: &Assertion,
    rendered: &Value,
) -> Result<(Vec<TemporalResult>, Option<Deferral>), CompileError> {
    // One value per planning field keeps conflicting interpretations out of one assertion.
    if assertion.temporal.len() > merl_core::temporal::TEMPORAL_ROLE_COUNT {
        return Err(CompileError::InvalidResponse);
    }
    let mut roles = Vec::new();
    let valid_span = |start, end| {
        start >= assertion.span_start
            && end <= assertion.span_end
            && source_span_valid(rendered, &assertion.source, start, end)
    };
    let source = rendered["sources"]
        .as_array()
        .and_then(|sources| sources.iter().find(|s| s["id"] == assertion.source))
        .ok_or(CompileError::InvalidResponse)?;
    let basis: Option<AuthorTime> = serde_json::from_value(source["author_time"].clone())
        .map_err(|_| CompileError::InvalidResponse)?;
    let mut results = Vec::new();
    for expression in &assertion.temporal {
        expression
            .validate()
            .map_err(|_| CompileError::InvalidResponse)?;
        if roles.contains(&expression.role)
            || !valid_span(expression.span_start, expression.span_end)
        {
            return Err(CompileError::InvalidResponse);
        }
        roles.push(expression.role);
        results.push(
            expression.normalize(
                assertion
                    .source
                    .clone()
                    .try_into()
                    .map_err(|_| CompileError::InvalidResponse)?,
                basis.as_ref(),
            ),
        );
    }
    if let Some(deferral) = &assertion.deferral
        && (assertion.predicate != "task"
            || !valid_span(deferral.reason.start, deferral.reason.end)
            || !roles.contains(&TemporalRole::ReviewAt)
                && !roles.contains(&TemporalRole::StartAfter))
    {
        return Err(CompileError::InvalidResponse);
    }
    Ok((results, assertion.deferral.clone()))
}
