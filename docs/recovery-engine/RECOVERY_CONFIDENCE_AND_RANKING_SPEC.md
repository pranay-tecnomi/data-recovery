# Recovery Confidence and Ranking Specification

## Principle
Confidence represents evidence quality, not a probability of user satisfaction.

## Evidence dimensions
Metadata integrity, allocation evidence, extent continuity, structural validation, content parser validation, read-error exposure and reconstruction completeness.

## Classes
HIGH: strong metadata and successful structural/content validation.
MEDIUM: substantial evidence with one meaningful uncertainty.
LOW: weak extent/boundary evidence or incomplete validation.
UNKNOWN: evidence insufficient for classification.

## Ranking
Sort within a class using explicit evidence counts; UI must not display invented percentages.

## Overrides
Unrecoverable read gaps or parser failure may lower confidence regardless of metadata.

## Auditability
Store reasons for each classification so the UI can explain it.

## Tests
Golden candidates covering each class and regression tests for every scoring change.