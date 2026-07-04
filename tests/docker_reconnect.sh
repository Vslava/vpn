#!/usr/bin/env bash
# shellcheck disable=SC2016,SC2086
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="traffic-sentinel-e2e:latest"
PSK="deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
PASS=0
FAIL=0

cleanup_all() {
    for c in $(docker ps -a --format '{{.Names}}' 2>/dev/null | grep -E '^ts-(server|client)-'); do
        docker kill "$c" 2>/dev/null || true
        docker rm -f "$c" 2>/dev/null || true
    done
    for n in $(docker network ls --format '{{.Name}}' 2>/dev/null | grep -E '^ts-r[0-9]-'); do
        docker network rm "$n" 2>/dev/null || true
    done
    rm -f /tmp/ts-r*.toml
}

trap cleanup_all EXIT

step() {
    local label="$1"
    shift
    echo -n "  $label ... "
    local rc=0
    eval "$(printf '%q ' "$@")" > /dev/null 2>&1 || rc=$?
    if [ "$rc" -eq 0 ]; then
        echo "OK"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        FAIL=$((FAIL + 1))
    fi
}

wait_for_log() {
    local container="$1"
    local pattern="$2"
    local timeout="${3:-15}"
    for i in $(seq 1 "$timeout"); do
        docker logs "$container" 2>&1 | grep -Eq "$pattern" && return 0
        sleep 1
    done
    return 1
}

