# Multi-Project Analysis Workbench

> Status: product and architecture draft, not an accepted decision or roadmap commitment.

## Outcome

Evolve the single-source, single-metric viewer into a local analysis workbench that:

- retains multiple imported local data sources;
- selects Runs across Projects and data sources;
- supports multiple analysis Views in one window; and
- renders one chart panel for every metric selected in the active View.

Preserve the native storage boundary, evidence semantics, public Python API,
catalog and Parquet schemas, and renderer-independent chart model.

## Proposed Product Language

Existing glossary terms remain authoritative: a **Project** contains related
Runs, a **Run** is one training execution, and a **metric key** names a metric
series. Product code must not introduce `Experiment` as a synonym.

This draft proposes three additional terms:

- **Data source**: one imported local native PulseOn store, possibly containing
  multiple Projects.
- **Analysis View**: a named workspace tab containing a Run selection, metric
  panels, alignment settings, and presentation state.
- **Metric panel**: one chart card for one metric key within an Analysis View.

If accepted, add these definitions without implementation details to `CONTEXT.md`.

## Experience

```text
+ Projects / Runs ------+ [Overview] [Training] [Ablation] [+] --------+
| + Import Source       | Runs: 6 | Metrics: 4 | Step | Refresh        |
|                       +----------------------+------------------------+
| v Project A           | train/loss           | train/lr               |
|   [x] baseline        | chart + brush         | chart + brush          |
|   [x] candidate       +----------------------+------------------------+
| v Project B           | train/accuracy       | eval/loss              |
|   [x] control         | chart + brush         | chart + brush          |
|   [ ] candidate       |                       |                        |
+-----------------------+-----------------------+------------------------+
```

### Sidebar

- Show a searchable, collapsible Project tree similar to the reference Codex
  sidebar. Flatten healthy sources while retaining source identity for errors
  and disambiguation.
- Show Runs under each Project with selection, lifecycle status, and a compact
  secondary identifier. Checkboxes reflect the active View's selection.
- Continue to allow at most 10 selected Runs per View.
- Provide Import Source, reveal path, refresh, and remove-from-workbench
  actions. Removing a source must not delete native data.
- Keep unavailable sources visible with a reconnect/error treatment rather
  than silently removing their Projects and selections.

### View Tabs and Toolbar

- Use a compact tab strip with create, activate, rename, duplicate, close, and
  overflow actions.
- Each View owns its name, ordered Runs and metrics, alignment axis, panel
  order, grid density, and chart viewport state.
- A new View starts empty or duplicates the active View explicitly; Views must
  never share mutable selection state implicitly.
- The active toolbar exposes metric selection, Step/Elapsed alignment,
  Refresh, Reset View, grid density, and pending/error status.
- Closing the last View creates a fresh empty View.

### Metric Grid

- Render one independently identifiable panel per selected metric in a
  responsive, scrollable grid. Narrow windows fall back to one column without
  shrinking charts below a usable interaction size.
- Each panel keeps its title, evidence legend, detail chart, overview brush,
  hover state, pending indicator, and panel-level error.
- Reordering or removing one panel must not invalidate unaffected panels.
- The metric picker shows the selected Runs' metric union. A Run without a
  selected metric remains in that panel's legend as unavailable evidence.
- Only panels in the active View and near the visible scroll region prepare
  GPUI geometry or request new detail data.

## Stable Selection Identity

`RunId` alone is insufficient once multiple data sources and Projects are open:

```text
RunRef = DataSourceId + ProjectId + RunId
```

`DataSourceId` is viewer-local, not a native-storage identifier. Series and
requests use the full `RunRef`, so duplicate identifiers cannot collide in
selections, caches, colors, hover results, or stale-result handling. Storage and
model crates retain their existing Project and Run types.

## State Boundaries

```text
Workbench
  imported data sources and source sessions
  ordered Analysis Views and active View identity

Analysis View
  ordered RunRefs and MetricPanels
  alignment and presentation settings

Metric panel
  overview/detail snapshots, brush, and hover state
  generations, pending state, and renderer cache revision
```

Render callbacks consume immutable or plain local snapshots and must not
recursively access the same GPUI entity. Background source events return to the
foreground before changing workbench or panel state.

## Query Coordination

- Keep one native read session, exclusively owned by a background worker, for
  each imported data source.
- Fan a panel request out by source, query that source's Runs, then merge
  immutable evidence snapshots at the viewer boundary.
- Preserve results and errors per source; one failure must not erase drawable
  evidence returned by another source.
- Tag requests with source, View, panel, request kind, and generation. Apply a
  result only while all identities still match current state.
- Coalesce pending work per source and panel. Newer generations replace older
  overview or detail work that has not started.
- Wheel/pinch updates the viewport immediately and submits detail work only
  after 100 ms without another event.
- Running native queries may finish, but stale results cannot replace current
  View or viewport state.
- Inactive Views issue no viewport queries. Off-screen panels keep their latest
  snapshot but suspend geometry preparation and refresh work.

Cross-source aggregation belongs to viewer coordination. Native connections
must not cross worker threads or weaken the storage ownership boundary.

## Persistence

Import implies persistence across restarts. Viewer-owned application data may
store source paths, View definitions, composite selections, presentation state,
and safe preferences. It must not store metric points, query snapshots,
credentials, connections, or renderer geometry.

Loading must tolerate unavailable sources, removed Projects or Runs, unknown
metrics, and unsupported state versions without mutating source data. The exact
format, migration policy, and location remain open.

## Performance Contract

- Preserve existing per-series overview and detail point budgets.
- Keep native queries and cross-source merging off the UI thread.
- Prepare visible-panel paths only; cache by panel revision, viewport, canvas,
  and theme.
- Do not query per wheel event or refresh inactive Views.
- Bound worker concurrency so sources and panels cannot create unbounded query
  fan-out.
- Measure frames separately from storage latency, then validate a multi-panel View.

## Delivery Slices

1. Add viewer-local data-source and composite selection identities while
   retaining the current single-panel UI.
2. Add retained data sources and the collapsible Project/Run sidebar.
3. Add Analysis View tabs with independent in-memory Run selections.
4. Split chart state into Metric panels and render the responsive metric grid.
5. Add visible-panel scheduling, cross-source merging, and performance gates.
6. Add versioned persistence and unavailable-source recovery.

Each slice uses persistent product names. Roadmap phase identifiers must not
enter source paths, modules, functions, tests, environment variables, or comments.

## Acceptance Scenarios

- Import two stores containing duplicate Project or Run identifiers;
  selections, colors, queries, and hover results remain distinct.
- Select Runs from two Projects and four metrics; receive one panel per metric
  with explicit unavailable evidence where appropriate.
- Switch between Views with different selections; each restores its own brush,
  snapshots, and pending generations.
- Rapidly zoom a visible panel; feedback stays immediate while queries remain
  trailing-debounced and coalesced.
- Scroll a large grid; off-screen panels stop preparing geometry and do not
  initiate viewport refreshes.
- Remove or move one source; other sources remain usable and saved selections
  reconcile without modifying native data.
- Restart; imported sources and View definitions return while query snapshots
  rebuild from native storage.

## Open Questions

- Are x viewports independent, linked per View by default, or explicitly linked?
- Should name collisions reveal source grouping in the Project tree?
- How many panels should be prepared around the visible scroll region?
- Does a new View start empty or copy the active Run selection?
- Is panel resizing initially required, or are grid-density choices sufficient?
- What persistence format and application-support path form the compatibility boundary?
