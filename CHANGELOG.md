# changelog

## Unreleased
  - Added interactive TUI with realtime project refresh, log panel, and command execution
  - Using local database to analytics
  - Added audio metadata extraction (symphonia) and image dimension detection (imagesize)
  - Added `--json` flag for machine-parseable output across all commands
  - Added credential management module for Supabase authentication
  - Fixed thread blocking in TUI by switching to polling with `tokio::spawn`

## 0.1.0-alpha.1
- scaffold changed to make it more modular
- expanded test coverage
- deployment script updated to perform based on artist and a subscription

## 0.1.0-alpha
- Installer added in release
- cite-cli created with commands (init, validate, lint, build, deploy, status, doctor, clean, upgrade, uninstall)
- Add module test and integration test
