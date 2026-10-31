# Declare reliability

Declare reliability overlays with put_entity_json, then run
validate_event_model on the model the indicators point into and fix all Errors
and Warnings.

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
