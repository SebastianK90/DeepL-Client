# Security reporting

Do not place credentials, private documents or unredacted logs in public issues.

After this repository is hosted on GitHub, use its private vulnerability reporting feature if the owner has enabled it. Otherwise contact the repository owner privately through a channel they publish. This source preparation does not invent a security email address or enable a hosted reporting feature.

Include the affected version/commit, operating system, a minimal synthetic reproduction and impact. Ordinary bugs without sensitive details can use the bug report template. There is no promised security-response SLA; this project currently has one development version and no separately maintained release branches.

## Window command boundaries

All custom commands are explicitly registered with Tauri's application ACL.
Capabilities use exact local window labels: main, Quick Assist, its language
popup, and screenshot selection. A separate runtime dispatcher rejects unknown
commands and callers outside the same command policy before invoking handlers.
Quick Assist bootstrap excludes history and avoids querying API-key presence;
its settings writes only update translation languages and writing style.
Run `node scripts/security-acl-check.cjs` and the `command_policy` Rust tests
to check command coverage and Tauri's resolved permissions on both platforms.

## Office package policy

DOCX/PPTX imports receive a bounded package preflight before any translation
request. It checks archive names, duplicate aliases, macro-free main content
types and all relationship files. VBA, ActiveX, OLE/package embeddings, add-in
parts, external templates/imports and non-HTTP(S)/mailto external hyperlinks
are rejected. Word fields that can import external resources or invoke DDE
are rejected, including split instructions in relocated XML parts. Ordinary
internal bookmark links and page-number fields remain supported.

This policy rejects unsupported documents instead of silently removing parts.
It is not a comprehensive antivirus, PDF sandbox or guarantee about the Office
application later opening a file. Windows export-origin metadata and PDF process limits are described below.
These measures do not certify a document as safe.

## History bounds and recovery

History is limited to 500 entries, 64 KiB of text/identifiers per entry, 4 MiB
of cumulative text and 8 MiB of serialized JSON. Invalid, oversized or damaged
history is renamed to an adjacent `history.recovery-<UUID>.json` before a new
history can be written. These recovery copies may contain sensitive text and
are never automatically deleted. If reading or preservation fails, startup
continues with history writes disabled for that session. Successful translations
remain usable even if their result exceeds history limits.

## Dependency and build automation

See [.github/SECURITY_CHECKS.md](.github/SECURITY_CHECKS.md) for daily advisory
scans, immutable Action references, exact build-tool versions and the narrow
expiring maintenance exceptions. The verified GLib backport remains mandatory.
Dependabot alerts/security updates and Secret Scanning with push protection
were enabled on GitHub. Workflow changes take effect once pushed.

## Windows document provenance

The import API provides a filename and bytes, not reliable source-origin
metadata. All translated exports are therefore conservatively marked with
`Zone.Identifier / ZoneId=3` before atomic publication on Windows. The marker
is written and read back on the same-volume temporary file; unsupported
filesystems or marking failures refuse the export and preserve the previous
destination. This may enable Protected View or other Office policy depending
on the installed application and policy; it does not invoke an antivirus scan
or claim to preserve a source URL that the application never received.
Native Windows/NTFS checks run through `--verify-export-origin` and unit tests.
Wine is not accepted as proof of this functionality.

## PDF subprocess containment

Imported PDFs are parsed and rewritten by the same executable in a dedicated
child mode, before GUI initialization. Only bounded frames pass over private
stdin/stdout pipes: PDF input up to 32 MiB, output up to 128 MiB, individual
text messages up to 20,000 bytes/5,000 characters. The parent retains providers,
API-key lookup and the 300,000-character/10,000-request budgets. Worker
environment inheritance is explicitly allowlisted and its engine cache is
private to the job. No untrusted PDF is parsed by the main document UI path.

Linux applies a 2 GiB address-space limit, 60-second soft CPU limit (61-second
hard limit), no core dumps, bounded file size, no_new_privs, parent-death
termination and noninheritance of unrelated descriptors. Windows applies
verified Job Object limits: 2 GiB committed memory, 60 seconds of user CPU,
one process, and termination when the parent closes the job handle. Limits
must be established before any PDF bytes are sent.

The parent enforces a 30-second pipe/worker wait limit excluding active
provider calls, a 30-minute document deadline including provider waits, and
cancellation checks at 50 ms intervals. Workers are killed and reaped on all
error paths; Linux process groups are also terminated after successful output
before the direct child's PID can be recycled.

This contains parser crashes and resource exhaustion. It is not a complete
filesystem/network OS sandbox: the worker still has the current user's OS
permissions. A native parser compromise is not claimed to be harmless.
Linux PDF subprocesses require kernel 5.11+; unsupported process limits fail
closed. Wine's incomplete Job Object limits are also refused.
