[中文](/.github/README_zh-CN.md) | English

# ETerm Workspace

Cargo Workspace for the ETerm ecosystem. Coordinates and manages all subprojects.

## Structure

```
ETerm-workspace/
├── ai-cli-session-collector/   # JSONL parser
├── ai-cli-session-db/          # Database + FFI
├── claude/                     # Vlaude services (daemon + server)
├── english/                    # ETerm.app (Swift/macOS)
└── memex/                      # Memex backend
```

## Quick Start

```bash
# Clone with all submodules
git clone --recursive git@github.com:vimo-ai/ETerm-workspace.git
cd ETerm-workspace

# Build all Rust components
./scripts/build.sh

# Or build separately
./scripts/build.sh ffi      # FFI dynamic library
./scripts/build.sh memex    # Memex binary
./scripts/build.sh plugins  # Swift plugins
```

## Submodule Management

```bash
# Pull/update all submodules
git submodule update --init --recursive

# Update submodules to latest
git submodule update --remote
```

## Documentation

- [Architecture](./docs/architecture/overview.md) - Architecture design
- [Development Guide](./docs/guides/development.md) - Development guide

## Repositories

| Subproject | Repository | Description |
|------------|------------|-------------|
| ai-cli-session-collector | [vimo-ai/AI-cli-session-collector](https://github.com/vimo-ai/AI-cli-session-collector) | JSONL parsing |
| ai-cli-session-db | [vimo-ai/ai-cli-session-db](https://github.com/vimo-ai/ai-cli-session-db) | Database + FFI |
| claude | [vimo-ai/vlaude](https://github.com/vimo-ai/vlaude) | Vlaude services |
| english | [vimo-ai/ETerm](https://github.com/vimo-ai/ETerm) | macOS app |
| memex | [vimo-ai/memex](https://github.com/vimo-ai/memex) | Memex backend |
