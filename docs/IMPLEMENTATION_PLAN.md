# Implementation Plan: traffic-sentinel

> **Note**: P0–P2 реализованы на TCP-транспорте. P3 (текущая фаза) переводит транспорт на UDP для устранения TCP-over-TCP meltdown. Ниже — оригинальный план (P0–P2) и новый план (P3). После P3 project structure и key interfaces будут обновлены под UDP.

## 1. Project Structure

```
traffic-sentinel/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI entry, mode dispatch
│   ├── config.rs         # TOML config: [client], [server], [tunnel]
│   ├── error.rs          # Unified error type
│   ├── crypto.rs         # XChaCha20-Poly1305 encrypt/decrypt, X25519 ECDH
│   ├── protocol.rs       # Packet framing: length + seq + flags + nonce + payload
│   ├── tun.rs            # TUN interface create/delete, IP/MTU/route setup
│   ├── route.rs          # Save/restore default route
│   ├── transport.rs      # TCP connect/listen, read/write framed packets
│   ├── handshake.rs      # Hybrid PSK + X25519 ECDH handshake
│   ├── client.rs         # Client pipeline: TUN→encrypt→TCP, TCP→decrypt→TUN
│   ├── server.rs         # Server pipeline: TCP→decrypt→forward, forward→encrypt→TCP
│   └── signal.rs         # SIGTERM/SIGINT → graceful shutdown
```

