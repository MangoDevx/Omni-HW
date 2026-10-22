# Releasing Omni-HW for Linux

Linux releases come from the `linux-support` branch and are tagged `linux-v<version>`, so they
never collide with the Windows releases (`v<version>` on `main`). They're never marked "latest":
the Windows update check and the website read the latest release.

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and run `cargo check` so `Cargo.lock` updates.
2. In `CHANGELOG.md`, rename `## Unreleased` to `## <version>`. Use New / Improved / Fixed bullets, user-visible changes only, one line each.
3. Commit, then tag and push:
   ```
   git tag linux-v<version>
   git push origin linux-support linux-v<version>
   ```
4. Open the Actions tab, wait for the `build` job, and approve the `release` environment run. The release (`omnihw-<version>-linux-x86_64.tar.gz`, its SHA-256 file and notes) is then created.

The release doesn't re-run fmt, clippy or tests; it waits for CI on the tagged commit, so push `linux-support` together with (or before) the tag.

The build fails early if the tag doesn't match the Cargo version, or if `CHANGELOG.md` has no bullets under `## <version>`.

The binary is built on `ubuntu-22.04`, so it runs on distributions with glibc 2.35 or newer.
