//! Standalone restricted transport for synthetic probes and compiler wrappers.

use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
use merl_eval::transport::{RemoteTransport, TransportPin, TransportRole};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: String,
    role: TransportRole,
    worker_program: PathBuf,
    worker_program_sha256: String,
    transport: TransportPin,
    limits: ProcessLimits,
}

pub(super) fn command(args: &[String]) -> Result<(String, bool), String> {
    if args == ["--help"] {
        return Ok(("merl-eval transport call --plan PATH --plan-sha256 HEX\nRead one JSON request from stdin; return response and transport receipts, including on failure.".into(),true));
    }
    if args.len() != 5 || args[0] != "call" || args[1] != "--plan" || args[3] != "--plan-sha256" {
        return Err("usage: merl-eval transport call --plan PATH --plan-sha256 HEX".into());
    }
    let plan_bytes = read(
        std::fs::File::open(&args[2]).map_err(|_| "transport plan unreadable")?,
        1_048_576,
    )?;
    if super::digest(&plan_bytes).strip_prefix("sha256:")
        != Some(args[4].strip_prefix("sha256:").unwrap_or(&args[4]))
    {
        return Err("transport plan pin mismatch".into());
    }
    let plan: Plan = serde_json::from_slice(&plan_bytes).map_err(|_| "invalid transport plan")?;
    if plan.schema != "merl.transport-plan/v1" {
        return Err("unsupported transport plan".into());
    }
    let program =
        IsolatedProgram::new(&plan.worker_program, plan.limits).map_err(|e| e.to_string())?;
    if program.sha256()
        != plan
            .worker_program_sha256
            .strip_prefix("sha256:")
            .unwrap_or(&plan.worker_program_sha256)
    {
        return Err("transport worker pin mismatch".into());
    }
    let mut transport =
        RemoteTransport::open(&plan.transport, plan.role).map_err(|e| e.to_string())?;
    let bytes = read(std::io::stdin().lock(), plan.limits.request_bytes)?;
    let request: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid transport request")?;
    let mut calls = Vec::new();
    let response =
        program.recorded_remote_call(&BTreeMap::new(), &request, &mut calls, &mut transport);
    let success = response.is_ok();
    let result = json!({"schema":"merl.transport-execution/v1","plan_sha256":super::digest(&plan_bytes),
        "response":response.as_ref().ok(),"error":response.as_ref().err().map(merl_eval::isolation::IsolationError::code),
        "worker_calls":calls,"transport":transport.report()});
    serde_json::to_string_pretty(&result)
        .map(|text| (text, success))
        .map_err(|_| "transport report encoding".into())
}

fn read(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "transport input unreadable")?;
    if bytes.len() > limit {
        return Err("transport input exceeds byte limit".into());
    }
    Ok(bytes)
}
