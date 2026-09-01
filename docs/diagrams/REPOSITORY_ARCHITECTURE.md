# Repository Architecture

```text
data-recovery/
├── apps/
│   └── macos/
├── crates/
│   ├── recovery-core/
│   ├── storage-io/
│   ├── partition/
│   ├── filesystem-fat/
│   ├── filesystem-exfat/
│   ├── carving/
│   ├── validators/
│   ├── session-store/
│   └── ffi/
├── tests/
│   ├── corpus/
│   ├── integration/
│   └── fixtures/
├── docs/
└── scripts/
```

```mermaid
flowchart TD
 APP[apps/macos] --> FFI[crates/ffi]
 FFI --> CORE[crates/recovery-core]
 CORE --> IO[storage-io]
 CORE --> PART[partition]
 CORE --> FAT[filesystem-fat]
 CORE --> EXFAT[filesystem-exfat]
 CORE --> CARVE[carving]
 CORE --> VAL[validators]
 CORE --> STORE[session-store]
```