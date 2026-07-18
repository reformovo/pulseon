# ADR 0012: Defer Remote Training Service

## Status
Accepted.

## Context
`docs/drafts/remote-training-architecture-notes.md` describes a stable PulseOn
control service that owns the catalog, run lifecycle, and writer lock so an
ephemeral rented GPU machine no longer carries essential project state. The
draft is exploratory and lists six open questions, including whether the service
is self-hosted or managed, whether live observation is required, and what metric
loss window is acceptable.

The current reality is: PulseOn has a single user, training runs on local Apple
Silicon MLX, and no active GPU rental workflow yet. Remote training solves a real
but not-yet-arrived pain. Building it now would commit the largest architectural
bet in the project (new process, network protocol, auth, distributed state)
against an unvalidated need, while the local training experience is still being
completed.

## Decision
Remote training service delivery is deferred until the local training experience
is complete and a real rented-GPU workflow exists. The 0.2.x release focuses on
the desktop curve viewer and the comparison alignment semantics that feed it.

This is a deliberate "not now," not a "never." When local training is mature and
rented-GPU use is real, revisit `docs/drafts/remote-training-architecture-notes.md`
and produce a remote training ADR before adding remote writers or shared catalog
coordination.

## Considered Options
- **Build remote training now.** Rejected: highest-risk bet, six unresolved open
  questions, no active rented-GPU workflow to validate against, and single-owner
  attention is better spent completing the local analysis experience first.
- **Produce a remote training architecture ADR now, without code.** Rejected as
  premature: an architecture ADR for a service not yet needed would lock
  hard-to-reverse decisions (protocol shape, auth model, catalog ownership)
  against an unvalidated pain. Revisit when the pain is real.

## Consequences
- 0.2.x does not add a remote client mode, a control service, or shared catalog
  coordination.
- The native single-writer contract and local-first storage boundary remain the
  only supported mode.
- The roadmap's "Evaluate the remote control-service boundary" item stays open
  but is not a 0.2.x deliverable.
- When remote training is revisited, the existing draft and this ADR's deferral
  rationale are the starting context, so the decision is not relitigated from
  scratch.