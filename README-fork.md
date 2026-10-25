Personal fork of OmaLauncher.
- `juan` branch: upstream + personal patches (no spare pages, uninstall in the home edit bar, crash watcher, own applicationId).
- `.github/workflows/sync-upstream.yml` (on main): daily, merges upstream into `juan`, tests, builds, signs, publishes a release.
