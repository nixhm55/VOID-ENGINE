<h1 align="center">VOID Engine</h1>

<p align="center"><strong>Turn a website into a lightweight desktop or Android app.</strong></p>

<p align="center">Configure a build in GitHub and let hosted runners package the app. You do not need Rust or a native build toolchain on your computer.</p>

<br />

<h2 align="center">What VOID builds</h2>

<p align="center"><code>.dmg</code> for macOS · <code>.exe</code> and <code>.msi</code> for Windows · <code>.deb</code> and <code>.AppImage</code> for Linux · <code>.apk</code> for Android</p>

The platform builds run in parallel. Total time depends on GitHub runner availability and the slowest platform job.

<br />

<h2 align="center">Build with GitHub Actions</h2>

This is the quickest way to build from your own fork. It does not require installing the VOID command-line client or configuring its token.

1. Fork this repository.
2. In your fork, open **Settings → Actions → General** and allow Actions to run.
3. Open **Actions → Build VOID → Run workflow**.
4. Enter the website URL and app name. Add an icon URL if you have one, and choose which platforms and package formats to build.
5. When the run finishes, download the generated packages from its **Artifacts** section.

<br />

<h2 align="center">Terminal</h2>

Publishing the client is automated, but it is not automatically installed on your computer. The **Release VOID Cloud Client** workflow attaches a small client for each supported desktop platform when a GitHub release is published. Install the matching client once; it sends build requests to the cloud and does not compile the app on your computer. To skip client installation, use the GitHub Actions steps above.

<h3 align="center">Install on macOS</h3>

The published macOS client currently targets Apple Silicon.

```sh
mkdir -p "$HOME/.local/bin"
curl --fail --location "https://github.com/nixhm55/VOID-ENGINE/releases/latest/download/void-macos" -o "$HOME/.local/bin/void"
chmod +x "$HOME/.local/bin/void"
export PATH="$HOME/.local/bin:$PATH"
```

To keep the PATH change in new Terminal windows, add `export PATH="$HOME/.local/bin:$PATH"` to `~/.zprofile`.

<br />

<h3 align="center">Install on Linux</h3>

```sh
mkdir -p "$HOME/.local/bin"
curl --fail --location "https://github.com/nixhm55/VOID-ENGINE/releases/latest/download/void-linux" -o "$HOME/.local/bin/void"
chmod +x "$HOME/.local/bin/void"
export PATH="$HOME/.local/bin:$PATH"
```

To keep the PATH change in new terminals, add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile, such as `~/.profile`.

<br />

<h3 align="center">Install on Windows (PowerShell)</h3>

```powershell
$bin = Join-Path $HOME ".local\bin"
New-Item -ItemType Directory -Force $bin | Out-Null
Invoke-WebRequest "https://github.com/nixhm55/VOID-ENGINE/releases/latest/download/void-windows.exe" -OutFile (Join-Path $bin "void.exe")
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ';') -notcontains $bin) {
  [Environment]::SetEnvironmentVariable("Path", (($userPath, $bin | Where-Object { $_ }) -join ';'), "User")
}
$env:Path = "$bin;$env:Path"
```

Open a new PowerShell window after installation if you want the updated PATH to load automatically.

<br />

<h3 align="center">Sign in to GitHub</h3>

Run `void login`. The client displays a one-time code and GitHub verification URL, then saves the access token in the operating system's credential store after you approve it. The token is never printed or added to your shell profile. Run `void login` again to replace the saved sign-in.

The OAuth App used by the client must have Device Flow enabled. The current default build repository is public; OAuth Apps cannot request the fine-grained **Actions: read** permission for private repositories. Use a GitHub App with Actions read permission if private repository support is needed.

<br />

<h2 align="center">Request a build</h2>

```sh
void https://example.com --name "My App"
void https://example.com --name "My App" --icon-url https://example.com/icon.png
```

The client sends the URL, app name, and optional icon URL through the VOID cloud proxy. It polls the GitHub Actions run every five seconds, then downloads and extracts that run's artifacts into the current folder. ZIP files are saved there too.

By default, the client polls `nixhm55/VOID-ENGINE`. Set `VOID_GITHUB_REPOSITORY=owner/repository` to poll a different build repository. See `void --help` for all options.

<br />

<h2 align="center">How the NSMS cloud build works</h2>

NSMS means **Nisam's Shift Method**. The master-binaries workflow builds reusable platform engines. For each request, the Vercel proxy starts a GitHub Actions run. That workflow applies the app's URL, name, and icon, then packages the requested formats on hosted runners. Platform jobs run concurrently; the app is not compiled on the user's computer.

<br />

<h2 align="center">Develop locally</h2>

The native wrapper can still be built from this repository with Cargo for development. Its runtime features and Android library are part of the main `void` package; the separate `void-cli` package contains the cloud client.
