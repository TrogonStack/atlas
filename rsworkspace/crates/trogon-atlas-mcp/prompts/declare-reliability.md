# Declare reliability

Declare reliability overlays with put_entity_json and read the mutation's
`issues` array. Reliability overlays are not EventModel members, so
single-model validation does not cover them. Run validate_project for the
namespace that contains the connected EventModel, then fix all Errors and
Warnings in its reports.

- Reliability overlays point outward like Tracker, never as event model
  members: a ServiceLevelIndicator names a Connection (CommandHandling,
  Projection, Reaction, Display, Integration, or Journey) that must resolve to
  a real slice or edge; a ServiceLevelObjective points at one indicator,
  carries the threshold and target, and lists the AlertPolicy entities that
  watch it; an AlertPolicy points at AlertNotificationTarget entities.
  Validation checks the connection resolves, a Journey is structurally
  reachable, correlation fields are sourced on both ends, the signal matches
  the measure kind, and an objective with no alert policy is flagged (warning
  when externally committed, info otherwise).
