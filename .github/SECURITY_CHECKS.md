# Dependency and workflow checks

`security.yml` runs on pushes, pull requests, manual dispatch, and daily at
06:17 UTC. `node scripts/security-dependencies.cjs` queries current OSV data
for every crate version in Cargo.lock, including local vendored crates and
all target platforms. The scanner has no third-party npm dependencies.
Network errors, incomplete results, new advisories, and expired exceptions
fail the job. This checks published advisories; it is not a source-code audit.

Before scanning, `scripts/check-glib-backport.cjs` verifies the reviewed GLib
0.18.5 fix against its pinned source digest and Cargo's actual resolved local
source. Only RUSTSEC-2024-0429 and its GHSA alias are accepted for that exact
verified backport. Changing the local source or using another GLib version
requires review. The Linux build also runs the optimized iterator regression.
See `vendor/glib/BACKPORT.json` for the registry archive digest, upstream fix,
and patched file digest. No global GLib advisory ignore is configured.

The five entries in `security-maintenance-exceptions.json` acknowledge existing
unmaintained dependencies, reviewed on 2026-11-02. Each entry binds one advisory,
crate, and exact version and expires on 2027-02-01. These entries cannot suppress
a vulnerability or unsoundness advisory. Update or migrate the dependency and
remove its entry, or review and document an extension before expiry. The reason
for each pending migration is recorded with the entry. GTK ecosystem changes
require compatibility testing; PDF font parsing changes require layout tests.

`node scripts/security-supply-chain.cjs` rejects floating Action references,
floating Node/Rust versions, and unpinned or unlocked `cargo install` commands
in workflows. All current Actions use full commits resolved from their official
repositories. Node is 22.23.4, Rust is 1.99.1, and the Tauri packager is 2.12.4.
Dependabot checks Cargo and GitHub Actions weekly, including commit pins.
These exact tool versions still need deliberate review and updates; this guard
does not pin hosted runner images or the Ubuntu package repositories.

Run policy regressions with:

```sh
node --test scripts/security-dependencies.test.cjs scripts/security-supply-chain.test.cjs
```

On GitHub, Dependabot alerts, Dependabot security updates, Secret Scanning, and
Secret Scanning push protection were enabled and read back on 2026-11-02.
Repository settings remain separate from the checked-in workflow files.
