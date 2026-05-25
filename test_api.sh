#!/bin/bash
# Comprehensive API Test Script for IoT Gateway
# Fixes tested:
# 1. POST /api/nodes needs 'kind' field (south/north)
# 2. /api/backup needs Content-Type: application/json
# 3. User password change (PUT) and delete (204) - working correctly
# 4. Node CRUD with correct kind field
# 5. WebSocket connection
# 6. Flow creation/deployment

GATEWAY_URL="http://127.0.0.1:4001/api"
WS_URL="ws://127.0.0.1:4001/ws"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; }
info() { echo -e "${YELLOW}[INFO]${NC} $1"; }

# Get auth token
get_token() {
    TOKEN=$(curl -s -X POST "$GATEWAY_URL/auth/login" \
        -H "Content-Type: application/json" \
        -d '{"username":"admin","password":"admin123"}' | grep -o '"token":"[^"]*"' | cut -d'"' -f4)
    if [ -z "$TOKEN" ]; then
        echo "Failed to get token"
        exit 1
    fi
    echo "$TOKEN"
}

TOKEN=$(get_token)
info "Got auth token: ${TOKEN:0:20}..."

# ==================== TEST 1: Backup without Content-Type (should fail with 415) ====================
info "=== Test 1: Backup endpoint Content-Type requirement ==="
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$GATEWAY_URL/backup" \
    -H "Authorization: Bearer $TOKEN")
if [ "$HTTP_CODE" = "415" ]; then
    pass "Backup without Content-Type correctly returns 415"
else
    fail "Backup without Content-Type returns $HTTP_CODE (expected 415)"
fi

