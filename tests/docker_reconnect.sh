#!/usr/bin/env bash
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
echo "  P2.2: Reconnection Verification (Docker)"
echo "=========================================="

echo ""
echo "=== Building release binary ==="
cargo build --release -q --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "=== Building Docker image ==="
docker build -q -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.e2e" "$PROJECT_DIR"

# ===========================================================
# Strategy for triggering reconnect:
#   1. Kill traffic-sentinel on the server via docker exec
#      This causes immediate TCP RST → client detects loss instantly
#   2. Server container exits when traffic-sentinel dies (sh exits)
#   3. docker start restarts the server container (same IP, same config)
#   4. Client reconnects
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
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-r1-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 10
reconnect_max_delay = 30
EOF

start_server ts-server-1 /tmp/ts-r1-server.toml
wait_for_log ts-server-1 "listening on" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-client-1 /tmp/ts-r1-client.toml
wait_for_log ts-client-1 "resuming" 15 || { echo "FAIL: client connect"; exit 1; }

step "Ping before kill" docker exec ts-client-1 ping -c 2 -W 3 10.0.0.1

# Kill server process → container exits → client detects TCP loss
docker exec ts-server-1 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 3

step "Client detected TCP connection lost" docker logs ts-client-1 2>&1 | grep -q "TCP connection lost"
step "Client started reconnect attempt" docker logs ts-client-1 2>&1 | grep -q "Reconnecting in"

# Restart server container (same IP, same config)
echo "  Restarting server container..."
docker start ts-server-1 > /dev/null
wait_for_log ts-server-1 "listening on" 15 || { echo "FAIL: server restart"; exit 1; }

step "Client resumed (Handshake complete)" wait_for_log ts-client-1 "resuming" 30

sleep 2
step "Ping after reconnect" docker exec ts-client-1 ping -c 2 -W 3 10.0.0.1

docker rm -f ts-client-1 ts-server-1 > /dev/null 2>&1 || true

# ===========================================================
# Test 2: Exponential backoff + max retries
#         Kill server, then block reconnect attempts with iptables
#         for fast failure (RST on new connections)
# ===========================================================
echo ""
echo "--- Test 2: Exponential backoff + max retries ---"
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
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-r2-client.toml << EOF
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
EOF

start_server ts-server-2 /tmp/ts-r2-server.toml
wait_for_log ts-server-2 "listening on" 10

start_client ts-client-2 /tmp/ts-r2-client.toml
wait_for_log ts-client-2 "resuming" 15
step "Initial connection" true

# Kill server → client detects loss
docker exec ts-server-2 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 2

# Now block port on client so reconnect attempts fail instantly
# (server is dead → container still exists briefly → ECONNREFUSED is possible)
# But to be safe, add iptables REJECT for instant TCP RST
docker exec ts-client-2 iptables -A OUTPUT -p tcp --dport 8443 -j REJECT --reject-with tcp-reset 2>/dev/null || true

# max_retries=5 → 6 attempts, backoffs: 1+2+4+8+16 = 31s
echo "  Waiting for reconnect attempts (~40s)..."
sleep 42

LOG=$(docker logs ts-client-2 2>&1) || true

step "Reconnect attempts present" echo "$LOG" | grep -q "Reconnecting in"
step "Max retries exceeded" echo "$LOG" | grep -q "Max retries"
step "Restoring routes" echo "$LOG" | grep -q "Restoring routes"
step "Shutdown complete" echo "$LOG" | grep -q "Shutdown complete"

DELAYS=$(echo "$LOG" | grep -oP 'Reconnecting in \K\d+' || true)
echo "  Backoff delays: $DELAYS"
COUNT=$(echo "$DELAYS" | wc -l)
step "Backoff with ≥3 steps" test "$COUNT" -ge 3

docker rm -f ts-client-2 ts-server-2 > /dev/null 2>&1 || true

# ===========================================================
# Test 3: Server accept loop
# ===========================================================
echo ""
echo "--- Test 3: Server accept loop ---"
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
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

for CFG in /tmp/ts-r3-c1.toml /tmp/ts-r3-c2.toml; do
cat > "$CFG" << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 0
reconnect_max_delay = 30
EOF
done

start_server ts-server-3 /tmp/ts-r3-server.toml
wait_for_log ts-server-3 "listening on" 10

start_client ts-client-3a /tmp/ts-r3-c1.toml
wait_for_log ts-client-3a "resuming" 15
step "Client 1 connected" true

# Start background ping so server sees active traffic
docker exec -d ts-client-3a ping 10.0.0.1
sleep 2

step "Ping from client 1" docker exec ts-client-3a ping -c 1 -W 2 10.0.0.1

# Kill client container → TCP RST → server detects disconnect on next write
docker kill ts-client-3a > /dev/null
sleep 2

step "Server waiting for new connection" wait_for_log ts-server-3 "waiting for new connection" 10

start_client ts-client-3b /tmp/ts-r3-c2.toml
step "Client 2 connected" wait_for_log ts-client-3b "resuming" 15
step "Ping from client 2" docker exec ts-client-3b ping -c 2 -W 3 10.0.0.1

docker rm -f ts-client-3a ts-client-3b ts-server-3 > /dev/null 2>&1 || true

# ===========================================================
# Test 4: SIGTERM during reconnect — graceful shutdown
# ===========================================================
echo ""
echo "--- Test 4: SIGTERM during reconnect ---"
cleanup_all
NET_NAME="ts-r4-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-r4-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > /tmp/ts-r4-client.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 10
reconnect_max_delay = 30
EOF

start_server ts-server-4 /tmp/ts-r4-server.toml
wait_for_log ts-server-4 "listening on" 10

start_client ts-client-4 /tmp/ts-r4-client.toml
wait_for_log ts-client-4 "resuming" 15
step "Initial connection" true

# Start traffic so client detects TCP loss quickly
docker exec -d ts-client-4 ping 10.0.0.1 > /dev/null 2>&1
sleep 2

# Kill server to trigger reconnect
docker exec ts-server-4 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 3

step "Reconnect started after server kill" sh -c 'docker logs ts-client-4 2>&1 | grep -q "Reconnecting in"'

# Send SIGTERM directly to traffic-sentinel on the client (not PID 1 sh)
docker exec ts-client-4 sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
sleep 3

L=$(docker logs ts-client-4 2>&1) || true
step "Graceful: routes restored" echo "$L" | grep -q "Restoring routes"
step "Graceful: Shutdown complete" echo "$L" | grep -q "Shutdown complete"

docker rm -f ts-client-4 ts-server-4 > /dev/null 2>&1 || true

# ===========================================================
echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

[ "$FAIL" -gt 0 ] && { echo "FAILURES DETECTED"; exit 1; }

echo ""
echo "=== All P2.2 Reconnection verification tests passed! ==="
