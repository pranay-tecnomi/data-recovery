# Windows Platform and Privilege Specification

Counterpart to MACOS_PLATFORM_AND_PRIVILEGE_SPECIFICATION.md. Both realise the
PlatformDevice seam defined in ADR-011.

## Device access
Physical drives are opened as `\\.\PhysicalDriveN`; mounted volumes as `\\.\X:`.
Enumeration uses the SetupAPI disk-class interface, with geometry from
`IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` and `IOCTL_STORAGE_QUERY_PROPERTY`.

Handles are opened read-only: `GENERIC_READ`, sharing `FILE_SHARE_READ |
FILE_SHARE_WRITE`, disposition `OPEN_EXISTING`. Write access is never requested,
satisfying ADR-003.

## Alignment
Raw volume and physical-drive handles reject unaligned access: offsets and
lengths must be multiples of the logical sector size, and Windows additionally
constrains buffer addresses. The adapter therefore reads the enclosing aligned
span and trims in memory, so callers continue to pass arbitrary byte ranges as
the BlockDevice contract requires. Alignment is never assumed to be 512 bytes;
4Kn drives report 4096.

## Elevation
Raw device reads require administrator rights. The application requests
elevation only when a physical device is selected; disk-image sources run
unprivileged. Elevation is obtained by launching a separate narrowly scoped
helper process via `ShellExecuteEx` with the `runas` verb, never by marking the
main application as always-elevated.

The helper exposes the same minimal operation set as the macOS helper: enumerate
devices, open a source read-only, read a validated range, close. It performs no
path resolution or policy decisions on behalf of the caller, and it never opens
a handle for writing.

## Volume locking
The MVP does not lock or dismount volumes. `FSCTL_LOCK_VOLUME` and
`FSCTL_DISMOUNT_VOLUME` are write-adjacent operations that affect the source's
observable state and are therefore out of scope. A mounted source may yield
inconsistent reads; this is reported as a diagnostic rather than remediated.

## Disconnection
Removable media may vanish mid-scan. `ERROR_DEVICE_NOT_CONNECTED`,
`ERROR_NO_SUCH_DEVICE` and `ERROR_MEDIA_CHANGED` map to the Disconnected error
in the shared taxonomy. Device identity is revalidated before resume, per the
storage I/O resume invariant.

## Long paths
Output paths use the `\\?\` prefix to avoid MAX_PATH truncation, since recovered
names may be long or deeply nested. Recovered filenames are sanitised against
Windows reserved names (CON, PRN, AUX, NUL, COM1-9, LPT1-9), reserved characters
(`<>:"/\|?*`), and trailing dots or spaces. Sanitisation is recorded as
provenance so the original on-disk name is never silently lost.

## Source and destination overlap
Overlap detection uses volume GUID paths and the file ID from
`GetFileInformationByHandle`, not drive letters, since letters are reassignable
and a path is not stable identity.

## Tests
Aligned and unaligned reads on 512e and 4Kn geometry, reads beyond capacity,
zero-length reads, disconnection mid-read, reserved-name sanitisation, long
paths, and elevation-denied handling.
