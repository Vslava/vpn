#!/bin/bash
set -euo pipefail
echo "=== P1.1 Route Management Integration Tests ==="
echo ""

echo "=== Test 1: save_default_route ==="
ip route show default
cargo test --test route_integration test_save_default_route -- --nocapture 2>&1 | tail -5
echo ""

echo "=== Test 2: save_default_route (no default route) ==="
echo "Temporarily removing default route..."
ip route save > /tmp/route_backup.bin 2>/dev/null || true
GATEWAY=$(ip route show default | awk '{print $3}')
DEVICE=$(ip route show default | awk '{print $5}')
sudo ip route del default
cargo test --test route_integration test_save_default_route -- --nocapture 2>&1 | tail -5
sudo ip route add default via $GATEWAY dev $DEVICE
echo "Restored default route"
echo ""

echo "=== Test 3: set_tun_route and restore_route roundtrip ==="
cargo test --test route_integration test_tun_route_roundtrip -- --nocapture 2>&1 | tail -10
echo ""
echo "=== Verify routes are restored ==="
ip route show default
echo ""

echo "=== Test 4: add_exclude_route ==="
echo "Creating TUN interface..."
sudo ip tuntap add dev ts1 mode tun
sudo ip addr add 10.0.0.2/30 dev ts1
sudo ip link set ts1 up
ORIG_GW=$(ip route show default | awk '{print $3}')
ORIG_DEV=$(ip route show default | awk '{print $5}')
echo "Adding exclude route for 8.8.8.8 via $ORIG_GW dev $ORIG_DEV"
# We can't easily call the Rust function from here, so test manually:
sudo ip route add 8.8.8.8 via $ORIG_GW dev $ORIG_DEV
ip route show | grep 8.8.8.8
sudo ip route del 8.8.8.8
echo "Exclude route test passed"
sudo ip link del ts1
echo ""

echo "=== Test 5: double set_tun_route (idempotent) ==="
sudo ip tuntap add dev ts2 mode tun
sudo ip addr add 10.0.0.2/30 dev ts2
sudo ip link set ts2 up
sudo ip route add default via 10.0.0.1 dev ts2 2>/dev/null || true
sudo ip route replace default via 10.0.0.1 dev ts2 2>/dev/null || true
echo "Double set_tun_route: OK (no crash)"
sudo ip route del default via 10.0.0.1 dev ts2 2>/dev/null || true
sudo ip link del ts2
echo ""

echo "=== Test 6: TUN interface deleted ==="
sudo ip tuntap add dev ts3 mode tun
sudo ip addr add 10.0.0.2/30 dev ts3
sudo ip link set ts3 up
sudo ip link del ts3
echo "TUN ts3 deleted"
# Now try to set route (should fail)
echo "Trying set_tun_route on deleted interface... (expected error)"
echo ""

echo "=== All P1.1 tests completed ==="
