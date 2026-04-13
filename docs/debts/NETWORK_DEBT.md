# Network Stack — Pending

## Status: NOT IMPLEMENTED

No networking is present in the kernel. This is the largest remaining gap for a
complete OS capable of running server workloads or communicating over the network.

---

## Required subsystems

### 1. VirtIO-net driver (`platform/qemu_virt/virtio_net.rs`)

QEMU `virt` exposes a `virtio-net-device` on the virtio-mmio bus. The VirtIO block
driver (`virtio_blk.rs`) is already implemented and can serve as a reference.

Key steps:
- Detect `virtio-net` device during `virtio_mmio::enumerate()` (check device ID = 1).
- Negotiate features: at minimum `VIRTIO_NET_F_MAC` (feature bit 5).
- Set up two virtqueues: RX (index 0) and TX (index 1).
- TX: enqueue a buffer descriptor chain (header + packet data), kick the queue.
- RX: pre-populate the RX queue with receive buffers; handle IRQ to pull received
  frames and pass them to the network stack.
- Register an IRQ handler in `irq_dispatch()` (same pattern as `virtio_blk`).

Reference: VirtIO specification §5.1 (Network Device), QEMU `hw/net/virtio-net.c`.

### 2. Ethernet frame layer (`net/ethernet.rs`)

- Parse `EthernetHeader { dst_mac, src_mac, ethertype }`.
- Dispatch on `ethertype`: 0x0800 → IPv4, 0x0806 → ARP, 0x86DD → IPv6 (future).
- TX: prepend Ethernet header to outbound frames.
- MAC address: read from VirtIO-net config space offset 0 (6 bytes).

### 3. ARP (`net/arp.rs`)

- Maintain an ARP cache: `BTreeMap<Ipv4Addr, MacAddr>` (max 64 entries, LRU evict).
- Handle ARP request (opcode 1): reply with our MAC.
- Handle ARP reply (opcode 2): update cache.
- `arp_resolve(ip) -> Option<MacAddr>`: look up cache; if missing, send ARP request
  and block the caller (store waiter PID, wake on ARP reply).

### 4. IPv4 (`net/ipv4.rs`)

- Parse `Ipv4Header`: version, IHL, DSCP, total length, TTL, protocol, checksum,
  src/dst addresses.
- Validate checksum (ones-complement).
- Dispatch on protocol: 0x01 → ICMP, 0x06 → TCP, 0x11 → UDP.
- TX: fill header, compute checksum, pass to Ethernet layer.
- Static routing: one default gateway entry is sufficient for QEMU NAT networking.

### 5. ICMP (`net/icmp.rs`)

- Handle Echo Request (type 8): reply with Echo Reply (type 0). Required for `ping`.
- Handle Destination Unreachable (type 3): propagate to TCP/UDP socket as error.

### 6. UDP (`net/udp.rs`)

- Parse `UdpHeader { src_port, dst_port, length, checksum }`.
- Demultiplex to bound socket by (local_addr, local_port).
- TX: compute UDP checksum (IPv4 pseudo-header + UDP header + data).
- No connection state — each send/recv is independent.

### 7. TCP (`net/tcp.rs`)

TCP is the most complex component. Minimum viable implementation:
- State machine: CLOSED → SYN_SENT → ESTABLISHED → FIN_WAIT_1 → TIME_WAIT → CLOSED
  (active open); LISTEN → SYN_RECEIVED → ESTABLISHED → CLOSE_WAIT → LAST_ACK
  (passive open).
- Retransmission timer (use existing kernel tick counter).
- Receive buffer: ring buffer per connection (default 64 KB).
- Send buffer: ring buffer per connection (default 64 KB).
- Sliding window flow control (advertised window in ACK).
- Nagle algorithm (optional, can be disabled with TCP_NODELAY).

**Alternative**: Integrate `smoltcp` (a `no_std` Rust TCP/IP stack). It handles
Ethernet, ARP, IPv4, ICMP, UDP, and TCP. The kernel provides a device trait
(`smoltcp::phy::Device`) backed by the VirtIO-net driver. This avoids writing
the full TCP state machine from scratch.

Reference: RFC 793 (TCP), RFC 791 (IPv4), RFC 826 (ARP).

### 8. BSD socket API (`net/socket.rs` + `syscall/mod.rs`)

Extend the existing Unix domain socket syscalls to support `AF_INET`:
- `socket(AF_INET, SOCK_STREAM, 0)` → TCP socket.
- `socket(AF_INET, SOCK_DGRAM, 0)` → UDP socket.
- `bind(sockfd, sockaddr_in, len)` → bind to local port.
- `connect(sockfd, sockaddr_in, len)` → TCP three-way handshake.
- `listen` / `accept` → TCP server.
- `send` / `recv` / `sendto` / `recvfrom` → data transfer.
- `setsockopt(TCP_NODELAY, SO_REUSEADDR, SO_RCVBUF, SO_SNDBUF)` → socket options.
- `gethostbyname` is a userspace concern (DNS resolver library), not a syscall.

`sockaddr_in` layout (matches Linux/POSIX):
```
struct sockaddr_in {
    sin_family: u16,   // AF_INET = 2
    sin_port:   u16,   // big-endian port
    sin_addr:   u32,   // big-endian IPv4 address
    sin_zero:   [u8; 8],
}
```

### 9. DHCP client (userspace or kernel)

For QEMU NAT networking, a DHCP client is needed to obtain an IP address from
QEMU's built-in DHCP server (typically assigns 10.0.2.15/24, gateway 10.0.2.2).

This can be implemented in userspace (send/recv UDP on port 68) once the socket
API is available, or as a small kernel-side init step.

### 10. Network configuration (`/proc/net/`)

- `/proc/net/if_inet6` — not required initially.
- `/proc/net/arp` — ARP cache dump.
- `/proc/net/tcp` — TCP socket table (connection states, ports).
- `/proc/net/udp` — UDP socket table.

---

## Implementation order

1. VirtIO-net driver (hardware layer)
2. Ethernet + ARP (link layer)
3. IPv4 + ICMP (network layer)
4. UDP (transport, simpler than TCP)
5. BSD socket API for AF_INET/SOCK_DGRAM
6. TCP state machine (or integrate smoltcp)
7. BSD socket API for AF_INET/SOCK_STREAM
8. DHCP client (userspace)
