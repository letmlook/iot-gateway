# Alarm Events Page Design

Date: 2026-05-30

## Goal

Make persisted alarm events visible as a standalone product capability for the v0.6 closed-loop roadmap.

The backend already has a basic alarm lifecycle: flow-generated alarm events are persisted to SQLite, listed through REST, acknowledged/resolved through REST, and broadcast through an alarm WebSocket channel. The missing product piece is a stable UI entry point that lets users see and operate those alarm events.

## Scope

Implement a standalone Alarm Events page.

In scope:

- Add a frontend route at `/monitor/alarms`.
- Add a navigation entry for the page.
- Add a new `AlarmEvents.vue` view that wraps the alarm list in the standard page shell.
- Improve `AlarmEventList.vue` so it matches the backend alarm event model.
- Use REST polling as the reliable data source.
- Keep the existing WebSocket connection as opportunistic enhancement only.
- Add zh/en i18n text for the new page and alarm list.
- Update `docs/product-capability-matrix.md` to reflect basic alarm UI visibility.

Out of scope:

- Do not embed the alarm list into `LiveMonitor.vue` in this iteration.
- Do not add backend alarm query filters.
- Do not change alarm lifecycle semantics in `AlarmStore`.
- Do not redesign WebSocket authentication.
- Do not change flow alarm operator semantics or duplicate-suppression behavior.

## User Experience

The user opens `/monitor/alarms` from the navigation.

The page shows:

- A title and short description.
- Severity filter chips for `critical`, `high`, `medium`, `low`, and `info`.
- A list of recent alarm events.
- Per-event severity, timestamp, message, tag/value/threshold/rule details, and lifecycle status.
- Action buttons for acknowledge and resolve.

Action behavior:

- `active` events can be acknowledged or resolved.
- `acknowledged` events cannot be acknowledged again, but can be resolved.
- `resolved` events cannot be acknowledged or resolved.

The component should remain useful even when WebSocket auth prevents the browser WebSocket from connecting. REST polling is the source of truth.

## Components

### `web/src/views/AlarmEvents.vue`

New view component.

Responsibilities:

- Render the standard page header.
- Render `AlarmEventList` in a full-page content area.
- Keep page-level layout simple so the list component remains reusable.

Dependencies:

- `PageHeader.vue`
- `AlarmEventList.vue`
- `vue-i18n`

### `web/src/components/AlarmEventList.vue`

Existing component, improved in place.

Responsibilities:

- Fetch alarm events through `api.alarmEvents()` on mount.
- Poll every 3 seconds.
- Optionally connect to `/api/ws/alarms`; failure must not break the page.
- Merge incoming events into local state.
- Provide local severity filtering.
- Call `api.ackAlarmEvent(id)` and `api.resolveAlarmEvent(id)` for lifecycle actions.
- Display loading/error/empty states.

Backend model compatibility:

- Severity comes from `event.level` or `event.severity`.
- Supported severities are `critical`, `high`, `medium`, `low`, `info`.
- Timestamp comes from `event.timestamp` or `event.created_at`.
- Status comes from `event.status`.

Merge behavior:

- If an incoming event has an existing id, update that event in place so polling can refresh ack/resolve state.
- If an incoming event is new, prepend it and mark `isNew` briefly.
- Keep the list bounded at the existing UI limit.
- Sort by timestamp descending after merges.

Error handling:

- REST load failure stores a lightweight error message and keeps existing data visible.
- Ack/resolve failures show an Element Plus message with the failure reason.
- WebSocket close/error is silent and does not stop polling.

## Routing and Navigation

### `web/src/router.js`

Add route:

```js
{
  path: '/monitor/alarms',
  name: 'AlarmEvents',
  component: () => import('./views/AlarmEvents.vue'),
  meta: { title: '告警事件' }
}
```

The route is authenticated like other non-public pages.

### `web/src/App.vue`

Add a navigation item for the standalone page. The preferred placement is in the system/monitoring section near Data Flow Metrics, because the existing main navigation is already crowded and Data Monitor remains focused on tag values.

Add i18n key usage:

```js
{ path: '/monitor/alarms', labelKey: 'menu.alarmEvents' }
```

## Internationalization

Add zh/en keys in `web/src/locales/zh.js` and `web/src/locales/en.js`.

Required key groups:

- `menu.alarmEvents`
- `menu.alarmEventsDesc` if needed later by cards/tooltips
- `alarms.title`
- `alarms.desc`
- `alarms.eventCount`
- `alarms.noEvents`
- `alarms.loadFailed`
- `alarms.ackFailed`
- `alarms.resolveFailed`
- `alarms.unknownAlarm`
- `alarms.filterHint`
- `alarms.actions.refresh`
- `alarms.actions.clear`
- `alarms.actions.ack`
- `alarms.actions.resolve`
- `alarms.fields.value`
- `alarms.fields.status`
- `alarms.fields.tag`
- `alarms.fields.threshold`
- `alarms.fields.rule`
- `alarms.levels.critical`
- `alarms.levels.high`
- `alarms.levels.medium`
- `alarms.levels.low`
- `alarms.levels.info`
- `alarms.status.active`
- `alarms.status.acknowledged`
- `alarms.status.resolved`

## Documentation Update

Update `docs/product-capability-matrix.md` after implementation.

Expected change:

- Move `Alarm events` from `Missing` to `Beta` or `Experimental` depending on final verification.
- Note that persisted backend lifecycle and standalone UI visibility exist.
- Keep caveats: query filters, dedupe/recovery semantics, and WebSocket auth hardening remain future work.

## Testing and Verification

Run frontend build:

```bash
cd web && npm run build
```

Run the existing alarm store test:

```bash
cargo test -p gateway-server alarm_store_persists_and_lists_events
```

Manual verification target:

- Route `/monitor/alarms` loads without frontend route errors.
- It calls `GET /api/alarm/events`.
- Existing events render with correct severity/status labels.
- Ack and resolve buttons call the existing REST endpoints and update the event state.
- WebSocket failure does not prevent the list from loading.

## Future Follow-ups

- Reuse `AlarmEventList.vue` inside `LiveMonitor.vue` once the standalone page is stable.
- Add backend query parameters for severity/status/source/time/limit.
- Fix WebSocket auth explicitly with a token query parameter or dedicated WS auth path.
- Add alarm dedupe/recovery semantics to avoid repeated active alarm rows.
- Improve source attribution so events clearly reference the original south node/group and flow id.
