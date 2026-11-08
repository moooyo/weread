# Open Questions and Evidence Gates

Recorded: 2026-11-03 (Asia/Shanghai).

These are unresolved items, not reasons to change the accepted independent-client architecture without evidence.

## Product and source capabilities

- Is WeChat Read the source for catalog, search, and account data, or is its confirmed role limited to covers?
- What authentication and session behavior is actually available to the client?
- What are the real catalog, search, detail, and cover contracts and failure cases?
- Are cover URLs stable, temporary, authenticated, or refreshed through a separate request?
- What reading or acquisition capabilities are actually available for a user's books?
- Is offline content available in a form usable by the application, and what reader can consume it?
- Does any verified requirement need a project-owned service? If so, define that service's responsibility before choosing a database or deployment architecture.

## Devices and runtime

- Which Scribe generation or generations are required?
- Which exact firmware releases are in scope for PW5, PW6, and Scribe?
- Which jailbreak and application-launch environment is available on each target?
- Which Slint/backend revision supports Chinese composition and search input on those devices?
- What are the confirmed touch, scaling, orientation, refresh, and sleep behaviors?
- How will the application safely hand control to a reader and restore system state?
- Investigate the single first-start gzip trailer mismatch on emulator candidate `f603f71974bea950fd115d23ea4ae84ad76df181d817576f04980d1f9b2a8b92`. The identical file passed on retry, all source gzip checks passed, and the next immutable candidate completed six startup/interaction cases without recurrence. Preserve integrity checks; FUSE/mapping/cache or emulator transience remains a hypothesis, not a confirmed cause. See [the emulator record](EMULATOR.md).

## UI specification

- Finalize type roles, grayscale tokens, spacing, borders, hit areas, and scale conversion.
- Define component states and page templates in the project's Slint component library.
- Choose navigation and task views after the actual acquisition capabilities are known.
- Set image slots, cache variants, and responsive layouts from target-device prototypes.

The earlier UI principles and size suggestions are proposals. They are not a completed or validated Kindle design system.

## Release and dependency decisions

- Choose the project's open-source or closed-source release model and license.
- Review licenses of Slint, reused backend code, fonts, icons, and any other dependencies for the intended distribution.
- Freeze versions only after the target prototype passes its acceptance gates.
- Decide package ID, visible name, icon, update channel, user-data migration, and uninstall behavior.

## Current evidence limitation

The original planning questions above predate implementation. The user subsequently authorized design handoff v0.2; application code, source-derived protocol contracts, remote desktop verification, and an ARM development build are now present. See [Implementation status](IMPLEMENTATION_STATUS.md) and [Verification](VERIFICATION.md) for current evidence. Public community reports and our own desktop/cross-compilation results still do not establish physical-device compatibility or live-account feature availability. Exact firmware acceptance, release licensing, and unsupported hardware capabilities remain unresolved.
