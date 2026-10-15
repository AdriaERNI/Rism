# G fixtures (provenance: research measurement set)

Ten planted call edges + one decoy used to measure the ##class-literal
workaround recall and to specify the `find_references` recall gate
(see ../G_RESEARCH.md "Fixtures + baseline recall" and ../G_SPEC.md §5).

Load order matters (macros before routines that `Include` them):

1. `FProbeG.Macros.inc` (doc-API stores it as `.mac` on 2025.3 — that IS the
   quirk under test; load as `.inc` name, verify with `StudioOpenDialog`)
2. `FProbeG.Mid.mac`, `FProbeG.EdgeR.mac`
3. `FProbeG.Target.cls`, `FProbeG.Caller.cls`, `FProbeG.Sub.cls`
   (compile classes with flags `cku`; Target MUST compile before Sub)

Teardown: delete all six docs, then verify
`SELECT Name FROM %Dictionary.ClassDefinition WHERE Name LIKE 'FProbeG%'`
and `StudioOpenDialog('FProbeG*',1,1,1,1,1)` return zero rows.

`recall.py` measures the shipped-search baseline; `proto_refs.py` is the
narrow+parse-on-fetch prototype whose pattern set the spec freezes (std-only
equivalent must be re-implemented in Rust — these scripts are references,
not shipped code).
