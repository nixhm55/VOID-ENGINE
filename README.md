<h1 align="center">VOID Engine</h1>

<p align="center"><strong>Turn a website into a lightweight desktop or Android app.</strong></p>

<p align="center">Configure a build in GitHub and let hosted runners package the app. You do not need Rust or a native build toolchain on your computer.</p>

<br />

<h2 align="center">What VOID builds</h2>

<p align="center"><code>.dmg</code> for macOS · <code>.exe</code> and <code>.msi</code> for Windows · <code>.deb</code> and <code>.AppImage</code> for Linux · <code>.apk</code> for Android</p>

<p align="center">The platform builds run in parallel. Total time depends on GitHub runner availability and the slowest platform job.</p>

<br />

<h2 align="center">Build with GitHub Actions</h2>

<p align="center">This is the quickest way to build from your own fork. It does not require installing the VOID command-line client or configuring its token.</p>

<p align="center">1. Fork this repository.</p>

<p align="center">2. In your fork, open <strong>Settings → Actions → General</strong> and allow Actions to run.</p>

<p align="center">3. Open <strong>Actions → Build VOID → Run workflow</strong>.</p>

<p align="center">4. Enter the website URL and app name. Add an icon URL if you have one, and choose which platforms and package formats to build.</p>

<p align="center">5. When the run finishes, download the generated packages from its <strong>Artifacts</strong> section.</p>

<br />

<h2 align="center">Lite vs Max</h2>

<p align="center">Choose one profile in the <strong>Build VOID</strong> workflow. <strong>Void Engine Lite — Lightweight</strong> keeps the small wrapper and omits the native device bridges. <strong>Void Engine Max — Full features</strong> includes the same base wrapper plus folder access, geolocation, and a native Bluetooth Low Energy bridge.</p>

<div align="center">

<table>
  <thead>
    <tr><th>VOID Engine Lite</th><th>VOID Engine Max</th></tr>
  </thead>
  <tbody>
    <tr><td>Base Wrapper</td><td>Base Wrapper</td></tr>
    <tr><td>Icon Conversion</td><td>Icon Conversion</td></tr>
    <tr><td>Notifications</td><td>Notifications</td></tr>
    <tr><td>Frameless Drag</td><td>Frameless Drag</td></tr>
    <tr><td></td><td>Folder Access</td></tr>
    <tr><td></td><td>Geolocation</td></tr>
    <tr><td></td><td>Bluetooth BLE</td></tr>
    <tr><td></td><td>Native Menus</td></tr>
    <tr><td></td><td>Web Inspector</td></tr>
  </tbody>
</table>

</div>

<p align="center"><strong>Base Wrapper</strong> is everything both profiles ship: the native window and system webview (WKWebView, WebView2, WebKitGTK, Android WebView), the bundled <code>void://</code> asset protocol, external-link routing, camera and microphone passthrough, macOS notifications, and the frameless header drag region.</p>

<p align="center"><strong>Lite</strong> is therefore just the base wrapper: the smallest binary, icon conversion, notifications, and window drag. It exposes no folder access, geolocation, Bluetooth, native menu bar, or Web Inspector.</p>

<p align="center"><strong>Max</strong> adds the native bridges on top of that same wrapper. Folder access backs <code>window.showDirectoryPicker()</code> with the real native dialog instead of the browser download sandbox. Geolocation routes <code>navigator.geolocation</code> to the platform location service. Bluetooth runs BLE over the native stack. On macOS, Max also installs the App, File, Edit, View, Window, Developer, and Help menus and opens Web Inspector from Developer → Inspect Element.</p>

<p align="center">Separate Lite and Max master-binary workflows publish both profiles when changes are pushed to <code>main</code>; each app build downloads one complete binary for its selected profile. The macOS master release includes Intel, Apple Silicon, and universal binaries; the app workflow uses the universal build by default.</p>

<p align="center">In Max, secure HTTPS sites can use <code>navigator.bluetooth</code> to request a nearby BLE device, connect to its GATT services, read and write characteristics, and receive notifications. VOID asks for origin access with a native three-choice prompt, then shows a native device picker. Bluetooth is limited to BLE; classic Bluetooth and Web Bluetooth APIs outside this supported subset are not exposed. Android uses the platform BLE APIs; macOS, Windows, and Linux use the system BLE stack through the native engine.</p>

<p align="center">Max location uses each platform's location service. On macOS, the generated app declares the macOS Core Location usage description. On Android, granting VOID location permission does not switch on device-wide Location; if it is off, VOID opens Android's Location settings and waits for you to turn it on. Android 11 and earlier also require Location to be on for BLE scanning. The Linux Debian package installs GeoClue; an AppImage needs the host system's GeoClue service available.</p>

<p align="center">Icon discovery loads the target page the way a browser does, with browser <code>User-Agent</code>, <code>Accept</code>, and <code>Referer</code> headers and three retries, then reads the page's <code>&lt;link&gt;</code> icons, any inline manifest JSON, and the site's web manifest before probing <code>/apple-touch-icon</code> and <code>/favicon</code> paths. It ranks Apple touch icons, declared sizes, and decoded resolution, renders SVG icons at 1024px, and converts PNG, ICO, SVG, WebP, JPEG, GIF, and BMP to PNG for packaging. If the site has no usable icon, VOID packages its generated fallback: a glossy black tile with a matte, recessed V.</p>

