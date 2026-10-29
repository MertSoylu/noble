# winget package

Package identifier: `MertSoylu.Noble` (publisher Mert Soylu, package name NOBLE).

Installs are portable zips. Each installer is the release archive `noble-windows-x86_64.zip` (x64) or
`noble-windows-aarch64.zip` (arm64), with `InstallerType: zip`, `NestedInstallerType: portable`,
`NestedInstallerFiles: noble.exe` and `PortableCommandAlias: noble`, so `noble` runs from any shell after the install.
The manifests here are a template for v1.12.1 (manifest schema 1.9.0). `<SHA256>` is a placeholder for the archive's
SHA-256, which the release also lists in its `.sha256` file.

## First submission (by hand, once)

winget-releaser only updates a package that already exists in microsoft/winget-pkgs, so the first version is
submitted by hand after its GitHub release is published. Either:

- run `wingetcreate new` with the two Windows archive URLs and follow the prompts (the identifier is `MertSoylu.Noble`);
  it opens the pull request for you; or
- open a pull request to microsoft/winget-pkgs with the files from `manifests/`, after replacing each `<SHA256>` with
  the real hash (`Get-FileHash -Algorithm SHA256 noble-windows-x86_64.zip` in PowerShell).

Keep the `WINGET_TOKEN` secret unset when the first version's tag is pushed. Otherwise the `winget` job runs before
the package exists, fails (the GitHub release is still published) and the tag run ends red. Set the secret once the
first submission is merged; later tags then update the package on their own.

## Later releases

Automatic. The `winget` job in `.github/workflows/release.yml` runs after the release is published on a `v*` tag and
submits the new version with `vedantmgoyal9/winget-releaser`. It needs the `WINGET_TOKEN` repository secret: a classic
GitHub personal access token with the `public_repo` scope. Without the secret the job skips (it does not fail).