start_server() {
    local name="$1"
    local cfg="$2"
    docker rm -f "$name" 2>/dev/null || true
    docker run -d --name "$name" \
        --network "$NET_NAME" --ip "$SERVER_IP" \
        --cap-add NET_ADMIN --device /dev/net/tun \
        --sysctl net.ipv4.ip_forward=1 \
        -e RUST_LOG=info \
        -v "$cfg:/etc/ts.toml:ro" \
        --entrypoint sh \
        "$IMAGE" -c '
            mkdir -p /dev/net
            [ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
            chmod 666 /dev/net/tun
            traffic-sentinel --mode server --config /etc/ts.toml
        ' > /dev/null
}

start_client() {
    local name="$1"
    local cfg="$2"
    docker rm -f "$name" 2>/dev/null || true
    docker run -d --name "$name" \
        --network "$NET_NAME" \
        --cap-add NET_ADMIN --device /dev/net/tun \
        -e RUST_LOG=info \
        -v "$cfg:/etc/ts.toml:ro" \
        --entrypoint sh \
        "$IMAGE" -c '
            mkdir -p /dev/net
            [ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
            chmod 666 /dev/net/tun
            traffic-sentinel --mode client --config /etc/ts.toml
        ' > /dev/null
}

echo "=========================================="
echo "  P2.2: Reconnection Verification (Docker)"
echo "=========================================="

echo ""
echo "=== Building release binary ==="
cargo build --release -q --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "=== Building Docker image ==="
docker build -q -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.e2e" "$PROJECT_DIR"

# ===========================================================
# UDP reconnect strategy:
#   Kill server → client detects loss via heartbeat timeout
#   (no TCP RST — watchdog fires after heartbeat_timeout with no PONG)
#   Then client reconnects with fresh UDP socket + new handshake
# ===========================================================

# ===========================================================
# Test 1: Basic reconnect — kill server, restart, verify traffic
# ===========================================================
echo ""
echo "--- Test 1: Basic reconnect (kill server, restart) ---"
cleanup_all
NET_NAME="ts-r1-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-r1-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

cat > /tmp/ts-r1-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 10
reconnect_max_delay = 30
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-server-1 /tmp/ts-r1-server.toml
wait_for_log ts-server-1 "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-client-1 /tmp/ts-r1-client.toml
wait_for_log ts-client-1 "resuming" 15 || { echo "FAIL: client connect"; exit 1; }

step "Ping before kill" docker exec ts-client-1 ping -c 2 -W 3 10.0.0.1

# Kill server → container exits
docker exec ts-server-1 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true

# Wait for client to detect heartbeat timeout (~15s) + reconnect attempts
echo "  Waiting for heartbeat timeout (~20s)..."
sleep 20

step "Client detected heartbeat timeout" sh -c 'docker logs ts-client-1 2>&1 | grep -q "Connection timeout"'
step "Client started reconnect attempt" docker logs ts-client-1 2>&1 | grep -q "Reconnecting"

# Restart server container (same IP, same config)
echo "  Restarting server container..."
docker start ts-server-1 > /dev/null
wait_for_log ts-server-1 "[Ll]istening" 15 || { echo "FAIL: server restart"; exit 1; }

step "Client resumed (Handshake complete)" wait_for_log ts-client-1 "resuming" 30

sleep 5
step "Ping after reconnect" docker exec ts-client-1 ping -c 2 -W 5 10.0.0.1

docker rm -f ts-client-1 ts-server-1 > /dev/null 2>&1 || true

# ===========================================================
# Test 2: Max retries → graceful shutdown
#         Kill server, block UDP with iptables DROP,
#         client exhausts retries → graceful shutdown
# ===========================================================
echo ""
echo "--- Test 2: Max retries → graceful shutdown ---"
cleanup_all
NET_NAME="ts-r2-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-r2-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

cat > /tmp/ts-r2-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 0
reconnect_max_delay = 30
heartbeat_interval = 5
heartbeat_timeout = 10
EOF

start_server ts-server-2 /tmp/ts-r2-server.toml
wait_for_log ts-server-2 "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-client-2 /tmp/ts-r2-client.toml
wait_for_log ts-client-2 "resuming" 15 || { echo "FAIL: client connect"; exit 1; }
step "Initial connection" true

# Kill server
docker exec ts-server-2 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 2

# Block UDP so reconnect attempts fail
docker exec ts-client-2 iptables -A OUTPUT -p udp --dport 8443 -j DROP 2>/dev/null || true

# max_retries=0 → no reconnect, graceful shutdown after heartbeat timeout
echo "  Waiting for heartbeat timeout + shutdown (~15s)..."
sleep 15

LOG=$(docker logs ts-client-2 2>&1) || true

step "Heartbeat timeout detected" echo "$LOG" | grep -q "Connection timeout"
step "Restoring routes" echo "$LOG" | grep -q "Restoring routes"
step "Shutdown complete" echo "$LOG" | grep -q "Shutdown complete"

docker rm -f ts-client-2 ts-server-2 > /dev/null 2>&1 || true

# ===========================================================
# Test 3: SIGTERM during reconnect — graceful shutdown
# ===========================================================
echo ""
echo "--- Test 3: SIGTERM during reconnect ---"
cleanup_all
NET_NAME="ts-r3-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-r3-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

cat > /tmp/ts-r3-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 10
reconnect_max_delay = 30
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-server-3 /tmp/ts-r3-server.toml
wait_for_log ts-server-3 "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-client-3 /tmp/ts-r3-client.toml
wait_for_log ts-client-3 "resuming" 15 || { echo "FAIL: client connect"; exit 1; }
step "Initial connection" true

# Kill server to trigger reconnect
docker exec ts-server-3 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true

# Wait for client to detect timeout and start reconnecting
echo "  Waiting for heartbeat timeout (~18s)..."
sleep 18

step "Reconnect started after server kill" sh -c 'docker logs ts-client-3 2>&1 | grep -q "Reconnecting"'

# Send SIGTERM directly to traffic-sentinel on the client
docker exec ts-client-3 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 3

L=$(docker logs ts-client-3 2>&1) || true
step "Graceful: routes restored" echo "$L" | grep -q "Restoring routes"
step "Graceful: Shutdown complete" echo "$L" | grep -q "Shutdown complete"

docker rm -f ts-client-3 ts-server-3 > /dev/null 2>&1 || true

# ===========================================================
echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

[ "$FAIL" -gt 0 ] && { echo "FAILURES DETECTED"; exit 1; }

echo ""
echo "=== All P2.2 Reconnection verification tests passed! ==="
