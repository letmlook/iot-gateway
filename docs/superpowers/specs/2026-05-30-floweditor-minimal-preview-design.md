# FlowEditor Minimal Preview Design

Date: 2026-05-30

## Goal

Make the FlowEditor minimally usable against the current backend by fixing plugin palette loading and replacing mock preview data with the real `/api/flows/:id/preview` endpoint.

## Scope

In scope:

- Fix FlowEditor south/north plugin palette loading when backend plugin list endpoints return arrays.
- Keep compatibility if plugin list endpoints return `{ "plugins": [...] }` in the future.
- Replace mock preview output with a real backend preview call.
- Use a fixed sample input payload for preview.
- Show clear preview success and error summaries in the existing preview panel.

Out of scope:

- Redesigning the FlowEditor UI.
- Changing backend API response shapes.
- Changing `web/src/api.js`.
- Fixing deeper flow model mismatches such as `operator_config` vs `config`.
- Auto-saving before preview.
- Adding user-editable preview JSON input.
- Changing subscriptions, bindings, deploy, or lifecycle behavior.

## Current Problems

### Plugin palette mismatch

`FlowEditor.vue` currently loads plugins with:

```js
const south = await api.pluginsSouth()
southPlugins.value = south.plugins || []
const north = await api.pluginsNorth()
northPlugins.value = north.plugins || []
```

The backend currently returns a JSON array from `/api/plugins/south` and `/api/plugins/north`, not an object with a `plugins` field. When the response is an array, the palette becomes empty.

### Preview still uses mock data

`refreshPreview()` currently creates fixed fake input and output locally. It does not call the real backend preview API, so users cannot see backend runtime validation, per-node output, alarm drafts, or errors.

## Design

### 1. Normalize plugin list responses locally

Add a small helper in `web/src/views/FlowEditor.vue`:

```js
function normalizePluginList(response) {
  return Array.isArray(response) ? response : (response?.plugins || [])
}
```

Use it only in FlowEditor plugin loading:

```js
const south = await api.pluginsSouth()
southPlugins.value = normalizePluginList(south)

const north = await api.pluginsNorth()
northPlugins.value = normalizePluginList(north)
```

This keeps the fix local and avoids changing shared API behavior for other pages.

### 2. Use backend preview with fixed sample input

Change `refreshPreview()` to be async.

If `flowId.value` is empty, do not call the backend. Show a clear message:

```json
{
  "message": "请先保存数据流后再预览"
}
```

If `flowId.value` exists, use this fixed sample request:

```js
const sampleInput = {
  input: {
    payload: {
      temperature: { type: 'Float64', value: 25.6 },
      humidity: { type: 'Float64', value: 60 }
    },
    metadata: {
      source: 'flow-editor-preview'
    }
  }
}
```

Then call:

```js
const result = await api.previewFlow(flowId.value, sampleInput)
```

Set:

```js
previewInput.value = JSON.stringify(sampleInput, null, 2)
previewOutput.value = JSON.stringify(result, null, 2)
```

### 3. Recent preview messages

On success, add a concise summary to `recentMessages`:

```text
preview ok: output=<count>, alarms=<count>, errors=<count>
```

Counts should come from `result.output`, `result.alarm_events`, and `result.errors`, treating missing arrays as empty arrays.

On failure, set `previewOutput` to:

```json
{
  "error": "<message>",
  "source": "backend-preview"
}
```

Also add a recent message:

```text
preview error: <message>
```

The editor should not throw uncaught errors or interrupt save/deploy controls when preview fails.

## Data Flow

```text
FlowEditor mounted
  -> api.pluginsSouth() / api.pluginsNorth()
  -> normalizePluginList()
  -> palette renders south/north items

Preview panel refresh
  -> no flowId: show save-first message
  -> has flowId: build fixed sample input
  -> api.previewFlow(flowId, sampleInput)
  -> render backend response as preview output
  -> append success/error summary to recentMessages
```

## Testing and Verification

Run:

```bash
cd web && npm run build
```

Manual or browser verification should confirm:

- South plugin palette displays when `/api/plugins/south` returns an array.
- North plugin palette displays when `/api/plugins/north` returns an array.
- The normalization still supports `{ plugins: [...] }` if that shape appears later.
- Unsaved flows show the save-first preview message and do not call preview.
- Saved flows call `/api/flows/:id/preview` with the fixed sample input.
- The preview panel displays backend result JSON instead of mock output.
- Preview errors are shown as JSON and recent message summaries.

## Acceptance Criteria

- `web/src/views/FlowEditor.vue` contains a local plugin-list normalizer.
- FlowEditor no longer assumes plugin list responses have a `plugins` property.
- `refreshPreview()` no longer emits fake transformed output.
- Saved flows use `api.previewFlow()` with the fixed sample input.
- Unsaved flows get a clear no-preview-until-save message.
- `npm run build` passes for the frontend.
