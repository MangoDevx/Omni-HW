# Releasing Omni-HW

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and run `cargo check` so `Cargo.lock` updates.
2. In `CHANGELOG.md`, rename `## Unreleased` to `## <version>`. Use New / Improved / Fixed bullets, user-visible changes only, one line each.
3. Commit, then tag and push:
   ```
   git tag v<version>
   git push origin main v<version>
   ```
4. Open the Actions tab, wait for the `build` job, and approve the `release` environment run. The release (exe, SHA-256 file and notes) is then created and the website is redeployed.

The release doesn't re-run fmt, clippy or tests; it waits for CI on the tagged commit, so push `main` together with (or before) the tag.

The build fails early if the tag doesn't match the Cargo version, or if `CHANGELOG.md` has no bullets under `## <version>`.

## One-time setup

- Repository environment named `release`, with a required reviewer.
- Secret `PAGES_DEPLOY_HOOK` (the Cloudflare Pages deploy hook URL). If missing, the release still publishes and a warning is shown.
- Later, once SignPath approves the project: secret `SIGNPATH_API_TOKEN`, then enable the `Sign (SignPath)` step in `release.yml` and remove the SmartScreen note from the release body.
