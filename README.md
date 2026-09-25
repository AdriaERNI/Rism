# Rism

Rism is **Prism with Rust**.

Prism is an MCP server and CLI for InterSystems IRIS development (SQL, documents,
compilation, debugging, testing, and ObjectScript execution via the Atelier REST
API). Rism is its Rust rewrite.

## Development environment

A local IRIS Community instance runs via Docker Compose:

```bash
docker compose up -d
```

- Image: `intersystemsdc/iris-community:2025.3` (same tag Prism CI uses)
- Atelier REST API: <http://localhost:52773/api/atelier/>
- Credentials: `_SYSTEM` / `SYS`
- Container name: `rism-iris` (default IRIS port; stop any other local IRIS
  instance first — e.g. Prism's — to free `52773`).

Shut down with `docker compose down` (add `-v` to wipe the data volume).
