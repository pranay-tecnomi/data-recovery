# Recovery Plugin Architecture Specification

## Plugin types
Filesystem detector/parser, file carver, validator and preview adapter.

## Contract
Plugins declare identifier, version, capabilities, required evidence, resource limits and test corpus.

## Isolation
Plugin failures must not corrupt global session state. MVP plugins run in-process with strict error boundaries; future process isolation is an explicit architectural option.

## Registration
Static registry for MVP; dynamic loading is deferred until security and compatibility requirements justify it.

## Compatibility
Core/plugin API versions are checked at build and runtime where applicable.