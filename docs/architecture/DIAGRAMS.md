# Architecture Diagrams

## System Context
```
User
  |
  v
Data Recovery Application (macOS and Windows)
  |--------- Internal Storage
  |--------- External Storage
  \--------- Disk Images
```

## Recovery Pipeline
```
Source → Profile → Safety Policy
                  |
           +------+------+
           |             |
          Scan       Image First
           |             |
           +------+------+
                  v
              Results
                  v
            Validation
                  v
             Recovery
```

## Privilege Boundary
```
Tauri App (unprivileged)
          |
          | authenticated IPC
          v
Privileged Helper (minimal scope)
          |
          v
macOS Storage APIs
          |
          v
Physical Storage
```

## Result Pipeline
```
Filesystem Results ----+
                       +--> Normalize --> Deduplicate --> Confidence --> Store
Carving Results -------+
```