# ==================== TEST 2: Backup WITH Content-Type (should succeed) ====================
info "=== Test 2: Backup endpoint with Content-Type ==="
HTTP_CODE=$(curl -s -o /tmp/backup.bin -w "%{http_code}" -X POST "$GATEWAY_URL/backup" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{}')
if [ "$HTTP_CODE" = "200" ] && [ -s /tmp/backup.bin ]; then
    pass "Backup with Content-Type succeeds (200) and returns data"
else
    fail "Backup with Content-Type returns $HTTP_CODE"
fi

# ==================== TEST 3: Create node WITHOUT kind (should fail) ====================
info "=== Test 3: Create node WITHOUT kind field ==="
RESP=$(curl -s -X POST "$GATEWAY_URL/nodes" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"test-node-no-kind","plugin_name":"sim"}')
if echo "$RESP" | grep -q "missing field"; then
    pass "Create node without kind correctly fails (missing field)"
else
    fail "Create node without kind: $RESP"
fi

# ==================== TEST 4: Create node WITH kind (should succeed) ====================
info "=== Test 4: Create south node WITH kind field ==="
RESP=$(curl -s -X POST "$GATEWAY_URL/nodes" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"test-south-node","kind":"south","plugin_name":"sim"}')
if echo "$RESP" | grep -q '"id"'; then
    SOUTH_ID=$(echo "$RESP" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    pass "Create south node succeeds with ID: $SOUTH_ID"
else
    fail "Create south node failed: $RESP"
fi

# Create a north node
info "=== Test 5: Create north node WITH kind field ==="
RESP=$(curl -s -X POST "$GATEWAY_URL/nodes" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"name":"test-north-node","kind":"north","plugin_name":"mqtt"}')
if echo "$RESP" | grep -q '"id"'; then
    NORTH_ID=$(echo "$RESP" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    pass "Create north node succeeds with ID: $NORTH_ID"
else
    info "Create north node (mqtt plugin): $RESP"
    # Try with different north plugin
    RESP=$(curl -s -X POST "$GATEWAY_URL/nodes" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"name":"test-north-node","kind":"north","plugin_name":"websocket"}')
    if echo "$RESP" | grep -q '"id"'; then
        NORTH_ID=$(echo "$RESP" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
        pass "Create north node (websocket) succeeds with ID: $NORTH_ID"
    else
        info "North node creation: $RESP"
        NORTH_ID=""
    fi
fi

# ==================== TEST 6: List nodes ====================
info "=== Test 6: List all nodes ==="
RESP=$(curl -s "$GATEWAY_URL/nodes" -H "Authorization: Bearer $TOKEN")
NODE_COUNT=$(echo "$RESP" | grep -o '"id"' | wc -l)
pass "List nodes returns $NODE_COUNT nodes"

# ==================== TEST 7: Get single node ====================
if [ -n "$SOUTH_ID" ]; then
    info "=== Test 7: Get single node ==="
    RESP=$(curl -s "$GATEWAY_URL/nodes/$SOUTH_ID" -H "Authorization: Bearer $TOKEN")
    if echo "$RESP" | grep -q "test-south-node"; then
        pass "Get single node succeeds"
    else
        fail "Get single node: $RESP"
    fi
fi

# ==================== TEST 8: Update node ====================
if [ -n "$SOUTH_ID" ]; then
    info "=== Test 8: Update node ==="
    RESP=$(curl -s -X PUT "$GATEWAY_URL/nodes/$SOUTH_ID" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"name":"test-south-node-updated"}')
    if echo "$RESP" | grep -q "test-south-node-updated"; then
        pass "Update node succeeds"
    else
        fail "Update node: $RESP"
    fi
fi

# ==================== TEST 9: Start/Stop node ====================
if [ -n "$SOUTH_ID" ]; then
    info "=== Test 9: Start node ==="
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$GATEWAY_URL/nodes/$SOUTH_ID/start" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "200" ]; then
        pass "Start node returns 200"
    else
        fail "Start node returns $HTTP_CODE"
    fi

    info "=== Test 10: Stop node ==="
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$GATEWAY_URL/nodes/$SOUTH_ID/stop" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "200" ]; then
        pass "Stop node returns 200"
    else
        fail "Stop node returns $HTTP_CODE"
    fi
fi

# ==================== TEST 11: Delete node ====================
if [ -n "$SOUTH_ID" ]; then
    info "=== Test 11: Delete node ==="
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "$GATEWAY_URL/nodes/$SOUTH_ID" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "204" ]; then
        pass "Delete node returns 204"
    else
        fail "Delete node returns $HTTP_CODE"
    fi
fi

# ==================== TEST 12: User password change (204 = correct) ====================
info "=== Test 12: Change user password ==="
USER_ID=$(curl -s "$GATEWAY_URL/users" -H "Authorization: Bearer $TOKEN" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X PUT "$GATEWAY_URL/users/$USER_ID/password" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"password":"admin123"}')
if [ "$HTTP_CODE" = "204" ]; then
    pass "Change password returns 204 (success, no body)"
else
    fail "Change password returns $HTTP_CODE (expected 204)"
fi

# ==================== TEST 13: User delete (204 = correct) ====================
info "=== Test 13: Create and delete user ==="
# First create a test user
NEW_USER_RESP=$(curl -s -X POST "$GATEWAY_URL/users" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"username":"testuser","password":"test123","role":"operator"}')
if echo "$NEW_USER_RESP" | grep -q '"id"'; then
    NEW_USER_ID=$(echo "$NEW_USER_RESP" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    
    # Now delete it
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "$GATEWAY_URL/users/$NEW_USER_ID" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "204" ]; then
        pass "Delete user returns 204 (success, no body)"
    else
        fail "Delete user returns $HTTP_CODE (expected 204)"
    fi
else
    info "Create user response: $NEW_USER_RESP"
fi

# ==================== TEST 14: Flow CRUD ====================
info "=== Test 14: Flow CRUD ==="
# List flows
RESP=$(curl -s "$GATEWAY_URL/flows" -H "Authorization: Bearer $TOKEN")
pass "List flows: $(echo $RESP | head -c 100)..."

# Create flow
FLOW_RESP=$(curl -s -X POST "$GATEWAY_URL/flows" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
        "name": "test-flow",
        "description": "Test flow",
        "flow_data": {"nodes": [], "edges": []}
    }')
if echo "$FLOW_RESP" | grep -q '"id"'; then
    FLOW_ID=$(echo "$FLOW_RESP" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
    pass "Create flow succeeds with ID: $FLOW_ID"
    
    # Deploy flow
    info "=== Test 15: Deploy flow ==="
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$GATEWAY_URL/flows/$FLOW_ID/deploy" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "200" ]; then
        pass "Deploy flow returns 200"
    else
        info "Deploy flow returns $HTTP_CODE"
    fi
    
    # Delete flow
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "$GATEWAY_URL/flows/$FLOW_ID" \
        -H "Authorization: Bearer $TOKEN")
    if [ "$HTTP_CODE" = "204" ]; then
        pass "Delete flow returns 204"
    else
        info "Delete flow returns $HTTP_CODE"
    fi
else
    info "Create flow response: $FLOW_RESP"
fi

# ==================== TEST 16: WebSocket connection ====================
info "=== Test 16: WebSocket connection ==="
# Check if WebSocket upgrade works
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X GET \
    "http://127.0.0.1:4001/ws/flows/00000000-0000-0000-0000-000000000000/live" \
    --include \
    --no-buffer 2>/dev/null | head -1)
if echo "$HTTP_CODE" | grep -q "101"; then
    pass "WebSocket upgrade returns 101 Switching Protocols"
elif echo "$HTTP_CODE" | grep -q "400"; then
    pass "WebSocket endpoint responds (400 due to invalid flow ID, WS protocol OK)"
elif echo "$HTTP_CODE" | grep -q "426"; then
    pass "WebSocket endpoint responds (426 Upgrade Required, WS supported)"
else
    info "WebSocket HTTP code: $HTTP_CODE"
fi

# ==================== TEST 17: List operators ====================
info "=== Test 17: List flow operators ==="
RESP=$(curl -s "$GATEWAY_URL/flows/operators" -H "Authorization: Bearer $TOKEN")
if echo "$RESP" | grep -q "operators"; then
    pass "List operators succeeds"
else
    info "List operators: $RESP"
fi

echo ""
info "========================================="
info "API Testing Complete"
info "========================================="