## 2. Dependencies (Cargo.toml)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
tun-rs = "2"                              # TUN interface (async)
xchacha20poly1305 = "0.10"                # AEAD cipher
x25519-dalek = { version = "2", features = ["static_secrets"] }
rand = "0.8"                              # CSPRNG for nonce + ephemeral keys
serde = { version = "1", features = ["derive"] }
toml = "0.8"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
# Linux only:
rtnetlink = "0.14"                        # netlink for route management
# Windows only (cfg-windows):
# iphlpapi — через winapi
```

## 3. Phase P0 — Proof of Concept

**Goal**: TUN → read → encrypt → TCP → decrypt → log. No route management, no handshake, hardcoded PSK.

### Step P0.1: Project init + scaffolding

- `cargo init --name traffic-sentinel`
- Write `Cargo.toml` with all dependencies
- Create `src/main.rs`: clap arg parsing (`--mode client|server`, `--config`)
- Create `src/config.rs`: minimal `Config` struct with `Mode` enum
- Create `src/error.rs`: `Error` enum wrapping `io::Error`, `tun::Error`, crypto errors

### Step P0.2: TUN interface (client mode)

- Implement `src/tun.rs`:
  - `create_tun(name: &str, mtu: u16, ip: Ipv4Addr) -> Result<Box<dyn AsyncTun>>`
  - Uses `tun-rs` v2 async API (`TunBuilder` → `Tun::async_()`)
  - Hardcoded: name `ts0`, MTU 1400, IP `10.0.0.2/30`
- Test: run as root, verify `ip addr show ts0`

### Step P0.3: Crypto engine

- Implement `src/crypto.rs`:
  - `struct Crypto { key: [u8; 32] }`
  - `encrypt(&self, nonce: &[u8; 24], plaintext: &[u8]) -> Result<Vec<u8>>`
  - `decrypt(&self, nonce: &[u8; 24], ciphertext: &[u8]) -> Result<Vec<u8>>`
  - Uses `xchacha20poly1305::XChaCha20Poly1305`
  - Nonce: random 24 bytes via `rand::rngs::OsRng`
- PSK: hardcoded hex string in P0, load from config later

### Step P0.4: Packet protocol framing

- Implement `src/protocol.rs`:
  - Frame format (TCP payload):
    ```
    [length: u16 BE] [nonce: 24 bytes] [seq: u32 BE] [flags: u8] [encrypted payload]
    ```
  - `fn encode(nonce, seq, flags, payload) -> Vec<u8>`
  - `fn decode(data: &[u8]) -> Result<(u16, [u8;24], u32, u8, Vec<u8>)>`
  - Length = nonce (24) + seq (4) + flags (1) + encrypted payload

### Step P0.5: TCP transport

- Implement `src/transport.rs`:
  - `connect(addr: SocketAddr) -> Result<TcpStream>` (tokio)
  - `listen(addr: SocketAddr) -> Result<TcpListener>` (tokio)
  - `read_frame(stream: &mut TcpStream) -> Result<Vec<u8>>` — read 2 bytes length, then N bytes
  - `write_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()>`
  - `TCP_NODELAY` on both sides

### Step P0.6: Client pipeline

- Implement `src/client.rs`:
  - Two tokio tasks:
    - **tun_to_tcp**: loop: `tun.read()` → `crypto.encrypt(random_nonce, buf)` → `protocol.encode(...)` → `tcp.write_all(frame)`
    - **tcp_to_tun**: loop: `read_frame(tcp)` → `protocol.decode(frame)` → `crypto.decrypt(nonce, payload)` → `tun.write(packet)`
  - Sequential pipeline — no channels, each task blocks on I/O

### Step P0.7: Server pipeline

- Implement `src/server.rs`:
  - Accept single TCP connection
  - Two tasks mirroring client:
    - **tcp_to_tun**: read frame → decrypt → write to tun (or just log in P0)
    - **tun_to_tcp**: read from tun → encrypt → write frame to tcp (not needed until bidirectional)
  - P0: server only decrypts and logs received packets

### Step P0.8: main.rs dispatch

- Match `--mode`:
  - `client`: init TUN → init crypto → connect TCP → spawn two tasks
  - `server`: init TUN → init crypto → listen TCP → accept → spawn two tasks
- Use `tokio::select!` or `JoinSet` to run both tasks

### Step P0.9: Loopback test

- Run server on localhost:8443
- Run client with TUN → packets get encrypted, sent, decrypted, logged
- Verify: `ping 10.0.0.1` from client → ICMP appears decrypted on server

### P0 Artifacts

- All above files compiling
- Successful loopback test (ping packet flows TUN → encrypt → TCP → decrypt → log)

---

## 4. Phase P1 — Minimal Viable

**Goal**: Full encrypted bidirectional forwarding with route management and ECDH handshake.

### Step P1.1: Route management

- Implement `src/route.rs`:
  - `save_default_route() -> DefaultRoute` — parse `/proc/net/route` (Linux) or `ip route show default` (fallback)
  - `set_default_tun_route(tun_ip: Ipv4Addr) -> Result<()>` — `ip route replace default via <tun_ip> dev ts0`
  - `add_exclude_route(server_ip: Ipv4Addr)` — route to server via original gateway (avoid loop)
  - `restore_route(original: DefaultRoute)` — on shutdown
  - Windows: WinAPI `CreateIpForwardEntry` / `DeleteIpForwardEntry`

### Step P1.2: ECDH handshake

- Implement `src/handshake.rs`:
  - **Client**:
    1. Generate ephemeral X25519 keypair
    2. Send `[PSK_hmac(public_key)]` + public_key (32 bytes) to server
    3. Receive server's public_key (32 bytes)
    4. Compute shared secret: `x25519(local_secret, remote_public)` → `blake2b(key, psk)` → session key
  - **Server**:
    1. Receive client's public_key + HMAC, verify HMAC with PSK
    2. Send own ephemeral public_key
    3. Compute same shared secret
  - `hybrid_key_exchange(stream, psk) -> Result<[u8; 32]>`
  - Session key used for all subsequent `Crypto` operations
  - Handshake happens once before data flow

### Step P1.3: Server forwarder

- Server side: after decryption, forward IP packets to real network:
  - **Option A (Linux)**: Raw socket `IPPROTO_RAW` with `IP_HDRINCL` — send decrypted IP packet as-is
  - **Option B (cross-platform)**: Second TUN interface on server — write decrypted packets into TUN, let OS route them
  - **Decision**: Use raw socket on Linux (P1), TUN fallback later
  - Response capture: raw socket `IPPROTO_RAW` in IP_HDRINCL mode only sends, doesn't receive. For receiving responses: need `IPPROTO_TCP`/`IPPROTO_UDP` sockets or TUN on server side
  - **Actual approach (P1)**: Server creates its own TUN interface, writes decrypted packets to it, reads response packets from it → encrypt → send back

### Step P1.4: Full config loading

- `src/config.rs`:
  - `[tunnel] psk = "hex..." mtu = 1400`
  - `[client] remote = "server-ip:8443" tun_ip = "10.0.0.2/30"`
  - `[server] listen = "0.0.0.0:8443" tun_ip = "10.0.0.1/30"`
  - Deserialize with serde, validate ranges

### Step P1.5: Wire into main.rs

- Update `main.rs` to:
  - Load config → init crypto (handshake) → init TUN → init route → start pipelines
  - Use config values instead of hardcoded

### P1 Artifacts

- End-to-end: client machine traffic flows through TUN → encrypted TCP → server → internet
- Response traffic flows back correctly
- Routes are restored on Ctrl+C

---

## 5. Phase P2 — Production Ready

**Goal**: Robust error handling, reconnection, graceful shutdown, logging.

### Step P2.1: Graceful shutdown

- `src/signal.rs`:
  - `tokio::signal::ctrl_c()` + `unix::signal(SIGTERM)`
  - On signal: restore routes → delete TUN → close TCP → exit 0
  - Drop order: `route.restore()` → `tun.delete()` → `stream.shutdown()`
  - Use `tokio::select!` in main to wait for signal or pipeline error

### Step P2.2: Reconnection (client)

- TCP disconnect detection: errors from `read_frame` / `write_frame`
- Reconnect loop:
  - Backoff: 1s, 2s, 4s, 8s, 16s, 30s (cap)
  - Repeat handshake, increment seq counter (reset nonce state)
  - If max retries exceeded → graceful shutdown
- Server side: detect disconnect, wait for new connection

### Step P2.3: Heartbeat / keepalive

- `TCP_KEEPALIVE` on TcpStream
- Application-level ping every 30s (empty frame with `flags = PING`)
- If no response in 60s → treat as dead → reconnect

### Step P2.4: Logging

- `tracing-subscriber` init in `main.rs`
- Events at key points:
  - `info`: startup, handshake complete, connected, reconnecting, shutdown
  - `debug`: packet sent/received, frame details
  - `warn`: slow read/write, retry attempts
  - `error`: crypto errors, TCP errors, TUN errors

### Step P2.5: Error handling audit

- All `Result` returns with meaningful `Error` variants
- No `unwrap()` / `expect()` in production paths (only in tests)
- Pipeline errors lead to reconnect (client) or shutdown (server)

### Step P2.6: Tests

- Unit tests:
  - `crypto.rs`: encrypt → decrypt roundtrip with random nonces
  - `protocol.rs`: encode → decode roundtrip
  - `handshake.rs`: client + server handshake produces matching keys
- Integration test (requires root):
  - Start server on localhost, connect client, verify TUN ↔ TUN loop

### P2 Artifacts

- Stable reconnection
- Clean shutdown on SIGTERM/SIGINT
- Structured logging throughout
- Test suite

---

---

## 6. Phase P3 — UDP Transport (Hardening)

**Goal**: Replace TCP transport with UDP to eliminate TCP-over-TCP meltdown. Performance-critical streaming (video) currently degrades because two TCP layers (inner application TCP + outer tunnel TCP) both handle retransmission, causing cascading congestion window collapse. UDP transport leaves reliability to inner TCP alone.

### Step P3.1: Protocol — remove length prefix

UDP datagrams are self-framing — no need for a 2-byte length prefix.

- **`src/protocol.rs`**:
  - `encode()` produces `[nonce:24][seq:4][flags:1][payload]` (no length prefix)
  - `decode()` parses the same format
  - Update all tests

### Step P3.2: Transport — UDP sockets

- **`src/transport.rs`**:
  - Remove `TcpListener`/`TcpStream`/`read_frame`/`write_frame`/`set_keepalive`
  - Add:
    - `udp_bind(addr: SocketAddr) -> Result<UdpSocket>` — bind + set SO_RCVBUF/SO_SNDBUF (2MB)
    - `udp_connect(addr: SocketAddr) -> Result<UdpSocket>` — ephemeral bind + connect to remote
  - No keepalive needed (application-level heartbeat already exists)

### Step P3.3: Handshake over UDP

- **`src/handshake.rs`**:
  - Replace `AsyncReadExt + AsyncWriteExt` (TCP stream) with `UdpSocket`
  - `client_handshake(socket: &UdpSocket, psk) -> Result<[u8; 32]>`:
    1. Send 64-byte client_hello (pubkey + HMAC)
    2. Wait for 64-byte response with 3s timeout
    3. On timeout → retransmit (up to 5 retries)
    4. Verify server HMAC, derive session key
  - `server_handshake(socket: &UdpSocket, psk) -> Result<([u8; 32], SocketAddr)>`:
    1. Wait for 64-byte datagram from any source
    2. Verify client HMAC
    3. Send 64-byte server_hello
    4. Return session key + client address
  - Update all tests (use real UDP sockets instead of `tokio::io::duplex`)

### Step P3.4: Server — UDP data loop

- **`src/server.rs`**:
  - Bind UDP instead of TCP listener
  - Call `server_handshake()` → get client address + session key
  - Data loop with UDP `recv_from` / `send_to`:
    - **h1**: UDP → TUN — `recv_from()` → filter by client_addr → decode → decrypt → handle PING → `tun.send()`
    - **h2**: TUN → UDP + PONG channel — `tun.recv()` → encrypt → encode → `send_to(client_addr)`
  - Drop datagrams from unknown sources during active session
  - Server-side timeout: if no data from client for 180s → back to handshake wait

### Step P3.5: Client — UDP data loop

- **`src/client.rs`**:
  - Use `udp_connect()` instead of TCP connect
  - Data loop with UDP `send` / `recv` (connected socket):
    - **h1**: TUN → UDP — `tun.recv()` → encrypt → encode → `socket.send()`
    - **h2**: UDP → TUN — `socket.recv()` → decode → decrypt → handle PONG → `tun.send()`
  - Heartbeat: PING/PONG over UDP (unchanged)
  - Timeout watchdog: unchanged

### Step P3.6: Update Docker test scripts

- `tests/docker_e2e.sh`, `tests/docker_heartbeat.sh`, `tests/docker_reconnect.sh`:
  - No TCP-specific setup needed (no `iptables -A OUTPUT -p tcp --dport 8443 -j DROP` etc.)
  - Test scripts remain largely the same (they test app behavior, not transport protocol)

### P3 Artifacts

- TCP-over-TCP meltdown eliminated
- Throughput for video streaming restored (tested via Docker e2e)
- All existing tests pass (updated for UDP)
- Handshake retransmission works (client retries on packet loss)
- No new dependencies

---

## 7. Platform-Specific Notes

### Linux (v1)

| Feature | Approach |
|---------|----------|
| TUN | `tun-rs` v2 async |
| Routes | `rtnetlink` (netlink) via `neli` or fallback to `Command("ip")` |
| Raw socket | `socket(AF_INET, SOCK_RAW, IPPROTO_RAW)` with `IP_HDRINCL` |
| Permissions | `sudo`, check `uid == 0` or `CAP_NET_ADMIN` |

### Windows (v1)

| Feature | Approach |
|---------|----------|
| TUN | `tun-rs` v2 with `wintun` DLL (bundled) |
| Routes | WinAPI `CreateIpForwardEntry` |
| Permissions | UAC `requireAdministrator` via manifest |
| Raw socket | Not available → use TUN on server side |

### Conditional compilation

```rust
#[cfg(target_os = "linux")]
mod route_linux;

