# ADR 0010: Shared Headless Read Surface

## Status

Accepted.

PulseOn 0.1.0a5 establishes a shared headless read surface for trainers and
agents: the typed Python SDK supports discovery and Arrow-compatible bulk
consumption, while a read-only CLI provides human-readable tables and versioned
JSON. V5 does not add a Web viewer, built-in plotting, MCP, agent memory,
arbitrary SQL, or file export because one domain query surface keeps the final
alpha independently reviewable and preserves the native storage and Parquet
boundaries.
