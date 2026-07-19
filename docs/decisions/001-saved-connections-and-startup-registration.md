# ADR-001: Saved connections and native startup registration

## Status

Accepted

## Date

2026-07-15

Amended 2026-07-16

## Context

KeyPeek currently keeps one connection recipe in memory. It cannot reconnect after an app restart, prioritize multiple keyboards, or launch itself at user login. The UI is shared across macOS, Windows, and Linux through egui, while startup registration is platform-specific.

## Decision

- Persist successful connection recipes with stable device identities as `SavedConnection` values in the existing settings file. Allow manual ZMK serial connections without a USB serial number, but do not persist their reusable port names as identities.
- Create or update a saved connection only after a successful connection.
- Deduplicate with exact protocol-owned identities: canonical QMK JSON path plus VID/PID, Vial keyboard UID, or exact ZMK BLE/serial identity. Do not use fuzzy or VID/PID-only ZMK matching.
- Auto-connect enabled connections in priority order for five rounds, waiting three seconds between rounds.
- Listen for native wake and unlock notifications behind one shared lifecycle signal, refresh discovery, and start a fresh reconnect cycle. Preserve an explicit manual disconnect across resume events.
- Support last-connected and manual ordering. Manual ordering uses egui drag and drop.
- Store a 32-bit visible-layer mask with each saved connection. Existing connections default to all layers, and reconnects preserve the user's mask.
- Refresh device discovery asynchronously whenever Settings opens and from an explicit refresh control.
- Implement startup registration behind one platform interface: `SMAppService` on macOS, the per-user Run registry key on Windows, and an XDG autostart desktop entry on Linux.
- Show startup registration only when the platform backend reports that the current executable is installed in a stable, supported form. macOS additionally requires a signed app bundle.

## Alternatives considered

### Generic startup-registration crate

Rejected. The native implementations are small, and direct ownership preserves platform-specific status and error reporting.

### Fuzzy connection matching

Rejected. Transport or VID/PID fallbacks can merge unrelated devices. Exact identities may leave a stale entry after a real transport identity change, but never silently target another device.

### Separate profile database

Rejected. The data is small and belongs with the existing user settings. A second storage system would add migration and synchronization complexity.

### UI scheduling-gap detection

Rejected. A lock screen does not reliably suspend the app, and wall-clock changes can look like scheduling gaps. Native listeners provide explicit events: `NSWorkspace` on macOS, power/session messages on Windows, and logind D-Bus signals on Linux.

## Consequences

- Existing settings remain compatible because new fields have defaults.
- Layer visibility follows the saved keyboard identity rather than applying one global layer-number preference to every keyboard.
- QMK JSON files remain external source artifacts; KeyPeek stores only their canonical paths.
- Two QMK devices with the same VID/PID and canonical JSON path intentionally share one saved connection recipe.
- Linux and Windows startup registration do not pretend to have macOS-style signing semantics.
- The fixed Settings window needs vertical scrolling as the connection section grows.
- Platform notifications are coalesced before the UI refreshes discovery and restarts reconnect attempts.
