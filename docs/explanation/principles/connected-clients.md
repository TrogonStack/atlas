# Clients stay connected; there is no offline mode

Part of the [Atlas principles](README.md).

Atlas's workflow depends on access to the backend and the AI services doing
the work. The browser and other presentation clients are thin clients:
they render backend information and help people communicate intent to AI.
They do not own a separate model, run an independent mutation workflow, or
queue model edits to reconcile after reconnecting.

The stack and its AI models may be hosted locally. A client connected to a
local backend still follows the same architecture: state, validation, and
execution remain backend responsibilities. Connected use requires access
to those services, whether they run locally or remotely.

When connectivity is lost, the interface must make its state clear. Cached
information remains a projection of an earlier backend state and cannot
claim to be current. Local file utilities or presentation caches do not
establish an offline product mode.

Backend behavior, validation, and change coordination belong behind the
API. Client code serves presentation and communication. See the
[Studio contract](../../../tsworkspace/webapps/studio/README.md) and
[shared client responsibilities](../../../rsworkspace/crates/trogon-atlas-client/README.md).
