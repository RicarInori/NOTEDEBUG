# Architecture Decision Records

The ADRs for this project live **inline in `docs/inkweave-dev-guide.md`**, in the order they were
made, because the guide is the single document an AI collaborator or a new contributor reads before
touching the tree. Duplicating them here would create two sources of truth that drift.

| # | Decision | Location |
| --- | --- | --- |
| ADR-001 | Flutter UI + PlatformView canvas | guide §1 |
| ADR-002 | Input path captured natively, bypassing Flutter | guide §1 |
| ADR-003 | First-party plugins are static crates (no dylib) | guide §1 |
| ADR-004 | Lock-free service resolution (`ArcSwap`) + `Arc` two-way hold | guide §1 |
| ADR-005 | PDFium handle pool + single writer thread | guide §1 |
| ADR-006 | Material Design 3 as the UI language | guide §1 |
| ADR-007 | "Hot load / shutdown" means mount, unmount, enable, disable — not code hot-swap | guide §1 |
| ADR-008 | The unified control panel is a preset internal module | guide §1 |

## Adding one

The guide is a living document: a change to an ADR must be written back into the guide with a version
bump in its revision log. Concretely:

1. Append the ADR to §1 with the next free number, including the alternatives considered and why
   they were rejected.
2. Add a row to the index in §12.2.
3. Add a row to the revision log at the top, dated.
4. If the decision supersedes an earlier one, mark the old ADR superseded rather than deleting it —
   the reasoning trail is the valuable part.