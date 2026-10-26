//! Fixed read-only metadata queries over an explicitly selected authenticated gh.

use crate::{Gh, GitHub, LaneError, Query, Reply, failure, input, protocol};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

// Metadata pages have 50 timeline events / 20 thread tails. Two MiB leaves ample
// headroom for bounded labels/cursors while preventing an executable from flooding memory.
const MAX_REPLY_BYTES: u64 = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const FIELDS: &str = "number headRefOid state isDraft updatedAt labels(first:100){nodes{name} pageInfo{hasNextPage}}";
const TYPES: &str = "[PULL_REQUEST_COMMIT,ISSUE_COMMENT,PULL_REQUEST_REVIEW,PULL_REQUEST_REVIEW_THREAD,HEAD_REF_FORCE_PUSHED_EVENT,MERGED_EVENT,CLOSED_EVENT,REOPENED_EVENT,CONVERT_TO_DRAFT_EVENT,READY_FOR_REVIEW_EVENT,LABELED_EVENT,UNLABELED_EVENT,REVIEW_DISMISSED_EVENT,REVIEW_REQUESTED_EVENT,REVIEW_REQUEST_REMOVED_EVENT]";
const EVENTS: &str = "pageInfo{endCursor hasNextPage} edges{cursor node{__typename}}";
const RATE: &str = "rateLimit{remaining resetAt}";

impl Gh {
    /// Configures the supported gh adapter with stable runtime working storage.
    ///
    /// Authentication remains in gh's existing configuration/environment. No token
    /// is copied into lane state, arguments, diagnostic records or Debug output.
    ///
    /// # Errors
    /// Rejects relative executable/runtime paths or an unavailable runtime directory.
    pub fn new(executable: &Path, home: &Path) -> Result<Self, LaneError> {
        if !executable.is_absolute() || !home.is_absolute() {
            return Err(input("gh and runtime paths must be absolute"));
        }
        Ok(Self {
            executable: executable.into(),
            home: home.canonicalize()?,
        })
    }
}

impl GitHub for Gh {
    fn fetch(
        &mut self,
        query: &Query,
        clock: &impl Fn() -> Result<i64, LaneError>,
    ) -> Result<Reply, LaneError> {
        let mut child = Command::new(&self.executable)
            .current_dir(&self.home)
            .args([
                "api",
                "--hostname",
                "github.com",
                "--include",
                "graphql",
                "--input",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                failure(
                    "LANE_GITHUB",
                    "configured gh could not start; checkpoint retained",
                )
            })?;
        let request = request(query).to_string();
        if child
            .stdin
            .take()
            .ok_or_else(protocol)?
            .write_all(request.as_bytes())
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("LANE_GITHUB", "gh request write failed"));
        }
        let stdout = child.stdout.take().ok_or_else(protocol)?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_REPLY_BYTES + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let bytes = reader
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        if bytes.len() as u64 > MAX_REPLY_BYTES {
            return Err(failure(
                "LANE_GITHUB_PROTOCOL",
                "gh metadata response exceeded two MiB; checkpoint retained",
            ));
        }
        if status.is_none() {
            return Err(failure(
                "LANE_GITHUB",
                "gh deadline or process failure; checkpoint retained",
            ));
        }
        decode(&bytes, clock()?, status.is_some_and(|s| s.success()))
    }
}

