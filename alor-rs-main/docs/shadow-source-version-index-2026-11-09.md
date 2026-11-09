# Shadow Source And Admission Version Index

This is an engineering contract index, not an inventory of active VPS units,
an approval file or evidence that a candidate artifact is deployed.

| Line | Role and status |
|---|---|
| Production gateway/runtime | Existing live inputs and action-scoped broker path; unchanged by this repair. |
| Authenticated REST closed-M10 source | Historical isolated implementation. Its full-equality/revision rules are not the current WS contract. |
| WS source v1/v2 | Earlier compatibility identities; do not mutate their manifests to add newer policy fields. |
| WS source v3 | Explicit context finalization and bounded ordinary collection; unknown late history remains strict. |
| WS source v4 | Opt-in late-context audit-only policy plus separate as-observed revisions; no canonical back-insert or model replay. |
| Admission v1/v2 | Existing strict preparation/admission plans; do not accept v4 sources silently. |
| Admission v3, prepared template | Strict v4 snapshots and gates in ExecStartPre; separate legacy supervisor start, no durable grant/fence claim. |
| Admission v3, granted template | New opt-in create-only launcher: fresh preflight, durable supervisor grant, then worker. Not an automatic upgrade of existing units. |
| RI numerical context pair | Separate collector and model contract, not converted by the reaction-pair launcher. |

The November9 transport repair preserves source v4 wire/config semantics and
changes only MD-only handling of normal remote Close/EOF. It is a new executable
artifact requiring its own pinned hash and controlled qualification; identical
wire version does not authorize substituting it into a running identity. The
shared Trading scope preserves its previous normal-close behavior in this line.

Executables/scripts:

- `alor_shadow_md_ws`: standalone MD-only WS source, no CWS/account consumer.
- `reaction_shadow_worker`: numerical request/response process, no broker commands.
- `reaction_shadow_preflight.py`: read-only checks, no model creation or service start.
- `reaction_shadow_service.py`: existing supervised create/resume protocol.
- `reaction_shadow_launch.py`: opt-in v3 create grant and supervised startup.

Configuration templates are examples, not approved target settings. The granted
template has no timer, Install section, automatic restart or resume. Verify real
UIDs, source/worker ACLs, pinned scripts/binaries/configs and denied command writes
before any target activation. No credential file belongs in this repository.

Read next: [repair/grant](shadow-review-repair-and-grant-2026-11-09.md),
[late context](shadow-late-context-recovery-2026-11-08.md),
[paired admission](reaction-shadow-paired-admission-v2-2026-11-06.md),
[WS source](shadow-md-ws-source-runbook-2026-11-02.md).
