# Contributing

Use the setup and checks in [docs/BUILD.md](docs/BUILD.md). Keep changes focused, preserve Windows and Linux behavior, and write code comments in English where practical.

Use synthetic documents and text for tests. Never commit API keys, environment files, user history, private documents, screenshot captures or proprietary application excerpts. Preserve the Windows MSVC static CRT configuration and run the PE verifier for Windows release changes. Test desktop integrations on the actual target desktop; Wine and mocked browser checks do not replace native UI testing.

For pull requests, describe the user-visible problem, resulting behavior, checks performed and remaining platform limitations. Keep dependency changes explicit and retain third-party notices. Contributions to original project code are submitted under the repository's MIT license; vendored code retains its own license.
