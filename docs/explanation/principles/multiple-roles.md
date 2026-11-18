# Roles and preferences shape the view

Part of the [Atlas principles](README.md).

People need different ways to understand the same product. A designer may
start with screens and journeys, an architect with context boundaries, and
a product engineer with commands, events, and scenarios. A participant's
role and preferences should shape the information presented and the depth
at which it is shown.

Interfaces and profiles should support that flexibility while preserving
shared identities, relationships, and meaning. Changing perspective
changes the projection, not the underlying facts. View preferences belong
to the participant and must not redefine the shared model for everyone
else.

Work undertaken for one role must respect the needs of the other roles and
preserve their shared contracts. For the scope being delivered, event-model
behavior, relationships, schemas, and scenarios must be modeled completely
and consistently. Validation and review must cover the impact on other
roles. Work is complete when that shared model is ready, rather than when
one role's projection looks finished.

An incomplete or broken graph must remain inspectable while work is in
progress. Its gaps, assumptions, and unresolved dependencies must stay
explicit and discoverable across affected views. Personalization must not
present unfinished model behavior as completed work. See
[layered information](layered-information.md) and
[Event Modeling](event-modeling.md).

A viewing profile and an authorization role answer different questions.
Personalization chooses a useful perspective; backend authorization
determines what the caller may access or change. Flexible views must honor
that boundary. See the [shared model contract](../../../proto/trogonatlas/eventmodel/v1alpha1/event_model.proto)
and [authorization](../authorization.md).