fn request(query: &Query) -> Value {
    match query {
        Query::Discover {
            repository,
            labels,
            after,
        } => {
            let (owner, repo) = repository.parts();
            let template = "query Discover($owner:String!,$repo:String!,$labels:[String!]!,$after:String){repository(owner:$owner,name:$repo){pullRequests(states:OPEN,labels:$labels,first:50,after:$after,orderBy:{field:CREATED_AT,direction:ASC}){pageInfo{endCursor hasNextPage} edges{cursor node{FIELDS timelineItems(last:1,itemTypes:TYPES){EVENTS}}}}} RATE}";
            json!({"operationName":"Discover","query":template.replace("FIELDS",FIELDS).replace("TYPES",TYPES).replace("EVENTS",EVENTS).replace("RATE",RATE),"variables":{"owner":owner,"repo":repo,"labels":labels.as_slice(),"after":after}})
        }
        Query::Observe {
            repository,
            number,
            after,
            threads_after,
            resync,
        } => {
            let (owner, repo) = repository.parts();
            let template = "query OPERATION($owner:String!,$repo:String!,$number:Int!,AFTER_VARIABLE$threadsAfter:String){repository(owner:$owner,name:$repo){pullRequest(number:$number){FIELDS timelineItems(PAGE_ARGS,itemTypes:TYPES){EVENTS} reviewThreads(first:20,after:$threadsAfter){pageInfo{endCursor hasNextPage} edges{cursor node{id isResolved comments(last:1){totalCount nodes{id updatedAt url}}}}}}} RATE}";
            let name = if *resync { "Resync" } else { "Observe" };
            let document = template
                .replace("OPERATION", name)
                .replace(
                    "AFTER_VARIABLE",
                    if *resync { "" } else { "$after:String," },
                )
                .replace(
                    "PAGE_ARGS",
                    if *resync {
                        "last:1"
                    } else {
                        "first:50,after:$after"
                    },
                )
                .replace("FIELDS", FIELDS)
                .replace("TYPES", TYPES)
                .replace("EVENTS", EVENTS)
                .replace("RATE", RATE);
            let mut variables =
                json!({"owner":owner,"repo":repo,"number":number,"threadsAfter":threads_after});
            if !resync {
                variables["after"] = json!(after);
            }
            json!({"operationName":name,"query":document,"variables":variables})
        }
    }
}

fn decode(bytes: &[u8], now: i64, success: bool) -> Result<Reply, LaneError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| protocol())?
        .replace("\r\n", "\n");
    let (head, body) = text.split_once("\n\n").ok_or_else(|| {
        failure(
            "LANE_GITHUB",
            "gh returned no HTTP metadata; checkpoint retained",
        )
    })?;
    if head.len() > 16_384 {
        return Err(protocol());
    }
    let status = head
        .lines()
        .next()
        .and_then(|s| s.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(protocol)?;
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|s| s.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let document: Value = serde_json::from_str(body).map_err(|_| protocol())?;
    let errors = document
        .get("errors")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty());
    let rate_message = |value: &Value| {
        value
            .as_str()
            .is_some_and(|s| s.to_ascii_lowercase().contains("rate limit"))
    };
    let rate = status == 429
        || rate_message(&document["message"])
        || status == 403
            && (headers.contains_key("retry-after")
                || headers
                    .get("x-ratelimit-remaining")
                    .is_some_and(|s| s == "0"))
        || errors.is_some_and(|e| {
            e.iter().any(|v| {
                v["type"] == "RATE_LIMITED"
                    || v["extensions"]["type"] == "RATE_LIMITED"
                    || rate_message(&v["message"])
            })
        })
        || document["data"]["rateLimit"]["remaining"].as_i64() == Some(0);
    let retry = retry_at(&headers, &document, now, rate)?;
    if rate {
        let mut error = failure(
            "LANE_RATE_LIMITED",
            "GitHub rate limit; progress retained until the server deadline",
        );
        error.retry_at = Some(retry.max(now.saturating_add(60_000)));
        return Err(error);
    }
    if let Some(errors) = errors {
        let cursor = errors.iter().any(|v| {
            v["type"] == "INVALID_CURSOR_ARGUMENTS"
                || v["message"].as_str().is_some_and(|s| {
                    let lower = s.to_ascii_lowercase();
                    lower.contains("cursor")
                        && (lower.contains("invalid") || lower.contains("not exist"))
                })
        });
        return Err(if cursor {
            failure(
                "LANE_CURSOR_GAP",
                "GitHub rejected a cursor; explicit resynchronization required",
            )
        } else {
            failure(
                "LANE_GITHUB",
                "GitHub GraphQL error or partial data; checkpoint retained",
            )
        });
    }
    if !success || !(200..300).contains(&status) {
        return Err(failure(
            "LANE_GITHUB",
            "GitHub request failed; checkpoint retained",
        ));
    }
    Ok(Reply {
        document,
        not_before: retry,
    })
}

