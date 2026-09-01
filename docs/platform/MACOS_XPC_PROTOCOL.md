# macOS XPC Protocol

Operations: GetSourceMetadata(source_ref), OpenReadOnly(source_ref), Read(handle, offset, length), Close(handle).

Validate client identity using supported macOS mechanisms, validate schemas and limits, resolve sources server-side, reject write flags.

Handles are opaque, bound to client/session, expire on inactivity and are invalidated on disconnect.

TOCTOU: resolve identity → fingerprint → open → revalidate → return handle.

No arbitrary path reads, shell execution, mount modification, repair or source writes.