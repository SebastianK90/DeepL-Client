# GLib 0.18.5 security backport

This is the published `glib` 0.18.5 crate with the official fix for
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)
backported from [gtk-rs/gtk-rs-core#1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343),
merge commit `05dff0ee696f9bcd8617cd48c4b812d046d440cb`.

The only changes to upstream library code are in `src/variant_iter.rs`:

- Declare the C string out-pointer as `let mut p`.
- Pass it to `g_variant_get_child` as `&mut p`, rather than writing through `&p`.

This preserves the 0.18.5 API, ABI and features required by GTK3/Tauri. The root
Cargo patch selects this source for transitive dependencies too. Original
license, copyright and crate files are retained. `BACKPORT.json` records the
verified registry archive checksum and original/patched source-file checksums.

The application's `glib_string_variant_iterator_handles_both_directions_and_unicode`
regression must also run with `cargo test --release --locked`, because compiler
optimization exposes the original undefined behavior. It covers forward,
reverse, mixed, skipping, last-element, Unicode and empty-array iteration.

Version-only advisory scanners may still report 0.18.5. Verify the selected
local source and this backport; do not change version numbers or silently
suppress unrelated advisories to make a scan look clean.