#[cfg(target_os = "windows")]
mod route_windows;
```

---

## 7. Key Interfaces

```rust
// tun.rs
pub trait TunInterface: Send {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    async fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    fn mtu(&self) -> u16;
}
pub async fn create_tun(name: &str, mtu: u16, ip: Ipv4Addr) -> Result<impl TunInterface>;

// crypto.rs
pub struct Crypto { key: [u8; 32] }
impl Crypto {
    pub fn new(key: [u8; 32]) -> Self;
    pub fn generate_nonce() -> [u8; 24];
    pub fn encrypt(&self, nonce: &[u8; 24], plain: &[u8]) -> Result<Vec<u8>>;
    pub fn decrypt(&self, nonce: &[u8; 24], cipher: &[u8]) -> Result<Vec<u8>>;
}

// protocol.rs
pub struct Frame {
    pub nonce: [u8; 24],
    pub seq: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}
pub fn encode(frame: &Frame) -> Vec<u8>;
pub fn decode(data: &[u8]) -> Result<Frame>;

// handshake.rs
pub async fn client_handshake(stream: &mut TcpStream, psk: &[u8; 32]) -> Result<[u8; 32]>;
pub async fn server_handshake(stream: &mut TcpStream, psk: &[u8; 32]) -> Result<[u8; 32]>;