<h3 align="center">Custom Native Menus (Max Only)</h3>

<p align="center">On macOS Max builds, VOID provides native App, File, Edit, View, Window, Developer, and Help menus. The Edit menu uses native Undo, Redo, Cut, Copy, Paste, and Select All actions; Developer → Inspect Element opens Web Inspector. A wrapped page can add native app-specific menus with <code>window.voidNativeMenu</code>:</p>

```js
window.voidNativeMenu.onAction('recent-chat', () => openRecentChat());
window.voidNativeMenu.setMenus([
  {
    title: 'Chats',
    items: [{ id: 'recent-chat', label: 'Recent chat' }]
  }
]);
```

<p align="center">Menu labels and action IDs are validated and size-limited by the native engine. Actions are dispatched only while the page that registered the menus remains the active secure origin.</p>

<br />

<h2 align="center">Terminal</h2>

<p align="center">Publishing the client is automated, but it is not automatically installed on your computer. The <strong>Release VOID Cloud Client</strong> workflow attaches a small client for each supported desktop platform when a GitHub release is published. Install the matching client once; it sends build requests to the cloud and does not compile the app on your computer. To skip client installation, use the GitHub Actions steps above.</p>

<h3 align="center">Install on macOS</h3>

<p align="center">The published macOS client currently targets Apple Silicon.</p>

```sh
mkdir -p "$HOME/.local/bin"
curl --fail --location "https://github.com/nixhm55/VOID-ENGINE/releases/latest/download/void-macos" -o "$HOME/.local/bin/void"
chmod +x "$HOME/.local/bin/void"
export PATH="$HOME/.local/bin:$PATH"
```

<p align="center">To keep the PATH change in new Terminal windows, add <code>export PATH="$HOME/.local/bin:$PATH"</code> to <code>~/.zprofile</code>.</p>

<br />

<h3 align="center">Install on Linux</h3>

```sh
mkdir -p "$HOME/.local/bin"
curl --fail --location "https://github.com/nixhm55/VOID-ENGINE/releases/latest/download/void-linux" -o "$HOME/.local/bin/void"
chmod +x "$HOME/.local/bin/void"
export PATH="$HOME/.local/bin:$PATH"
```

<p align="center">To keep the PATH change in new terminals, add <code>export PATH="$HOME/.local/bin:$PATH"</code> to your shell profile, such as <code>~/.profile</code>.</p>

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

<p align="center">Open a new PowerShell window after installation if you want the updated PATH to load automatically.</p>

<br />

<h3 align="center">Sign in to GitHub</h3>

<p align="center">Run <code>void login</code>. The client displays a one-time code and GitHub verification URL, then saves the access token in the operating system's credential store after you approve it. The token is never printed or added to your shell profile. Run <code>void login</code> again to replace the saved sign-in.</p>

<p align="center">The OAuth App used by the client must have Device Flow enabled. The current default build repository is public; OAuth Apps cannot request the fine-grained <strong>Actions: read</strong> permission for private repositories. Use a GitHub App with Actions read permission if private repository support is needed.</p>

<p align="center">The CLI package is also configured for GitHub Packages as <code>@nixhm55/void-engine</code>. The <strong>Publish VOID Engine CLI to GitHub Packages</strong> workflow publishes it when manually run or when a <code>void-engine-v&lt;version&gt;</code> release is published. GitHub Packages installs require authentication; see <a href="npm/void-engine/README.md"><code>npm/void-engine/README.md</code></a>.</p>

<br />

<h2 align="center">Request a build</h2>

```sh
void https://example.com --name "My App"
void https://example.com --name "My App" --icon-url https://example.com/icon.png
```

<p align="center">The client sends the URL, app name, and optional icon URL through the VOID cloud proxy. It polls the GitHub Actions run every five seconds, then downloads and extracts that run's artifacts into the current folder. ZIP files are saved there too.</p>

<p align="center">By default, the client polls <code>nixhm55/VOID-ENGINE</code>. Set <code>VOID_GITHUB_REPOSITORY=owner/repository</code> to poll a different build repository. See <code>void --help</code> for all options.</p>

<br />

<h2 align="center">How the NSMS cloud build works</h2>

<p align="center">NSMS means <strong>Nisam's Shift Method</strong>. The master-binaries workflow builds reusable platform engines. For each request, the Vercel proxy starts a GitHub Actions run. That workflow applies the app's URL, name, and icon, then packages the requested formats on hosted runners. Platform jobs run concurrently; the app is not compiled on the user's computer.</p>

<br />

<h2 align="center">Develop locally</h2>

<p align="center">The native wrapper can still be built from this repository with Cargo for development. Its runtime features and Android library are part of the main <code>void</code> package; the separate <code>void-cli</code> package contains the cloud client.</p>
