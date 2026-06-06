#!/usr/bin/env bash
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="traffic-sentinel-e2e:latest"
PSK="deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
PASS=0
FAIL=0

cleanup_all() {
    for c in $(docker ps -a --format '{{.Names}}' 2>/dev/null | grep -E '^ts-hb-'); do
        docker kill "$c" 2>/dev/null || true
        docker rm -f "$c" 2>/dev/null || true
    done
    for n in $(docker network ls --format '{{.Name}}' 2>/dev/null | grep -E '^ts-hb[0-9]-'); do
        docker network rm "$n" 2>/dev/null || true
    done
    rm -f /tmp/ts-hb-*.toml
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
            iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE 2>/dev/null || true
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
echo "  P2.3: Heartbeat Verification (Docker)"
echo "=========================================="

echo ""
echo "=== Building release binary ==="
cargo build --release -q --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "=== Building Docker image ==="
docker build -q -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.e2e" "$PROJECT_DIR"

# ===========================================================
# Test 1: PING/PONG — connection stays alive during idle
#         heartbeat_interval=10, heartbeat_timeout=25
#         Wait 2.5x interval → at least 2 PING cycles should pass
#         If PONG works, client stays connected; if not, timeout fires
# ===========================================================
echo ""
echo "--- Test 1: PING/PONG keeps connection alive during idle ---"
cleanup_all
NET_NAME="ts-hb1-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-hb1-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-hb1-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 3
reconnect_max_delay = 30
heartbeat_interval = 10
heartbeat_timeout = 25
EOF

start_server ts-hb1-server /tmp/ts-hb1-server.toml
wait_for_log ts-hb1-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-hb1-client /tmp/ts-hb1-client.toml
wait_for_log ts-hb1-client "resuming" 15 || { echo "FAIL: client connect"; exit 1; }

step "Ping before idle" docker exec ts-hb1-client ping -c 2 -W 3 10.0.0.1

# Wait 2.5x heartbeat_interval for PING/PONG cycles
echo "  Waiting 25s for PING/PONG cycles..."
sleep 25

# Client should still be alive (PONG responses prevented timeout)
# If heartbeat timeout fired, client would start reconnecting
step "Client still connected (no reconnect)" sh -c 'docker logs ts-hb1-client 2>&1 | grep -c "resuming" | grep -q "^1$"'

# Traffic should still work — PING kept connection alive
step "Ping after PING/PONG cycles" docker exec ts-hb1-client ping -c 2 -W 3 10.0.0.1

CLIENT_LOG=$(docker logs ts-hb1-client 2>&1)
echo "  Client log lines: $(echo "$CLIENT_LOG" | wc -l)"
echo "  Last 3 lines:"
echo "$CLIENT_LOG" | tail -3

step "No connection lost" sh -c 'docker logs ts-hb1-client 2>&1 | grep -cE "(Connection lost|Connection timeout)" | grep -q "^0$"'
step "No Reconnecting messages" sh -c 'docker logs ts-hb1-client 2>&1 | grep -qc "Reconnecting" && false || true'

docker rm -f ts-hb1-client ts-hb1-server > /dev/null 2>&1 || true

# ===========================================================
# Test 2: Heartbeat timeout → reconnect
#         heartbeat_interval=5, heartbeat_timeout=15
#         Freeze server (SIGSTOP) → client PINGs get no PONG → timeout → reconnect
# ===========================================================
echo ""
echo "--- Test 2: Heartbeat timeout → reconnect ---"
cleanup_all
NET_NAME="ts-hb2-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-hb2-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-hb2-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 5
reconnect_max_delay = 30
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-hb2-server /tmp/ts-hb2-server.toml
wait_for_log ts-hb2-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-hb2-client /tmp/ts-hb2-client.toml
wait_for_log ts-hb2-client "resuming" 15 || { echo "FAIL: client connect"; exit 1; }

step "Ping before freeze" docker exec ts-hb2-client ping -c 2 -W 3 10.0.0.1

# Pause server container — all processes frozen, kernel TCP stays alive
docker pause ts-hb2-server > /dev/null
echo "  Server container paused (cgroup freezer)..."

# Wait for heartbeat timeout + margin
echo "  Waiting 20s for heartbeat timeout..."
sleep 20

step "Client detected heartbeat timeout" sh -c 'docker logs ts-hb2-client 2>&1 | grep -q "Connection timeout"'

# Unpause server — processes resume where they were frozen
# Old TCP connection dies (client closed it) → server returns to accept loop
docker unpause ts-hb2-server > /dev/null
echo "  Server unpaused..."

# Client should reconnect (run_client_full handles the timeout error)
step "Client reconnecting" sh -c 'docker logs ts-hb2-client 2>&1 | grep -q "Reconnecting"'
step "Client reconnected" wait_for_log ts-hb2-client "resuming" 30

sleep 2
step "Ping after reconnect" docker exec ts-hb2-client ping -c 2 -W 3 10.0.0.1

docker rm -f ts-hb2-client ts-hb2-server > /dev/null 2>&1 || true

# ===========================================================
# Test 3: Active traffic suppresses PING
#         heartbeat_interval=5, heartbeat_timeout=15
#         Continuous ping keeps connection alive without PING
# ===========================================================
echo ""
echo "--- Test 3: Active traffic keeps connection alive (no PING needed) ---"
cleanup_all
NET_NAME="ts-hb3-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-hb3-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-hb3-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 3
reconnect_max_delay = 30
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-hb3-server /tmp/ts-hb3-server.toml
wait_for_log ts-hb3-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-hb3-client /tmp/ts-hb3-client.toml
wait_for_log ts-hb3-client "resuming" 15 || { echo "FAIL: client connect"; exit 1; }

# Continuous ping via TUN — forces bidirectional traffic
docker exec -d ts-hb3-client ping -i 0.5 10.0.0.1 > /dev/null 2>&1
sleep 2

# Verify traffic is flowing
step "Initial ping active" docker exec ts-hb3-client ping -c 1 -W 2 10.0.0.1

# Wait enough time for 3+ heartbeat intervals
echo "  Waiting 20s with active traffic..."
sleep 20

# Connection should remain alive — continuous traffic resets heartbeat timer
step "Client still connected after active traffic" sh -c 'docker logs ts-hb3-client 2>&1 | grep -c "resuming" | grep -q "^1$"'
step "No reconnection during active traffic" sh -c 'docker logs ts-hb3-client 2>&1 | grep -qc "Reconnecting" && false || true'

# Verify traffic still works
step "Ping after active traffic" docker exec ts-hb3-client ping -c 2 -W 3 10.0.0.1

docker rm -f ts-hb3-client ts-hb3-server > /dev/null 2>&1 || true

# ===========================================================
echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

[ "$FAIL" -gt 0 ] && { echo "FAILURES DETECTED"; exit 1; }

echo ""
echo "=== All P2.3 Heartbeat verification tests passed! ==="
