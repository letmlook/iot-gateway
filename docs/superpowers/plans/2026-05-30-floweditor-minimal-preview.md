# FlowEditor Minimal Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make FlowEditor minimally usable with current backend plugin-list and flow-preview APIs.

**Architecture:** Keep the change local to `web/src/views/FlowEditor.vue`. Add small local helper functions for plugin-list response normalization, fixed preview sample construction, preview summary creation, and preview error rendering; wire them into existing mounted plugin loading and preview refresh flow without changing backend APIs or shared `api.js`.

**Tech Stack:** Vue 3 single-file component, Element Plus message UI, existing `web/src/api.js`, Vite build.

---

## File Structure

Modify only:

- `web/src/views/FlowEditor.vue` — normalize south/north plugin list responses and replace mock preview generation with backend `api.previewFlow()` using fixed sample input.

Do not modify:

- `web/src/api.js`
- `gateway/*`
- Flow save/bind/subscription behavior
- Playwright config or frontend routing

---

### Task 1: Normalize FlowEditor Plugin Palette Responses

**Files:**
- Modify: `web/src/views/FlowEditor.vue:570-586`

- [ ] **Step 1: Verify the current mismatch exists**

Run:

```bash
rg -n "southPlugins\.value = south\.plugins|northPlugins\.value = north\.plugins" web/src/views/FlowEditor.vue
```

Expected output includes:

```text
southPlugins.value = south.plugins || []
northPlugins.value = north.plugins || []
```

This is the failing condition: backend plugin endpoints return arrays, while FlowEditor expects `{ plugins: [...] }`.

- [ ] **Step 2: Add the normalizer helper**

In `web/src/views/FlowEditor.vue`, after the `statusLabel` function near line 479, add:

```js
function normalizePluginList(response) {
  return Array.isArray(response) ? response : (response?.plugins || [])
}
```

The surrounding section should look like:

```js
function statusLabel(s) {
  return { running: '运行中', stopped: '已停止', error: '错误', unknown: '未知' }[s] || s || '未知'
}

function normalizePluginList(response) {
  return Array.isArray(response) ? response : (response?.plugins || [])
}
```

- [ ] **Step 3: Use the normalizer when loading plugins**

Replace this block in `onMounted`:

```js
  // Load plugins
  try {
    const south = await api.pluginsSouth()
    southPlugins.value = south.plugins || []
    const north = await api.pluginsNorth()
    northPlugins.value = north.plugins || []
  } catch (e) { console.error(e) }
```

with:

```js
  // Load plugins
  try {
    const south = await api.pluginsSouth()
    southPlugins.value = normalizePluginList(south)
    const north = await api.pluginsNorth()
    northPlugins.value = normalizePluginList(north)
  } catch (e) {
    console.error(e)
    southPlugins.value = []
    northPlugins.value = []
  }
```

- [ ] **Step 4: Run frontend build**

Run:

```bash
cd web && npm run build
```

Expected: Vite build succeeds. Existing chunk-size warnings are acceptable; TypeScript/Vue compile errors are not.

- [ ] **Step 5: Commit palette fix**

Run:

```bash
git add web/src/views/FlowEditor.vue
git commit -m "fix(web): normalize FlowEditor plugin lists" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Replace Mock Preview with Backend Preview API

**Files:**
- Modify: `web/src/views/FlowEditor.vue:916-953`

- [ ] **Step 1: Verify the current mock preview exists**

Run:

```bash
rg -n "Mock data for preview|temperature: 77\.1|refreshPreview\(\)" web/src/views/FlowEditor.vue
```

Expected output includes the current mock implementation:

```text
function refreshPreview()
// Mock data for preview (real WebSocket comes Phase 4)
temperature: 77.1
```

This is the failing condition: preview output is generated locally instead of by the backend.

- [ ] **Step 2: Add fixed sample and summary helpers**

In `web/src/views/FlowEditor.vue`, after `normalizePluginList`, add:

```js
function buildPreviewSampleInput() {
  return {
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
}

function previewResultSummary(result) {
  const outputCount = Array.isArray(result?.output) ? result.output.length : 0
  const alarmCount = Array.isArray(result?.alarm_events) ? result.alarm_events.length : 0
  const errorCount = Array.isArray(result?.errors) ? result.errors.length : 0
  return `preview ok: output=${outputCount}, alarms=${alarmCount}, errors=${errorCount}`
}

function previewErrorPayload(error) {
  return {
    error: error?.message || String(error),
    source: 'backend-preview'
  }
}

function addRecentPreviewMessage(dir, data) {
  recentMessages.value = [
    { time: new Date().toLocaleTimeString(), dir, data },
    ...recentMessages.value.slice(0, 3)
  ]
}
```

The helpers intentionally stay in this SFC because the scope is local to FlowEditor.

- [ ] **Step 3: Replace `refreshPreview()` with async backend implementation**

Replace the current `refreshPreview()` function:

```js
function refreshPreview() {
  if (!selectedNode.value) return
  // Mock data for preview (real WebSocket comes Phase 4)
  const now = new Date().toLocaleTimeString()
  previewInput.value = JSON.stringify({ temperature: 25.6, humidity: 60, timestamp: now }, null, 2)
  previewOutput.value = JSON.stringify({ temperature: 77.1, humidity: 60, timestamp: now }, null, 2)

  // Add to recent messages
  recentMessages.value = [
    { time: now, dir: 'in', data: `temperature=25.6, humidity=60` },
    { time: now, dir: 'out', data: `temperature=77.1, humidity=60` },
    ...recentMessages.value.slice(0, 3)
  ]
}
```

with:

```js
async function refreshPreview() {
  if (!selectedNode.value) return

  const sampleInput = buildPreviewSampleInput()
  previewInput.value = JSON.stringify(sampleInput, null, 2)

  if (!flowId.value) {
    const message = { message: '请先保存数据流后再预览' }
    previewOutput.value = JSON.stringify(message, null, 2)
    addRecentPreviewMessage('out', message.message)
    return
  }

  try {
    const result = await api.previewFlow(flowId.value, sampleInput)
    previewOutput.value = JSON.stringify(result, null, 2)
    addRecentPreviewMessage('out', previewResultSummary(result))
  } catch (e) {
    const errorPayload = previewErrorPayload(e)
    previewOutput.value = JSON.stringify(errorPayload, null, 2)
    addRecentPreviewMessage('out', `preview error: ${errorPayload.error}`)
  }
}
```

- [ ] **Step 4: Run frontend build**

Run:

```bash
cd web && npm run build
```

Expected: Vite build succeeds. Existing chunk-size warnings are acceptable; compile errors are not.

- [ ] **Step 5: Verify mock preview strings are gone**

Run:

```bash
rg -n "Mock data for preview|temperature: 77\.1|real WebSocket comes Phase 4" web/src/views/FlowEditor.vue
```

Expected: no output.

- [ ] **Step 6: Commit preview fix**

Run:

```bash
git add web/src/views/FlowEditor.vue
git commit -m "fix(web): use backend FlowEditor preview" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Final Verification

**Files:**
- Validate only; no file edits expected.

- [ ] **Step 1: Run frontend build**

Run:

```bash
cd web && npm run build
```

Expected: build succeeds.

- [ ] **Step 2: Run workspace tests**

Run:

```bash
cargo test --workspace
```

Expected: all Rust workspace tests pass.

- [ ] **Step 3: Run full-chain E2E with configured MQTT broker**

Run:

```bash
E2E_MQTT_HOST=127.0.0.1 E2E_MQTT_PORT=1883 PYTHON_BIN=e2e/full_chain/.venv/bin/python ./scripts/e2e_full_chain.sh
```

Expected:

```text
[result] full-chain E2E passed
```

If the local venv is missing, create it and install dependencies first:

```bash
python3 -m venv e2e/full_chain/.venv
e2e/full_chain/.venv/bin/python -m pip install -r e2e/full_chain/requirements.txt
```

- [ ] **Step 4: Inspect git status**

Run:

```bash
git status --short
```

Expected: clean working tree.

---

## Self-Review Against Spec

Spec coverage:

- Plugin palette response normalization: Task 1.
- Keep compatibility with arrays and `{ plugins: [...] }`: Task 1 helper.
- Real backend preview API call: Task 2.
- Fixed sample input: Task 2 `buildPreviewSampleInput`.
- Unsaved flow message and no backend call: Task 2 `if (!flowId.value)` branch.
- Backend preview error JSON and recent message: Task 2 catch branch.
- No backend or `api.js` changes: file structure and tasks constrain changes to `FlowEditor.vue` only.
- Build verification: Tasks 1, 2, and 3.

Type consistency:

- `api.previewFlow(id, input)` matches `web/src/api.js` signature.
- Sample `DataValue` shape uses `{ type: 'Float64', value: number }`, matching backend serde enum shape used elsewhere.
- Preview result summary uses `output`, `alarm_events`, and `errors`, matching backend preview response.
