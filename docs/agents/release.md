# Releases (signing, notarization, downloads)

Same pipeline as Alaria (hermes-mac) and Murmur, minus Sparkle auto-update, which needs a public repo.

- **CI** (`ci.yml`, every push to main and every PR): engine tests, app tests, build, unsigned DMG kept for 7 days.
- **Release** (`release.yml`): push a tag `vX.Y.Z` on main. `build` (no secrets) tests and builds unsigned; `sign` (the
  only job with secrets, and it runs no project code) signs, notarizes and staples; `publish` creates the release.
  Refuses to publish unless fully signed, and never replaces a published file.
- **Secrets**, set once from a real Terminal window (it prompts):
  `scripts/ci/set-release-secrets.sh <DeveloperID.p12> <AuthKey_ID.p8>`.
- **Local:** `./scripts/build.sh && DMG_NO_FINDER=1 ./scripts/package.sh`. Signed with the Developer ID in the login
  keychain, so permissions granted to an installed copy survive updates.
