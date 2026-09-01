# Deployment Architecture

```mermaid
flowchart TD
 DEV[Developer] --> CI[CI Pipeline]
 CI --> TEST[Test + Fuzz Gates]
 TEST --> BUILD[Build Rust + Swift]
 BUILD --> SIGN[Code Signing]
 SIGN --> NOTAR[Notarization]
 NOTAR --> PKG[DMG/Installer]
 PKG --> MAC[User Mac]
 MAC --> APP[App + Helper]
```

Release artifacts are versioned and traceable to source revisions.