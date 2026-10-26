# ADR 0006: Reusable user setup and opt-in mailbox defaults

Status: accepted

Date: 2026-10-26

## Decision

[#155](https://github.com/infagent/merl/issues/155) adds `merl init`, reused by the verified binary installer's optional terminal offer. Setup is user-global only. It embeds the exact release skill, installs selected local Codex/Claude skills, chooses a durable mailbox directory, and saves one known user settings file. No nearest-project discovery or native host configuration editing is introduced.

This extends ADR 0004's explicit-home contract: mail and lane commands may use an opt-in saved mailbox default. An explicit `--home` bypasses all settings reads, including a corrupt `--config`. Otherwise `--config` selects a file, or one platform user location is checked. Invalid settings fail before opening a mailbox. Help, version and unknown commands remain independent of settings and runtime.

Only controlling-terminal input can accept interactive setup; script stdin stays available to `curl | sh`. JSON never prompts. Unattended installation defaults to binary-only; explicit setup requires noninteractive initializer choices. Verification and atomic binary replacement finish before setup. Setup failure leaves that verified binary installed and reports unfinished configuration.

## Ownership and recovery

Preflight checks all destinations before mutation. A setup-specific receipt contains generated expected bytes, ownership and pending publication identity, never backups of unrelated host files. A stable settings-scoped lock serializes setup. Atomic staged publications follow durable intent; file identity recognizes a publication interrupted before acknowledgment. Repeating the same initializer options resumes it. Pre-existing identical skills stay borrowed through retries and recovery; edits conflict even with `--yes`. Moving a recorded host destination requires uninstalling that host first.

Uninstall removes only unchanged owned files and empty directories whose recorded identity still matches. It preserves every mailbox directory and descendant, borrowed files and edits. A zero-byte coordination lock remains beside the selected config to prevent unlink/open-fd lock races; its directory may therefore remain. Setup never recursively deletes user directories. Unknown staging files are preserved; staging attempts and receipt reads are bounded. Filesystems must support Unix identity, hard links, atomic rename and sync; failures are explicit, with no weaker fallback.

## Dispatcher boundary

The optional offer is **show dispatcher start instructions**, with `not_started` status, or `missing_codex` when no real executable is detected. There is no service manager, automatic process start, guessed session or binding replacement. Claude skill setup does not imply Claude wake support. Existing runtime migration, background lifecycle and additional adapters remain separate work.

## Validation

Public CLI/API and synthetic installer-terminal scenarios cover default resolution, exact skills, pure preview, local conflicts, borrowed ownership, cancellation/recovery, upgrades, scoped uninstall, preserved mail and piped script input. Package smoke explicitly chooses binary-only installation on all four native platforms. No real user setup or live runtime transition is part of implementation checks.
