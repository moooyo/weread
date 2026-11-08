# Repository Instructions

## Language

- Use Chinese for user-facing conversation, progress updates, questions, and final responses.
- Write code, code comments, and repository documentation entirely in English.

## Current scope

- Application implementation was authorized on 2026-11-03 using the supplied design handoff v0.2. Keep implementation status and outstanding acceptance gates explicit.
- Preserve the current decisions in README.md and docs/ARCHITECTURE.md.
- Record proposed choices and unresolved questions separately from accepted decisions and verified facts.
- Do not infer upstream book-download capabilities from cover-image availability.
- Do not present community project claims as verification performed by this project.

## Development and verification

- The operator uses Windows and PowerShell. Use valid PowerShell syntax for commands on the operator's machine.
- Do not run local tests, validation suites, build verification, smoke tests, or runtime probes unless the user explicitly authorizes local verification in the current task.
- Otherwise, use the designated remote environment through `ssh test-env` for verification that can run there.
- If the remote verification environment is unavailable, report verification as blocked rather than falling back to local verification.
- Device behavior must be assessed on an explicitly identified Kindle model and firmware combination. Keep device claims separate from emulator results.
- Ordinary source review, documentation editing, and Git operations are not application verification.

## Architecture boundaries

- The product is an independent Rust and Slint application, not a Jobman dependency or a KOReader plugin.
- Keep UI, application use cases, source adapters, persistence, and Kindle platform details separate.
- Maintain a replaceable Kindle platform boundary and pin any reused implementation to reviewed revisions.
- Use local cover caching. Do not add S3 or other server-side object storage without a demonstrated requirement and a recorded architecture decision.
- A custom backend and server database are conditional, not baseline requirements.

## Repository hygiene

- Never commit credentials, account tokens, session cookies, device identifiers, private user data, downloaded books, or research archives.
- Do not choose a project license, claim device compatibility, or freeze dependency versions without documenting the supporting decision and evidence.
