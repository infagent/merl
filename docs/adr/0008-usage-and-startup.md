# ADR 0008: Short startup guidance and safe canonical skill setup

Status: accepted scope for #163

Date: 2026-10-27

## Decision

Add a pure `merl usage` primer and optional short `merl` skill. The skill reads invocation text, reuses explicit mailbox/scope/identity and saved guidance, and loads only relevant CLI contracts. Fresh members need supplied starting guidance; roles remain optional labels. Startup grants no authority, session replacement, spawning or dispatcher start. The merged scope/discovery/mail features are sufficient; notices and team-targeted lanes follow independently.

Fresh setup installs exact bundled canonical bytes at `merl/SKILL.md`. Archives retain unchanged `merl-mail` compatibility bytes too. Extend ADR 0006 only for the known sibling destination change within the selected host root; preserve recorded custom roots. Other moves still require uninstall. Retain legacy files and their ownership without rewriting or adopting them. Unmanaged legacy paths are metadata, not private-content backups. Modified skills or settings, borrowed settings that would change, symlinks and canonical collisions fail preflight.

Keep mail configuration `merl.config/v1`. Receipt `merl.init-receipt/v2` reuses the publication journal and adds at most one retained legacy path per supported host. Publish v2 intent before canonical directories/files and settings last. Old initializers reject that receipt before writes, including during pending intent; old mail tools can still use the saved home. New interrupted publication/removal resumes existing ownership and host associations.

Incomplete v1 setup does not migrate. Preserve its bytes and give a concrete new-binary safe uninstall/retry command; resuming with an original compatible initializer is optional. V1 uninstall keeps its receipt version until removal completes. No cross-version pending engine, generic alias framework, detach mode, force overwrite or private backup is introduced.

## Validation and consequences

Public regressions cover pure primer/help, canonical install, legacy/custom-root/ownership retention, preflight conflicts, interrupted v2 publication/removal and safe v1 recovery. Package checks compare both skill files to the frozen source; the embedded canonical bytes match installation. The released v0.3.0 initializer is separately checked against synthetic pending and completed v2 receipts, without changing real setup.

An isolated installed-Codex probe captures one rejecting loopback request and saves only Merl's own skill and synthetic invocation. It proves loading/argument preservation, not semantic execution or another host wake adapter. No inference or paid call occurs. CLI and ordinary CI remain independent of installed hosts. No global setup, runtime transition or release follows from this change.
