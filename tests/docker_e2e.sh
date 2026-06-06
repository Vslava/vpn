#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
NET_NAME="ts-e2e-$$"
SERVER_NAME="ts-server-$$"
CLIENT_NAME="ts-client-$$"
SERVER_TOML="/tmp/ts-server-$$.toml"
CLIENT_TOML="/tmp/ts-client-$$.toml"
PSK="deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
IMAGE="traffic-sentinel-e2e:latest"

PASS=0
FAIL=0

cleanup() {
    echo "=== Cleaning up ==="
    docker kill "$SERVER_NAME" 2>/dev/null || true
    docker kill "$CLIENT_NAME" 2>/dev/null || true
    docker rm -f "$SERVER_NAME" 2>/dev/null || true
    docker rm -f "$CLIENT_NAME" 2>/dev/null || true
    docker network rm "$NET_NAME" 2>/dev/null || true
    rm -f "$SERVER_TOML" "$CLIENT_TOML"
}
trap cleanup EXIT

run_test() {
    local name="$1"
    shift
    echo ""
    echo "--- Test: $name ---"
    if eval "$@"; then
        echo "=== PASS ==="
        PASS=$((PASS + 1))
    else
        echo "=== FAIL ==="
        FAIL=$((FAIL + 1))
    fi
}

echo "=== Building release binary ==="
cargo build --release -q --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "=== Building Docker e2e image ==="
docker build -q -t "$IMAGE" -f "$SCRIPT_DIR/Dockerfile.e2e" "$PROJECT_DIR"

echo "=== Creating Docker network ==="
docker network create "$NET_NAME"

echo "=== Generating configs ==="
cat > "$SERVER_TOML" <<EOF
[tunnel]
psk = "$PSK"
mtu = 1400

[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > "$CLIENT_TOML" <<EOF
[tunnel]
psk = "$PSK"
mtu = 1400

[client]
remote = "SERVER_IP_PLACEHOLDER:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
EOF

echo "=== Starting server ==="
docker run -d --name "$SERVER_NAME" \
    --network "$NET_NAME" \
    --cap-add NET_ADMIN \
    --device /dev/net/tun \
    --sysctl net.ipv4.ip_forward=1 \
    -e RUST_LOG=info \
    -v "$SERVER_TOML:/etc/traffic-sentinel.toml:ro" \
    --entrypoint sh \
    "$IMAGE" -c '
        mkdir -p /dev/net
        [ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
        chmod 666 /dev/net/tun
        traffic-sentinel --mode server --config /etc/traffic-sentinel.toml &
        TS_PID=$!
        for i in $(seq 1 20); do
            ip addr show ts0 2>/dev/null | grep -q "10.0.0.1" && break
            sleep 0.5
        done
        socat TCP-LISTEN:9999,bind=10.0.0.1,fork,reuseaddr EXEC:cat &
        echo "socat echo server started on 10.0.0.1:9999"
        iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE 2>/dev/null || true
        wait $TS_PID
    ' > /dev/null

SERVER_IP=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$SERVER_NAME")
echo "Server IP: $SERVER_IP"
sed -i "s/SERVER_IP_PLACEHOLDER/$SERVER_IP/" "$CLIENT_TOML"

echo "=== Waiting for server to listen ==="
for i in $(seq 1 10); do
    docker logs "$SERVER_NAME" 2>&1 | grep -qi "listening" && { echo "Server ready (attempt $i)"; break; }
    [ "$i" -eq 10 ] && { echo "ERROR: Server did not start"; docker logs "$SERVER_NAME"; exit 1; }
    sleep 1
done

echo "=== Starting client ==="
docker run -d --name "$CLIENT_NAME" \
    --network "$NET_NAME" \
    --cap-add NET_ADMIN \
    --device /dev/net/tun \
    -e RUST_LOG=info \
    -v "$CLIENT_TOML:/etc/traffic-sentinel.toml:ro" \
    --entrypoint sh \
    "$IMAGE" -c '
        mkdir -p /dev/net
        [ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
        chmod 666 /dev/net/tun
        traffic-sentinel --mode client --config /etc/traffic-sentinel.toml
    ' > /dev/null

echo "=== Waiting for client handshake ==="
for i in $(seq 1 10); do
    docker logs "$CLIENT_NAME" 2>&1 | grep -qi "handshake complete" && { echo "Client ready (attempt $i)"; break; }
    [ "$i" -eq 10 ] && { echo "ERROR: Client handshake failed"; docker logs "$SERVER_NAME"; docker logs "$CLIENT_NAME"; exit 1; }
    sleep 1
done
sleep 1

echo ""
echo "=========================================="
echo "  Test suite: End-to-End Tunnel Tests"
echo "=========================================="

# 1. ICMP to server TUN IP
run_test "ICMP: ping 10.0.0.1 through tunnel" \
    "docker exec $CLIENT_NAME ping -c 3 -W 3 10.0.0.1"

# 2. TCP echo through tunnel to socat on server TUN IP
run_test "TCP: echo through tunnel to server TUN IP" \
    "docker exec $CLIENT_NAME sh -c 'echo hello-vpn | timeout 5 socat - TCP:10.0.0.1:9999,connect-timeout=3 | grep -q hello-vpn'"

# 3. HTTP to internet through tunnel
run_test "HTTP: curl example.com through tunnel" \
    "docker exec $CLIENT_NAME curl -s --connect-timeout 10 https://example.com | head -5 | grep -q html"

# 4. DNS to internet through tunnel
run_test "DNS: dig google.com @8.8.8.8 through tunnel" \
    "docker exec $CLIENT_NAME dig +short google.com @8.8.8.8 +timeout=5 | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'"

# 5. ICMP to internet through tunnel
run_test "ICMP: ping 8.8.8.8 through tunnel" \
    "docker exec $CLIENT_NAME ping -c 3 -W 3 8.8.8.8"

echo ""
echo "=========================================="
echo "  Results: $PASS passed, $FAIL failed"
echo "=========================================="

if [ "$FAIL" -gt 0 ]; then
    echo ""
    docker logs "$SERVER_NAME" 2>&1 || true
    docker logs "$CLIENT_NAME" 2>&1 || true
    exit 1
fi

echo ""
echo "=== All tests passed! ==="
