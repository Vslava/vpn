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
    for c in $(docker ps -a --format '{{.Names}}' 2>/dev/null | grep -E '^ts-mc-'); do
        docker kill "$c" 2>/dev/null || true
        docker rm -f "$c" 2>/dev/null || true
    done
    for n in $(docker network ls --format '{{.Name}}' 2>/dev/null | grep -E '^ts-mc[0-9]-'); do
        docker network rm "$n" 2>/dev/null || true
    done
    rm -f /tmp/ts-mc-*.toml
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
echo "  P4: Multi-client Verification (Docker)"
echo "=========================================="

echo ""
echo "=== Building release binary ==="
cargo build --release -q --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "=== Building Docker image ==="
docker build -q -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.e2e" "$PROJECT_DIR"

# ===========================================================
# Test 1: Two clients simultaneously with different IPs
# ===========================================================
echo ""
echo "--- Test 1: Two clients, different IPs ---"
cleanup_all
NET_NAME="ts-mc1-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-mc1-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

cat > /tmp/ts-mc1-client-a.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 3
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

cat > /tmp/ts-mc1-client-b.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 3
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-mc1-server /tmp/ts-mc1-server.toml
wait_for_log ts-mc1-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-mc1-client-a /tmp/ts-mc1-client-a.toml
wait_for_log ts-mc1-client-a "Handshake complete" 15 || { echo "FAIL: client A connect"; exit 1; }

start_client ts-mc1-client-b /tmp/ts-mc1-client-b.toml
wait_for_log ts-mc1-client-b "Handshake complete" 15 || { echo "FAIL: client B connect"; exit 1; }

sleep 5

step "Client A ping server" docker exec ts-mc1-client-a ping -c 2 -W 5 10.0.0.1
step "Client B ping server" docker exec ts-mc1-client-b ping -c 2 -W 3 10.0.0.1

# Verify they got different IPs
LOG_A=$(docker logs ts-mc1-server 2>&1 | sed 's/\x1b\[[0-9;]*m//g')
LOG_CLIENTS=$(echo "$LOG_A" | grep "Handshake complete" | grep "tun_ip")
echo "  Server log handshake lines:"
echo "$LOG_A" | grep "Handshake complete"

IP_A=$(echo "$LOG_A" | grep "Handshake complete" | head -1 | grep -oP 'tun_ip=\K[0-9.]+')
IP_B=$(echo "$LOG_A" | grep "Handshake complete" | tail -1 | grep -oP 'tun_ip=\K[0-9.]+')

step "Different client IPs" [ "$IP_A" != "$IP_B" ]

docker rm -f ts-mc1-client-a ts-mc1-client-b ts-mc1-server > /dev/null 2>&1 || true

# ===========================================================
# Test 2: Disconnect one client, other survives + IP reuse
# ===========================================================
echo ""
echo "--- Test 2: Client disconnect + IP reuse ---"
cleanup_all
NET_NAME="ts-mc2-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-mc2-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

cat > /tmp/ts-mc2-client-a.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 0
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

cat > /tmp/ts-mc2-client-b.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 3
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_server ts-mc2-server /tmp/ts-mc2-server.toml
wait_for_log ts-mc2-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-mc2-client-a /tmp/ts-mc2-client-a.toml
wait_for_log ts-mc2-client-a "Handshake complete" 10 || { echo "FAIL: client A connect"; exit 1; }

start_client ts-mc2-client-b /tmp/ts-mc2-client-b.toml
wait_for_log ts-mc2-client-b "Handshake complete" 10 || { echo "FAIL: client B connect"; exit 1; }

# Note assigned IP of client A
IP_A=$(docker logs ts-mc2-server 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep "Handshake complete" | head -1 | grep -oP 'tun_ip=\K[0-9.]+')
echo "  Client A IP: $IP_A"

# Kill client A
docker exec ts-mc2-client-a sh -c 'pkill -TERM -f traffic-sentinel' 2>/dev/null || true
echo "  Waiting for server to detect disconnect (~35s)..."
sleep 35

step "Client A disconnected" sh -c 'docker logs ts-mc2-server 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -q "Client disconnected"'
step "Client B still connected" docker exec ts-mc2-client-b ping -c 2 -W 3 10.0.0.1

# Start client C with same config — should get Client A's old IP
cat > /tmp/ts-mc2-client-c.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 0
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF

start_client ts-mc2-client-c /tmp/ts-mc2-client-c.toml
wait_for_log ts-mc2-client-c "Handshake complete" 10 || { echo "FAIL: client C connect"; exit 1; }

IP_C=$(docker logs ts-mc2-server 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep "Handshake complete" | tail -1 | grep -oP 'tun_ip=\K[0-9.]+')
echo "  Client C IP: $IP_C ($([ "$IP_C" = "$IP_A" ] && echo 'reused' || echo 'new'))"

step "Client C reused client A IP" [ "$IP_C" = "$IP_A" ]

docker rm -f ts-mc2-client-a ts-mc2-client-b ts-mc2-client-c ts-mc2-server > /dev/null 2>&1 || true

# ===========================================================
# Test 3: Three clients simultaneously
# ===========================================================
echo ""
echo "--- Test 3: Three clients ---"
cleanup_all
NET_NAME="ts-mc3-$$"
docker network create --subnet=172.30.0.0/16 "$NET_NAME" > /dev/null
SERVER_IP="172.30.0.10"

cat > /tmp/ts-mc3-server.toml << EOF
[tunnel]
psk = "$PSK"
mtu = 1400
[server]
listen = "0.0.0.0:8443"
EOF

for i in a b c; do
    cat > "/tmp/ts-mc3-client-${i}.toml" << EOF2
[tunnel]
psk = "$PSK"
mtu = 1400
[client]
remote = "${SERVER_IP}:8443"
max_retries = 0
reconnect_max_delay = 10
heartbeat_interval = 5
heartbeat_timeout = 15
EOF2
done

start_server ts-mc3-server /tmp/ts-mc3-server.toml
wait_for_log ts-mc3-server "[Ll]istening" 10 || { echo "FAIL: server start"; exit 1; }

start_client ts-mc3-client-a /tmp/ts-mc3-client-a.toml
wait_for_log ts-mc3-client-a "Handshake complete" 10 || { echo "FAIL: client A connect"; exit 1; }

start_client ts-mc3-client-b /tmp/ts-mc3-client-b.toml
wait_for_log ts-mc3-client-b "Handshake complete" 10 || { echo "FAIL: client B connect"; exit 1; }

start_client ts-mc3-client-c /tmp/ts-mc3-client-c.toml
wait_for_log ts-mc3-client-c "Handshake complete" 10 || { echo "FAIL: client C connect"; exit 1; }

sleep 3

step "Client A ping" docker exec ts-mc3-client-a ping -c 2 -W 3 10.0.0.1
step "Client B ping" docker exec ts-mc3-client-b ping -c 2 -W 3 10.0.0.1
step "Client C ping" docker exec ts-mc3-client-c ping -c 2 -W 3 10.0.0.1

HANDSHAKES=$(docker logs ts-mc3-server 2>&1 | grep -c "Handshake complete")
step "Three clients accepted" [ "$HANDSHAKES" -ge 3 ]

docker rm -f ts-mc3-client-a ts-mc3-client-b ts-mc3-client-c ts-mc3-server > /dev/null 2>&1 || true

# ===========================================================
echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

[ "$FAIL" -gt 0 ] && { echo "FAILURES DETECTED"; exit 1; }

echo ""
echo "=== All P4 Multi-client verification tests passed! ==="
