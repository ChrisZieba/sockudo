# Manual Build Guide

## How to Trigger Manual Builds

### For Pull Requests
1. **Automatic Instructions**: When you create a PR, a bot comment will appear with a direct link
2. **Click the link** in the bot comment to go to the Manual Build workflow
3. **Select Linux architectures** you want to build
4. **Artifacts** will be posted back to the PR when complete

### For Any Branch
1. Go to [Actions → Manual Build](../actions/workflows/manual-build.yml)
2. Click **"Run workflow"** (green button)
3. Select your branch from the dropdown
4. Choose artifacts to build:
   - ☑️ **Linux x64** (GNU + musl)
   - ☑️ **Linux ARM64** (GNU + musl)
   - ☑️ **Docker image** - Containerized application
5. Click **"Run workflow"**

## What Happens Next

1. **Build Status**: Check the Actions tab for build progress
2. **Artifacts**: Download from the workflow run page (available for 30 days)
3. **PR Comments**: If triggered from a PR, results are posted as comments

## Artifact Downloads

### Binaries
- **Format**: `sockudo-{target}` (e.g., `sockudo-x86_64-unknown-linux-gnu`)
- **Usage**: Verify the adjacent `.sha256` file, extract, and run
- **Platforms**: Linux x86_64 and ARM64, with GNU and musl variants

### Docker Images
- **Format**: `sockudo-docker-{branch}-{timestamp}.tar.gz`
- **Usage**: 
  ```bash
  # Extract and load
  gunzip sockudo-docker-*.tar.gz
  docker load < sockudo-docker-*.tar
  
  # Run
  docker run -p 6001:6001 sockudo:your-tag
  ```

## Build Times

| Platform | Typical Time | Cache Hit |
|----------|--------------|-----------|
| Linux x64 | ~8 minutes | ~3 minutes |
| Linux ARM64 | ~8 minutes | ~3 minutes |
| Docker | ~10 minutes | ~4 minutes |

## FAQ

**Q: Why are manual builds separate from CI?**
A: To save CI minutes and allow flexible platform selection only when needed.

**Q: Can I build multiple platforms at once?**
A: Yes! Check multiple platforms in the workflow dispatch form.

**Q: How long are artifacts stored?**
A: 30 days (longer than the default 7 days).

**Q: Can I build from any branch?**
A: Yes, select any branch from the dropdown when triggering manually.