// route.rs
pub struct DefaultRoute { /* platform-specific */ }
pub async fn save_default_route() -> Result<DefaultRoute>;
pub async fn set_tun_route(tun_gw: Ipv4Addr) -> Result<()>;
pub async fn add_exclude_route(server_ip: Ipv4Addr) -> Result<()>;
pub async fn restore_route(route: DefaultRoute) -> Result<()>;

// transport.rs (P0–P2: TCP; P3: UDP)
pub async fn udp_bind(addr: SocketAddr) -> Result<UdpSocket>;
pub async fn udp_connect(addr: SocketAddr) -> Result<UdpSocket>;

// handshake.rs (P3: UDP)
pub async fn client_handshake(socket: &UdpSocket, psk: &[u8; 32]) -> Result<[u8; 32]>;
pub async fn server_handshake(socket: &UdpSocket, psk: &[u8; 32]) -> Result<([u8; 32], SocketAddr)>;

// protocol.rs (P3: no length prefix)
pub fn encode(frame: &Frame) -> Vec<u8>;       // [nonce:24][seq:4][flags:1][payload]
pub fn decode(data: &[u8]) -> Result<Frame>;   // no length prefix
```

---

## 8. Execution Order

```
P0.1  ─→ P0.2 ─→ P0.3 ─→ P0.4 ─→ P0.5 ─→ P0.6 ─→ P0.7 ─→ P0.8 ─→ P0.9
                                                              ↓
