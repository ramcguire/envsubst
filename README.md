# envsubst

[![CI](https://github.com/ramcguire/envsubst/actions/workflows/ci.yml/badge.svg)](https://github.com/ramcguire/envsubst/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/ramcguire/envsubst/branch/main/graph/badge.svg)](https://codecov.io/gh/ramcguire/envsubst)
[![Image size](https://img.shields.io/docker/image-size/ramcguire/envsubst/latest?registry_url=https://ghcr.io&label=image%20size)](https://ghcr.io/ramcguire/envsubst)
[![Platforms](https://img.shields.io/badge/platforms-amd64%20%7C%20arm64%20%7C%20arm%2Fv7-informational)](https://ghcr.io/ramcguire/envsubst)

Minimal alternative to the GNU `envsubst` utility. Substitutes environment variables using `${VAR}` / `$VAR` syntax in files matched by glob patterns. Uses the expansion syntax from the [shellexpand](https://crates.io/crates/shellexpand) crate, but does not support tilde (~) expansion.

Packaged as a [distroless](https://github.com/GoogleContainerTools/distroless) container for use in init containers and CI pipelines.

Unknown variables are left as `${VAR}` literals by default.

## Docker usage

```sh
docker run --rm \
  -e DB_HOST=postgres \
  -v $(pwd)/templates:/in:ro \
  -v $(pwd)/rendered:/out \
  ghcr.io/ramcguire/envsubst \
  "/in/**/*.yaml" --output /out
```

## CLI reference

```
envsubst [options] PATTERN [PATTERN...]
```

| Argument | Description |
|---|---|
| `PATTERN...` | One or more glob patterns matching input files |
| `-o, --output DIR` | Write substituted files to `DIR`, mirroring the input directory structure. Omit to write to stdout. |
| `-e, --env-file GLOB` | Load variables from files matching `GLOB` (`.env` syntax). Repeatable. **Mutually exclusive with the real environment** — when any `-e` flag is given, only the loaded files are consulted; system env vars are ignored. |
| `-f, --fail-on-missing` | Exit with code `1` if any variables remain unresolved after substitution. **This includes variables that have no substitution and are using the default syntax.** |
| `-v, --verbose` | Print processed file paths and a summary of unresolved variables to stderr. |

### Variable lookup

| Mode | Source |
|---|---|
| No `-e` flag | Real process environment |
| One or more `-e` flags | `.env` files only — system env is **not** consulted |

The two modes are mutually exclusive by design. The recommended usage mode is to use `-e` where possible as it is more declarative of intent. If you need both `.env` support and real environment variables you can source the `.env` first or simply run the tool twice.

## Examples

```sh
# Substitute from the real environment, write to /out
envsubst "templates/**/*.yaml" --output /out

# Load vars from a .env file; fail if any are unresolved
envsubst "config/*.conf" --env-file .env --fail-on-missing --output /out

# Multiple .env files (later files win on conflict)
envsubst "k8s/**/*.yaml" --env-file base.env --env-file override.env --output /out

# Glob for .env files; preview on stdout with verbose output
envsubst "templates/*.yaml" --env-file "envs/*.env" --verbose
```

## Container images

Published to `ghcr.io/ramcguire/envsubst`. All images are multi-platform: `linux/amd64`, `linux/arm64`, `linux/arm/v7`.

### Variants

Four variants are published, covering two Debian bases × two distroless tags:

| Tag | Base image |
|---|---|
| `latest` *(default)* | `distroless/static-debian12:latest` |
| `nonroot` | `distroless/static-debian12:nonroot` |
| `debian13` | `distroless/static-debian13:latest` |
| `debian13-nonroot` | `distroless/static-debian13:nonroot` |

### Tag scheme

Each variant follows the same scheme. The table below uses the default (`latest`) variant; substitute the variant suffix for others (e.g. `v0.1.0-nonroot`, `sha-abc123-debian13`).

| Tag | Updated | Example |
|---|---|---|
| `latest` / `nonroot` / `debian13` / `debian13-nonroot` | Every merge to `main` | `ghcr.io/ramcguire/envsubst:nonroot` |
| `vX.Y.Z` / `vX.Y` | On semver release tag | `ghcr.io/ramcguire/envsubst:v0.1.0` |
| `sha-XXXXXXX` | Every push | `ghcr.io/ramcguire/envsubst:sha-abc1234-debian13-nonroot` |
