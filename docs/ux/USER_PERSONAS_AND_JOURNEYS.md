# User Personas and Complete User Journeys

## Personas
### Consumer
Needs safe defaults and clear guidance.

### Advanced User
Understands partitions, images, and raw devices; needs configuration and technical details.

### IT Professional
Needs repeatable workflows, detailed logs, and reports.

## Global journey
START → Device Discovery → Select Source → Device Assessment → Safety Decision → Scan or Image First → Results → Select Files → Validate Destination → Recover → Validate → END

## Key scenarios
- Accidentally deleted file.
- Formatted drive.
- Corrupted filesystem.
- Failing drive.
- Active startup disk.
- Disk image recovery.

## Safety UX rules
- Never default to writing on the source.
- Block recovery to the source.
- Warn on instability.
- Clearly distinguish scan, image, recover, and repair.
- Never guarantee recovery.
- Do not use misleading confidence precision.

## Accessibility
Keyboard navigation, screen-reader compatibility, clear progress indicators, and non-color-only status indicators.