P1.1 ────────────────────────────────→ P1.2 ─→ P1.3 ─→ P1.4 ─→ P1.5
                                              ↓
P2.1 ─→ P2.2 ─→ P2.3 ─→ P2.4 ─→ P2.5 ─→ P2.6
                                              ↓
P3.1 ─→ P3.2 ─→ P3.3 ─→ P3.4 ─→ P3.5 ─→ P3.6
```

- P0 steps are sequential (each builds on the previous)
- P1.1 (route) and P1.2 (handshake) can be developed in parallel
- P2 steps are mostly independent
- P3 steps must be sequential (each file depends on previous changes)

---

## 9. Acceptance Criteria per Phase

### P0
- [ ] TUN interface `ts0` created with IP 10.0.0.2/30
- [ ] Ping to 10.0.0.1 produces readable ICMP on server side
- [ ] Encrypted TCP stream contains no plaintext IP headers
- [ ] Server decrypts payload correctly

### P1
- [ ] Default route switched to TUN on client start
- [ ] Original route restored on shutdown
- [ ] ECDH handshake produces matching session keys
- [ ] HTTP request from client reaches internet via server
- [ ] Response reaches client application

### P2
- [ ] SIGTERM → clean shutdown in < 1s
- [ ] TCP disconnect → reconnect within 30s
- [ ] All errors logged with context
- [ ] `cargo test --all` passes

### P3
- [ ] Transport переведён с TCP на UDP
- [ ] encode/decode в protocol.rs не используют length prefix
- [ ] Handshake работает поверх UDP с retransmission на клиенте
- [ ] UDP-датаграмма не превышает ~1445 байт (TUN MTU 1400 + overhead 45)
- [ ] Видео (HTTP/TCP стриминг) работает без деградации скорости
- [ ] Все существующие тесты проходят (обновлены под UDP)
- [ ] Docker e2e тесты проходят
- [ ] `cargo clippy -- -D warnings` — чисто
