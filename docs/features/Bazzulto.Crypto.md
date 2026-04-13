# Bazzulto.Crypto

**Priority:** Medium — implement before networking (TLS dependency) and package signing.
**Area:** `userspace/lib/bsl/Bazzulto.Crypto/`

## Motivation

Any OS that wants to support HTTPS, signed packages, authenticated IPC, or
user passwords needs a solid cryptographic foundation. Rather than implement
primitives from scratch (error-prone and unaudited), Bazzulto.Crypto wraps the
[RustCrypto](https://github.com/RustCrypto) family of crates — all `no_std`,
battle-tested, and with built-in AArch64 hardware acceleration.

## Scope

### Hash functions
- SHA-256, SHA-384, SHA-512 (`sha2` crate)

### Symmetric encryption
- AES-CBC (`aes` + `cbc` crates)
- AES-GCM ⭐ — authenticated encryption, required for TLS 1.3 (`aes-gcm` crate)

### Message authentication
- HMAC-SHA-256 (`hmac` + `sha2` crates)

### Key derivation
- HKDF — used in TLS and modern key exchange (`hkdf` crate)
- PBKDF2 — used for password hashing (`pbkdf2` crate)

### Asymmetric cryptography
- ECDSA — digital signatures over P-256 / P-384 (`p256`, `p384` crates)
- ECDH — key exchange over P-256 / P-384 (same crates, `ecdh` feature)
- RSA — legacy certificates and signatures (`rsa` crate)

### Randomness
- `rand_core::RngCore` trait as the uniform interface
- Entropy source: `/dev/random` (kernel side: seeded from `CNTPCT_EL0` + virtio-rng when available)
- A `BazzultoRng` adapter exposes the kernel entropy to all crypto operations

### X.509 / PKI (future)
- ASN.1 parsing via `x509-cert` + `der` (RustCrypto)
- Certificate chain verification
- Root CA trust store at `/system/certs/`
- Required for HTTPS

## Hardware acceleration

AArch64 Crypto Extensions (AES, SHA instructions) are supported automatically
by the RustCrypto crates when the target features are enabled:

```
RUSTFLAGS="-C target-feature=+aes,+sha2"
```

QEMU `virt` emulates these instructions for Cortex-A. Real hardware (e.g.
Raspberry Pi 4, Apple M-series) executes them natively in one cycle per block.

## Crate dependencies

```toml
[dependencies]
sha2      = { version = "0.10", default-features = false }
aes       = { version = "0.8",  default-features = false, features = ["zeroize"] }
aes-gcm   = { version = "0.10", default-features = false, features = ["aes"] }
cbc       = { version = "0.1",  default-features = false }
hmac      = { version = "0.12", default-features = false }
hkdf      = { version = "0.12", default-features = false }
pbkdf2    = { version = "0.12", default-features = false }
p256      = { version = "0.13", default-features = false, features = ["ecdsa", "ecdh"] }
rsa       = { version = "0.9",  default-features = false }
rand_core = { version = "0.6",  default-features = false }
zeroize   = { version = "1",    default-features = false }
```

## Implementation order

1. **SHA-2 + HMAC** — hash and MAC, no external state needed
2. **AES-GCM** — authenticated symmetric encryption
3. **Entropy source** — `virtio-rng` driver or `CNTPCT`-seeded ChaCha20 CSPRNG → `/dev/random`
4. **HKDF + PBKDF2** — key derivation (trivial once SHA-2 exists)
5. **ECDSA / ECDH** (P-256) — modern asymmetric crypto, required for TLS 1.3
6. **RSA** — legacy compatibility (certificates, older TLS)
7. **X.509 + trust store** — prerequisite for HTTPS

## Relation to other subsystems

- **Networking (future):** TLS 1.3 requires AES-GCM, ECDH, HKDF, and SHA-256.
  Without Bazzulto.Crypto, HTTPS is impossible.
- **Package manager (future):** Package signatures use ECDSA over SHA-256.
- **User authentication:** Password storage uses PBKDF2 or HKDF over a salted hash.
- **IPC:** Authenticated channels between processes can use HMAC.
- **`/dev/random`:** The kernel entropy device is the single source of truth for
  all randomness — both the kernel ASLR (already implemented) and userspace
  crypto draw from the same pool.