fn retry_at(
    headers: &BTreeMap<String, String>,
    document: &Value,
    now: i64,
    rate: bool,
) -> Result<i64, LaneError> {
    let mut retry = 0;
    if let Some(value) = headers.get("retry-after") {
        retry = value
            .parse::<i64>()
            .ok()
            .filter(|s| *s >= 0)
            .map(|seconds| now.saturating_add(seconds.saturating_mul(1000)))
            .or_else(|| http_date(value))
            .ok_or_else(protocol)?;
    }
    if rate {
        if let Some(reset) = headers
            .get("x-ratelimit-reset")
            .and_then(|s| s.parse::<i64>().ok())
        {
            retry = retry.max(reset.saturating_mul(1000));
        }
        if let Some(reset) = document["data"]["rateLimit"]["resetAt"]
            .as_str()
            .and_then(utc_millis)
        {
            retry = retry.max(reset);
        }
    }
    Ok(retry)
}

fn utc_millis(text: &str) -> Option<i64> {
    if text.len() != 20
        || !text.is_ascii()
        || &text[4..5] != "-"
        || &text[7..8] != "-"
        || &text[10..11] != "T"
        || &text[13..14] != ":"
        || &text[16..17] != ":"
        || &text[19..] != "Z"
    {
        return None;
    }
    let year = text[..4].parse().ok()?;
    let month = text[5..7].parse().ok()?;
    let day = text[8..10].parse().ok()?;
    let hour = text[11..13].parse::<i64>().ok()?;
    let minute = text[14..16].parse::<i64>().ok()?;
    let second = text[17..19].parse::<i64>().ok()?;
    date_millis(year, month, day, hour, minute, second)
}

fn date_millis(
    year: i64,
    month: usize,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> Option<i64> {
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return None;
    }
    let lengths = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=lengths[month - 1]).contains(&day) {
        return None;
    }
    // This bounded conversion is used only for provider retry dates, not a clock.
    let days = (1970..year)
        .map(|y| if leap(y) { 366 } else { 365 })
        .sum::<i64>()
        + lengths[..month - 1].iter().sum::<i64>()
        + day
        - 1;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000)
}

fn http_date(text: &str) -> Option<i64> {
    let parts = text.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|s| *s == parts[2])?
        + 1;
    let time = parts[4]
        .split(':')
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if time.len() != 3 {
        return None;
    }
    date_millis(
        parts[3].parse().ok()?,
        month,
        parts[1].parse().ok()?,
        time[0],
        time[1],
        time[2],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_retry_dates_and_http_200_rate_metadata_preserve_the_longest_deadline() {
        let limited = json!({"errors":[{"type":"RATE_LIMITED"}]}).to_string();
        let metadata =
            json!({"data":{"rateLimit":{"remaining":0,"resetAt":"1970-01-01T00:10:01Z"}}})
                .to_string();
        for (headers, body, expected) in [
            ("Retry-After: 600", limited.as_str(), 601_001),
            (
                "Retry-After: Thu, 01 Jan 1970 00:10:01 GMT",
                limited.as_str(),
                601_000,
            ),
            ("X-RateLimit-Remaining: 1000", metadata.as_str(), 601_000),
            (
                "Retry-After: 600\r\nX-RateLimit-Reset: 900",
                limited.as_str(),
                900_000,
            ),
        ] {
            let raw = format!("HTTP/2.0 200 OK\r\n{headers}\r\n\r\n{body}");
            let error = decode(raw.as_bytes(), 1001, true).unwrap_err();
            assert_eq!(error.code, "LANE_RATE_LIMITED");
            assert_eq!(error.retry_at, Some(expected));
        }
        assert_eq!(utc_millis("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
        for (status, body) in [
            (
                "429 Too Many Requests",
                json!({"message":"Too Many Requests"}),
            ),
            (
                "200 OK",
                json!({"errors":[{"message":"secondary rate limit exceeded"}]}),
            ),
        ] {
            let raw = format!("HTTP/2.0 {status}\r\n\r\n{body}");
            let error = decode(raw.as_bytes(), 1001, status.starts_with("200")).unwrap_err();
            assert_eq!(error.code, "LANE_RATE_LIMITED");
            assert_eq!(error.retry_at, Some(61_001));
        }
        assert_eq!(utc_millis("2023-02-29T00:00:00Z"), None);
        assert!(
            retry_at(
                &[("retry-after".into(), "invalid".into())].into(),
                &Value::Null,
                1001,
                true
            )
            .is_err()
        );
    }
}
