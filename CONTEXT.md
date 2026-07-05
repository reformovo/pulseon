# PulseOn Context

PulseOn tracks local-first training runs and numeric metric series. This
context defines the product language used across native architecture decisions.

## Language

**Project**:
A lightweight namespace for related training runs.

**Run**:
One training execution with a user-supplied or generated run identifier.
_Avoid_: Experiment

**Metric key**:
The user-facing name for a metric series.

**Metric series**:
All metric points for one run and metric key.

**Metric point**:
One numeric observation in a metric series.

**Metric reporting**:
The hot-path handoff from training code to PulseOn.

**Queued report**:
A metric report received by PulseOn but not yet durably admitted.

**Accepted report**:
A metric report that PulseOn has durably admitted and can recover after process
restart.

**Persisted metric point**:
A metric point that has been written to native storage and is visible to
PulseOn queries.

**Closed run**:
A run that no longer accepts metric reports because it is being finalized or has
already reached a terminal state.

**Terminal run**:
A run whose lifecycle has ended as either finished or failed.

**Run finalization**:
The explicit transition of a run from running to a terminal lifecycle state.

**Metric aggregate**:
Derived index state over an effective metric series.

**Catalog backend**:
The database engine used for native storage metadata.

**Data path**:
The local filesystem location used for Parquet metric data.

**Metric key encoded**:
A storage-facing percent-encoded form of a metric key used for data-path
partitioning.
_Avoid_: Metric key partition
