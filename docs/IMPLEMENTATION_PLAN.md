# Implementation Plan

Recorded: 2026-11-03 (Asia/Shanghai).

This is a plan, not a record of completed implementation or verification. The current repository contains documentation only.

## Phase 1: Resolve feasibility boundaries

Run the device and upstream investigations as independent workstreams.

### Device workstream

- Identify the exact PW5, PW6, and Scribe models and firmware releases to support.
- Select a candidate Rust, Slint, Kindle backend, and cross-compilation combination.
- Produce an independent launcher and application process.
- Establish Chinese rendering and input, including composition, candidates, and search submission.
- Exercise a fixed-geometry image list, pagination, touch, scaling, and rotation where required.
- Confirm background-to-UI event delivery.
- Confirm sleep/resume and restoration of the stock UI after exit or failure.

Acceptance evidence must name the device, firmware, dependency revisions, procedure, result, and known limitations. Emulator results and author-reported community results remain separate from this project's physical-device results.

### Upstream workstream

- Confirm the source and lifecycle of cover URLs.
- Determine the actual login and session model, if account access is required.
- Establish catalog, search, and detail capabilities without inventing endpoint contracts.
- Determine whether content is delivered as files, another reading interface, or a capability unavailable to this application.
- Identify supported reading formats and any required handoff.
- Determine whether a callback, proxy, conversion, or synchronization requirement genuinely needs our own service.

Passing cover retrieval alone does not pass content-acquisition feasibility.

## Phase 2: Build the bookstore minimum

- Implement the project-owned Slint component foundation.
- Add browsing, search, book details, and account state for confirmed upstream capabilities.
- Add SQLite metadata and a bounded local cover/thumbnail cache.
- Preserve search, filter, pagination, selection, and return context.
- Add useful offline and failure states for locally available metadata and images.
- Include download and reader-handoff functionality only for confirmed capabilities.

## Phase 3: Make interruption behavior reliable

- Persist supported task state and recover after interruption.
- Support cancellation, retries, and resumption where the upstream transport allows them.
- Handle incomplete or stale cache entries safely.
- Confirm TLS behavior, image memory use, sleep during network work, and exit cleanup on target devices.
- Establish model-specific refresh and input settings from evidence.

## Phase 4: Freeze and package

- Freeze the supported model/firmware matrix and dependency revisions.
- Record the project release model and compatible dependency licenses.
- Produce a KPM package with an independent ID, version, launcher, icon, installation behavior, and upgrade policy.
- Keep user settings and local state separate from replaceable application files.
- Document installation, upgrades, removal, recovery, and known limitations.

## Verification environment

Follow the repository's AGENTS.md. Local tests, builds, validation suites, smoke tests, and runtime probes require explicit authorization in the current task. Otherwise use the designated remote environment, and report a blocked environment rather than silently switching to local verification.

Device-only behavior requires an explicitly identified target Kindle or an appropriate device-access setup. No such setup or acceptance result is claimed by this plan.
